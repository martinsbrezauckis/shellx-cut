//! Safe ownership around the AVFoundation camera helper.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result};
use serde::Deserialize;

use crate::macos_camera_finalization::MacCameraStage;
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
}

unsafe impl Send for NativeCameraRun {}

impl NativeCameraRun {
    pub(super) fn start(
        capture_dir: &std::path::Path,
        capture_id: &str,
        device_uid: &str,
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
        })
    }

    pub(super) fn stop(mut self) -> Result<StoppedCameraRun> {
        let device_lost = self.stop_native()?;
        let observations = self.collector.take()?;
        let seal = self
            .stage
            .take()
            .ok_or_else(|| error("finalize macOS camera", "camera stage is missing"))?
            .finalize()?;
        Ok(StoppedCameraRun {
            observations,
            seal,
            device_lost,
        })
    }

    fn stop_native(&mut self) -> Result<bool> {
        let Some(handle) = self.handle.take() else {
            return Ok(false);
        };
        let mut message = [0_i8; ERROR_CAPACITY];
        let mut device_lost = 0_i32;
        let status = unsafe {
            sxc_macos_camera_stop(
                handle.as_ptr(),
                message.as_mut_ptr(),
                message.len(),
                &mut device_lost,
            )
        };
        if status == 0 {
            Ok(device_lost != 0)
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
    observations: Vec<CameraFrameObservation>,
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
        state
            .observations
            .push(CameraFrameObservation::new(started_at, ended_at));
    }

    fn take(&self) -> Result<Vec<CameraFrameObservation>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.observations.is_empty() {
            return Err(error(
                "finalize macOS camera",
                "AVFoundation delivered no camera frames",
            ));
        }
        Ok(std::mem::take(&mut state.observations))
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
    ) -> i32;
}
