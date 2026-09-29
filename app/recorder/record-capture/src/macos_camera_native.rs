//! Safe ownership around the AVFoundation camera helper.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result};
use serde::Deserialize;

use crate::macos_camera_finalization::MacCameraStage;
use crate::macos_camera_movie_timing::NativeMovieTiming;
use crate::CameraFrameObservation;

const ERROR_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct NativeCameraDevice {
    pub(super) uid: String,
    pub(super) label: String,
}

pub(super) fn devices() -> Result<Vec<NativeCameraDevice>> {
    let required = unsafe { sxc_macos_camera_devices_json(std::ptr::null_mut(), 0) };
    if required == 0 || required > 1024 * 1024 {
        return Err(error(
            "enumerate macOS cameras",
            "AVFoundation returned invalid device metadata",
        ));
    }
    let mut bytes = vec![0_u8; required];
    let written = unsafe { sxc_macos_camera_devices_json(bytes.as_mut_ptr().cast(), bytes.len()) };
    if written != required || bytes.last() != Some(&0) {
        return Err(error(
            "enumerate macOS cameras",
            "AVFoundation device metadata changed during enumeration",
        ));
    }
    serde_json::from_slice(&bytes[..bytes.len() - 1])
        .map_err(|cause| error("decode macOS cameras", &cause.to_string()))
}

pub(super) fn authorization() -> i32 {
    unsafe { sxc_macos_camera_authorization() }
}

pub(super) struct NativeCameraRun {
    handle: Option<NonNull<c_void>>,
    collector: Box<FrameCollector>,
    stage: Option<MacCameraStage>,
    screen_origin: Instant,
}

unsafe impl Send for NativeCameraRun {}

impl NativeCameraRun {
    pub(super) fn start(
        capture_dir: &std::path::Path,
        capture_id: &str,
        device_uid: &str,
        screen_origin: Instant,
    ) -> Result<Self> {
        let stage = MacCameraStage::prepare(capture_dir, capture_id)?;
        let uid = CString::new(device_uid)
            .map_err(|_| error("start macOS camera", "camera identity contains a NUL byte"))?;
        let output =
            CString::new(stage.stage_path().as_os_str().as_encoded_bytes()).map_err(|_| {
                error(
                    "start macOS camera",
                    "camera output path contains a NUL byte",
                )
            })?;
        let mut collector = Box::new(FrameCollector::default());
        let mut message = [0_i8; ERROR_CAPACITY];
        let handle = unsafe {
            sxc_macos_camera_start(
                uid.as_ptr(),
                output.as_ptr(),
                observe_frame,
                collector.as_mut() as *mut FrameCollector as *mut c_void,
                message.as_mut_ptr(),
                message.len(),
            )
        };
        let handle =
            NonNull::new(handle).ok_or_else(|| native_error("start macOS camera", &message))?;
        Ok(Self {
            handle: Some(handle),
            collector,
            stage: Some(stage),
            screen_origin,
        })
    }

    pub(super) fn stop(mut self) -> Result<StoppedCameraRun> {
        let (device_lost, native_timing) = self.stop_native()?;
        let anchor = self.collector.take_anchor()?;
        // Preserve the pre-publication native clock admission. The sealed
        // movie may later name a presentation start after its warmup edit.
        anchor.movie_start(native_timing.start_pts_ns)?;
        let (seal, observations) = self
            .stage
            .take()
            .ok_or_else(|| error("finalize macOS camera", "camera stage is missing"))?
            .finalize(native_timing, |movie_timing| {
                let movie_start = anchor.movie_start(movie_timing.start_pts_ns)?;
                let first_offset = movie_start
                    .checked_duration_since(self.screen_origin)
                    .ok_or_else(|| {
                        error(
                            "finalize macOS camera",
                            "movie starts before the screen CaptureClock",
                        )
                    })?;
                let first_offset_ms = u64::try_from(first_offset.as_millis()).map_err(|_| {
                    error(
                        "finalize macOS camera",
                        "movie CaptureClock offset overflowed",
                    )
                })?;
                // MovieFileOutput is the encoded stream. Report its verified
                // presentation interval, never DataOutput warmup callbacks.
                Ok(vec![movie_observation(
                    self.screen_origin,
                    first_offset_ms,
                    movie_timing.duration_ms,
                )?])
            })?;
        Ok(StoppedCameraRun {
            observations,
            seal,
            device_lost,
        })
    }

