//! Continuous screen-sample delivery, independent of checkpoint finalization.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use screencapturekit::cm::CMSampleBuffer;
use screencapturekit::dispatch_queue::{DispatchQoS, DispatchQueue};

use crate::macos_segment_frames::FrameWindow;

const ERROR_CAPACITY: usize = 1024;

pub(crate) fn private_stage(
    checkpoint: &crate::CheckpointConfig,
) -> std::io::Result<record_recovery::PrivateStaging> {
    record_recovery::PrivateStaging::create(
        Path::new(&checkpoint.manifest_dir),
        "screen-writer",
        "segment.mp4",
    )
}

pub(crate) struct NativeSegment(NonNull<c_void>, u32);

// The native owner serializes AVAssetWriter work on its own queue. Its public
// admission and finish operations are additionally serialized by ScreenWriter.
unsafe impl Send for NativeSegment {}

impl NativeSegment {
    pub(crate) fn new(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| "screen movie path contains NUL".to_string())?;
        let mut error = [0; ERROR_CAPACITY];
        let raw = unsafe {
            sxc_screen_writer_create(
                path.as_ptr(),
                width,
                height,
                fps,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        NonNull::new(raw)
            .map(|raw| Self(raw, fps))
            .ok_or_else(|| message(&error))
    }

    fn append(&self, sample: &CMSampleBuffer) -> Result<(), String> {
        let mut error = [0; ERROR_CAPACITY];
        let status = unsafe {
            sxc_screen_writer_append(
                self.0.as_ptr(),
                sample.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        (status == 0).then_some(()).ok_or_else(|| message(&error))
    }

    pub(crate) fn finish(&self, end_pts_ns: i64) -> Result<(), String> {
        let mut error = [0; ERROR_CAPACITY];
        let status = unsafe {
            sxc_screen_writer_finish(self.0.as_ptr(), end_pts_ns, error.as_mut_ptr(), error.len())
        };
        (status == 0).then_some(()).ok_or_else(|| message(&error))
    }

    #[cfg(test)]
    fn stats(&self) -> (i64, i64, u64) {
        let (mut first, mut last, mut count) = (0, 0, 0);
        assert_eq!(
            unsafe { sxc_screen_writer_stats(self.0.as_ptr(), &mut first, &mut last, &mut count) },
            0
        );
        (first, last, count)
    }
}

impl Drop for NativeSegment {
    fn drop(&mut self) {
        unsafe { sxc_screen_writer_destroy(self.0.as_ptr()) }
    }
}

#[derive(Clone)]
pub(crate) struct ScreenWriter(Arc<Mutex<State>>, Arc<DispatchQueue>);

struct State {
    current: Option<NativeSegment>,
    origin_ns: i64,
    first_pts_ns: Option<i64>,
    last_pts_ns: Option<i64>,
    last_duration_ns: i64,
    nominal_duration_ns: i64,
    stop_pts_ns: Option<i64>,
    last_arrival: Instant,
    frames: u64,
    failure: Option<String>,
}

pub(crate) struct Boundary {
    pub(crate) pts_ns: i64,
    pub(crate) elapsed_ms: u64,
    pub(crate) window: FrameWindow,
}

impl ScreenWriter {
    pub(crate) fn new(segment: NativeSegment) -> Self {
        let nominal_duration_ns = 1_000_000_000 / i64::from(segment.1);
        Self(
            Arc::new(Mutex::new(State {
                current: Some(segment),
                origin_ns: clock_now_ns(),
                first_pts_ns: None,
                last_pts_ns: None,
                last_duration_ns: 0,
                nominal_duration_ns,
                stop_pts_ns: None,
                last_arrival: Instant::now(),
                frames: 0,
                failure: None,
            })),
            Arc::new(DispatchQueue::new(
                "cut.screen.samples",
                DispatchQoS::UserInteractive,
            )),
        )
    }

    pub(crate) fn delivery_queue(&self) -> &DispatchQueue {
        &self.1
    }

    /// Call after SCK acknowledges Stop and before closing sample admission.
    pub(crate) fn drain(&self) -> Result<(), String> {
        let mut error = [0; ERROR_CAPACITY];
        let result = unsafe {
            sxc_screen_writer_drain_queue(self.1.as_ptr(), error.as_mut_ptr(), error.len())
        };
        (result == 0).then_some(()).ok_or_else(|| message(&error))
    }

    pub(crate) fn set_origin(&self, origin: Instant) {
        let mut state = self.lock();
        state.origin_ns = clock_now_ns()
            .saturating_sub(i64::try_from(origin.elapsed().as_nanos()).unwrap_or(i64::MAX));
    }

    /// Called on every delivered picture. The native side retains the sample
    /// before returning; preview readback and container completion cannot own it.
    pub(crate) fn append(&self, sample: &CMSampleBuffer) {
        let mut state = self.lock();
        if state.failure.is_some() {
            return;
        }
        let Some(current) = state.current.as_ref() else {
            return;
        };
        let pts = unsafe { sxc_screen_writer_sample_pts_ns(sample.as_ptr()) };
        if state.stop_pts_ns.is_some_and(|end| pts >= end) {
            return;
        }
        match current.append(sample) {
            Ok(()) => {
                let ns = unsafe { sxc_screen_writer_sample_pts_ns(sample.as_ptr()) };
                state.first_pts_ns.get_or_insert(ns);
                state.last_pts_ns = Some(ns);
                state.last_duration_ns = sample
                    .duration()
                    .as_seconds()
                    .map(|seconds| (seconds * 1_000_000_000.0) as i64)
                    .unwrap_or(0);
                state.last_arrival = Instant::now();
                state.frames = state.frames.saturating_add(1);
            }
            Err(error) => state.failure = Some(error),
        }
    }

    pub(crate) fn request_stop(&self) {
        self.lock().stop_pts_ns.get_or_insert_with(clock_now_ns);
    }

    pub(crate) fn window(&self) -> Result<FrameWindow, String> {
        let state = self.lock();
        state.check()?;
        Ok(state.window())
    }

    /// Installing the next writer and closing admission to the old one is one
    /// short operation. Finish/probe/fsync always run after releasing this lock.
    pub(crate) fn switch(
        &self,
        next: Option<NativeSegment>,
    ) -> Result<(NativeSegment, Boundary), String> {
        let mut state = self.lock();
        state.check()?;
        let wall_ns = state.stop_pts_ns.unwrap_or_else(clock_now_ns);
        // Use the sample clock for an active moving screen. Callback backlog
        // under load must not move a checkpoint forward on the wall clock.
        // A still screen retains its elapsed held-picture interval.
        let pts_ns = if next.is_some() && state.last_arrival.elapsed().as_millis() <= 250 {
            state
                .last_pts_ns
                .map(|pts| {
                    pts.saturating_add(if state.last_duration_ns > 0 {
                        state.last_duration_ns
                    } else {
                        state.nominal_duration_ns
                    })
                })
                .unwrap_or(wall_ns)
        } else {
            wall_ns
        };
        let boundary = Boundary {
            pts_ns,
            elapsed_ms: state.elapsed_ms(pts_ns),
            window: state.window(),
        };
        let old = std::mem::replace(&mut state.current, next)
            .ok_or_else(|| "screen writer is already closed".to_string())?;
        state.frames = 0;
        state.first_pts_ns = None;
        state.last_pts_ns = None;
        Ok((old, boundary))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl State {
    fn check(&self) -> Result<(), String> {
        self.failure
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }

    fn elapsed_ms(&self, pts_ns: i64) -> u64 {
        pts_ns.saturating_sub(self.origin_ns).max(0) as u64 / 1_000_000
    }

    fn window(&self) -> FrameWindow {
        FrameWindow {
            frames: self.frames,
            first_ms: self.first_pts_ns.map(|pts| self.elapsed_ms(pts)),
        }
    }
}

pub(crate) fn clock_now_ns() -> i64 {
    unsafe { sxc_screen_writer_clock_now_ns() }
}

fn message(error: &[c_char]) -> String {
    unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

extern "C" {
    fn sxc_screen_writer_create(
        path: *const c_char,
        width: u32,
        height: u32,
        fps: u32,
        error: *mut c_char,
        capacity: usize,
    ) -> *mut c_void;
    fn sxc_screen_writer_append(
        ctx: *mut c_void,
        sample: *mut c_void,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn sxc_screen_writer_finish(
        ctx: *mut c_void,
        end_pts_ns: i64,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn sxc_screen_writer_destroy(ctx: *mut c_void);
    fn sxc_screen_writer_drain_queue(
        queue: *const c_void,
        error: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn sxc_screen_writer_clock_now_ns() -> i64;
    fn sxc_screen_writer_sample_pts_ns(sample: *mut c_void) -> i64;
    #[cfg(test)]
    fn sxc_screen_writer_stats(
        ctx: *mut c_void,
        first: *mut i64,
        last: *mut i64,
        count: *mut u64,
    ) -> i32;
}

#[cfg(test)]
#[path = "macos_screen_writer_tests.rs"]
mod tests;