    fn stop_native(&mut self) -> Result<(bool, NativeMovieTiming)> {
        let Some(handle) = self.handle.take() else {
            return Err(error(
                "stop macOS camera",
                "native camera handle is already stopped",
            ));
        };
        let mut message = [0_i8; ERROR_CAPACITY];
        let mut device_lost = 0_i32;
        let mut timing = NativeMovieTiming {
            start_pts_ns: 0,
            last_pts_ns: 0,
            last_duration_ns: 0,
            last_cadence_ns: 0,
        };
        let status = unsafe {
            sxc_macos_camera_stop(
                handle.as_ptr(),
                message.as_mut_ptr(),
                message.len(),
                &mut device_lost,
                &mut timing.start_pts_ns,
                &mut timing.last_pts_ns,
                &mut timing.last_duration_ns,
                &mut timing.last_cadence_ns,
            )
        };
        if status == 0 {
            Ok((device_lost != 0, timing))
        } else {
            Err(native_error("stop macOS camera", &message))
        }
    }
}

impl Drop for NativeCameraRun {
    fn drop(&mut self) {
        let _ = self.stop_native();
    }
}

pub(super) struct StoppedCameraRun {
    pub(super) observations: Vec<CameraFrameObservation>,
    pub(super) seal: crate::camera_finalization::CameraMediaSeal,
    pub(super) device_lost: bool,
}

#[derive(Default)]
struct FrameCollector {
    state: Mutex<FrameCollectorState>,
}

#[derive(Default)]
struct FrameCollectorState {
    first_pts_ns: Option<u64>,
    origin: Option<Instant>,
    last_end: Option<Instant>,
    has_accepted_frame: bool,
}

impl FrameCollector {
    fn observe(&self, pts_ns: u64, duration_ns: u64) {
        if duration_ns == 0 {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let first_pts = *state.first_pts_ns.get_or_insert(pts_ns);
        let origin = *state.origin.get_or_insert_with(Instant::now);
        let Some(offset_ns) = pts_ns.checked_sub(first_pts) else {
            return;
        };
        let Some(started_at) = origin.checked_add(Duration::from_nanos(offset_ns)) else {
            return;
        };
        let Some(ended_at) = started_at.checked_add(Duration::from_nanos(duration_ns)) else {
            return;
        };
        if ended_at <= started_at || state.last_end.is_some_and(|last| started_at < last) {
            return;
        }
        state.last_end = Some(ended_at);
        state.has_accepted_frame = true;
    }

    fn take_anchor(&self) -> Result<SampleAnchor> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.has_accepted_frame {
            return Err(error(
                "finalize macOS camera",
                "AVFoundation delivered no camera frames",
            ));
        }
        Ok(SampleAnchor {
            first_pts_ns: state
                .first_pts_ns
                .ok_or_else(|| error("finalize macOS camera", "first DataOutput PTS is missing"))?,
            arrived_at: state.origin.ok_or_else(|| {
                error(
                    "finalize macOS camera",
                    "first DataOutput arrival is missing",
                )
            })?,
        })
    }
}

struct SampleAnchor {
    first_pts_ns: u64,
    arrived_at: Instant,
}

impl SampleAnchor {
    fn movie_start(&self, start_pts_ns: u64) -> Result<Instant> {
        let offset_ns = self.first_pts_ns.checked_sub(start_pts_ns).ok_or_else(|| {
            error(
                "finalize macOS camera",
                "DataOutput begins before movie start",
            )
        })?;
        self.arrived_at
            .checked_sub(Duration::from_nanos(offset_ns))
            .ok_or_else(|| {
                error(
                    "finalize macOS camera",
                    "movie start cannot map to CaptureClock",
                )
            })
    }
}

fn movie_observation(
    screen_origin: Instant,
    first_offset_ms: u64,
    verified_duration_ms: u64,
) -> Result<CameraFrameObservation> {
    let first = screen_origin
        .checked_add(Duration::from_millis(first_offset_ms))
        .ok_or_else(|| {
            error(
                "finalize macOS camera",
                "movie CaptureClock start overflowed",
            )
        })?;
    let end = first
        .checked_add(Duration::from_millis(verified_duration_ms))
        .ok_or_else(|| error("finalize macOS camera", "movie CaptureClock end overflowed"))?;
    Ok(CameraFrameObservation::new(first, end))
}

#[cfg(test)]
mod movie_clock_tests {
    use super::*;

    #[test]
    fn data_output_anchor_keeps_constant_state_for_long_recordings() {
        let collector = FrameCollector::default();
        assert!(collector.take_anchor().is_err());
        for frame in 0..100_000_u64 {
            collector.observe(1_000_000_000 + frame * 33_333_333, 33_333_333);
        }
        let anchor = collector.take_anchor().unwrap();
        assert_eq!(anchor.first_pts_ns, 1_000_000_000);
        let state = collector
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(state.has_accepted_frame);
        let last_end = state.last_end.unwrap();
        drop(state);
        collector.observe(1_000_000_000, 33_333_333);
        assert_eq!(collector.state.lock().unwrap().last_end, Some(last_end));
    }

    #[test]
    fn data_output_anchor_projects_the_verified_movie_not_its_own_shorter_interval() {
        let screen_origin = Instant::now();
        let movie_start_pts_ns = 34_421_447_440_000;
        let anchor = SampleAnchor {
            first_pts_ns: movie_start_pts_ns + 100_030_000,
            arrived_at: screen_origin + Duration::from_millis(2_000),
        };
        let movie_start = anchor.movie_start(movie_start_pts_ns).unwrap();
        let first_offset_ms = movie_start.duration_since(screen_origin).as_millis() as u64;
        let interval = movie_observation(screen_origin, first_offset_ms, 7_992).unwrap();
        assert_eq!(first_offset_ms, 1_899);
        assert_eq!(
            interval
                .ended_at
                .duration_since(interval.started_at)
                .as_millis(),
            7_992
        );
        assert_ne!(
            interval
                .ended_at
                .duration_since(interval.started_at)
                .as_millis(),
            7_966
        );
        assert_eq!(
            interval.ended_at.duration_since(screen_origin).as_millis(),
            9_891
        );
        assert!(SampleAnchor {
            first_pts_ns: movie_start_pts_ns - 1,
            arrived_at: screen_origin,
        }
        .movie_start(movie_start_pts_ns)
        .is_err());
    }
}

unsafe extern "C" fn observe_frame(context: *mut c_void, pts_ns: u64, duration_ns: u64) {
    if let Some(collector) = unsafe { (context as *const FrameCollector).as_ref() } {
        collector.observe(pts_ns, duration_ns);
    }
}

fn native_error(stage: &str, bytes: &[c_char]) -> RecordError {
    let detail = unsafe { CStr::from_ptr(bytes.as_ptr()) }.to_string_lossy();
    error(
        stage,
        if detail.is_empty() {
            "AVFoundation camera operation failed"
        } else {
            &detail
        },
    )
}

fn error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
}

unsafe extern "C" {
    fn sxc_macos_camera_devices_json(buffer: *mut c_char, capacity: usize) -> usize;
    fn sxc_macos_camera_authorization() -> i32;
    fn sxc_macos_camera_start(
        device_uid: *const c_char,
        output_path: *const c_char,
        callback: unsafe extern "C" fn(*mut c_void, u64, u64),
        context: *mut c_void,
        error: *mut c_char,
        error_capacity: usize,
    ) -> *mut c_void;
    fn sxc_macos_camera_stop(
        handle: *mut c_void,
        error: *mut c_char,
        error_capacity: usize,
        device_lost: *mut i32,
        movie_start_pts_ns: *mut u64,
        movie_last_pts_ns: *mut u64,
        movie_last_duration_ns: *mut u64,
        movie_last_cadence_ns: *mut u64,
    ) -> i32;
}
