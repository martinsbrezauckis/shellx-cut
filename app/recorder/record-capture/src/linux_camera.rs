//! Explicit-use Linux camera adapter. Passive paths use only the C1 sysfs inventory.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::Instant;

use crate::camera_finalization::{
    finalize_staged_camera_media, DeclaredCameraMediaFacts, StagedCameraMedia,
};
use crate::camera_finalization_owner::CameraCaptureDirectory;
use crate::camera_runtime::{CameraRuntime, CameraRuntimeAdapter};
use crate::camera_session::{CameraSessionBackend, CameraStopOutcome};
use crate::linux_camera_devices::{self, LinuxCameraNode};
use crate::{
    CameraDevice, CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent,
};
use record_core::{error_codes, CameraTerminalState, RecordError, Result};

const FIRST_FRAME_MS: u32 = 2_000;
const DRAIN_MS: u32 = 5_000;

#[path = "linux_camera_native.rs"]
mod native;
use native::*;

pub(crate) fn private_runtime(capture_directory: &Path) -> Result<CameraRuntime> {
    let capture = CameraCaptureDirectory::reserve(capture_directory)?;
    Ok(CameraRuntime::with_adapter(Box::new(LinuxCameraAdapter {
        capture_directory: capture_directory.to_owned(),
        capture,
        active: None,
        observations: Vec::new(),
        refusal: None,
        device_lost: false,
    })))
}

pub(crate) fn private_devices() -> Result<Vec<CameraDevice>> {
    Ok(linux_camera_devices::devices()?
        .into_iter()
        .map(|node| public_device(&node))
        .collect())
}

pub(crate) fn private_readiness(device_id: &str) -> CameraReadiness {
    match selected(device_id) {
        Ok(Some(node)) => CameraReadiness::PermissionRequired {
            device: public_device(&node),
            detail: "Start recording to open and check this camera.".into(),
        },
        Ok(None) => CameraReadiness::Missing {
            detail: "The selected Linux camera is disconnected or its identity changed.".into(),
        },
        Err(_) => CameraReadiness::Missing {
            detail: "Linux camera inventory metadata is unavailable.".into(),
        },
    }
}

struct LinuxCameraAdapter {
    capture_directory: PathBuf,
    capture: CameraCaptureDirectory,
    active: Option<NativeRun>,
    observations: Vec<CameraFrameObservation>,
    refusal: Option<(String, CameraReadiness)>,
    device_lost: bool,
}

impl CameraRuntimeAdapter for LinuxCameraAdapter {
    fn enumerate(&self) -> Result<Vec<CameraDevice>> {
        private_devices()
    }
}

impl CameraSessionBackend for LinuxCameraAdapter {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness {
        if self.active.is_some() {
            return CameraReadiness::Busy {
                detail: "The previous Linux camera owner has not completed native shutdown.".into(),
            };
        }
        if let Some((id, refusal)) = &self.refusal {
            if id == &request.device_id {
                return refusal.clone();
            }
        }
        private_readiness(&request.device_id)
    }

    fn start(&mut self, intent: &CameraUseIntent, screen_origin: Instant) -> Result<()> {
        if self.active.is_some() {
            return Err(error(
                "start Linux camera",
                "another native camera run is active",
            ));
        }
        self.observations.clear();
        self.device_lost = false;
        let request = intent.request();
        let node = selected(&request.device_id)?.ok_or_else(|| {
            error(
                "start Linux camera",
                "the selected camera was disconnected or replaced",
            )
        })?;
        let missing = unsafe { sxc_linux_camera_missing_plugin() };
        if !missing.is_null() {
            let name = unsafe { CStr::from_ptr(missing) }.to_string_lossy();
            return Err(error(
                "start Linux camera",
                &format!("required GStreamer plugin {name} is unavailable"),
            ));
        }
        let selected_node = CString::new(node.node.as_os_str().as_bytes())
            .map_err(|_| error("start Linux camera", "selected device path contains NUL"))?;
        let capture = self.capture.capture_dir()?;
        capture.ensure_plain_child(c"camera")?;
        let writer = capture.nameless_writable_stage()?;
        let stage_name = CString::new(format!("/proc/self/fd/{}", writer.as_raw_fd()))
            .expect("numeric descriptor path has no NUL");
        let mut reason = [0_i8; 256];
        let pointer = unsafe {
            sxc_linux_camera_start(
                selected_node.as_ptr(),
                node.rdev,
                node.node_inode,
                stage_name.as_ptr(),
                reason.as_mut_ptr(),
                reason.len() as u32,
            )
        };
        let pointer = match NonNull::new(pointer) {
            Some(pointer) => pointer,
            None => {
                let diagnosis = unsafe { CStr::from_ptr(reason.as_ptr()) }.to_string_lossy();
                self.refusal = refusal_for(&node, &diagnosis);
                return Err(error(
                    "start Linux camera",
                    reason_kind(&diagnosis).detail(),
                ));
            }
        };
        let mut run = NativeRun {
            pointer,
            writer: Some(writer),
            gst_clock_ns: 0,
            clock_anchor: None,
            screen_origin,
        };
        // The source's prepare-format callback validates the actual opened fd.
        // Re-resolve C1's opaque identity after that open to reject a replacement
        // during GStreamer's transition into PLAYING.
        let first = unsafe { sxc_linux_camera_first(run.ptr(), FIRST_FRAME_MS) };
        if first != 0 {
            let diagnosis = run.diagnosis();
            let _ = run.retire();
            if first == 1 {
                self.refusal = Some((
                    node.id.clone(),
                    CameraReadiness::NoFrame {
                        device: public_device(&node),
                        detail: "The selected Linux camera opened but delivered no native frame within two seconds.".into(),
                    },
                ));
            } else {
                self.refusal = refusal_for(&node, &diagnosis);
            }
            return Err(error(
                "start Linux camera",
                reason_kind(&diagnosis).detail(),
            ));
        }
        let after = selected(&request.device_id)?;
        if after.as_ref() != Some(&node) {
            let _ = run.retire();
            return Err(error(
                "start Linux camera",
                "selected camera identity changed while opening",
            ));
        }
        let mut gst_ns = 0;
        let mut mono_ns = 0;
        if unsafe { sxc_linux_camera_clock_pair(run.ptr(), &mut gst_ns, &mut mono_ns) } != 0 {
            let _ = run.retire();
            return Err(error(
                "start Linux camera",
                "native monotonic clock calibration failed",
            ));
        }
        run.gst_clock_ns = gst_ns;
        run.clock_anchor = Some(monotonic_instant(mono_ns)?);
        self.refusal = None;
        self.active = Some(run);
        Ok(())
    }

    fn stop(&mut self, requested: CameraTerminalState) -> Result<CameraStopOutcome> {
        let run = self
            .active
            .as_mut()
            .ok_or_else(|| error("stop Linux camera", "no active native camera"))?;
        // A failed NULL transition retains the entire run and its stage writer
        // in this adapter. The session may fail, but it cannot free live
        // callback userdata or misreport a closed artifact.
        let drained = run.retire()?;
        if !drained {
            self.device_lost = true;
            let diagnosis = run.diagnosis();
            self.active.take(); // NULL succeeded; drop the unsealed nameless stage.
            return Err(error("stop Linux camera", reason_kind(&diagnosis).detail()));
        }
        let mut run = self.active.take().expect("retired run remains owned");
        let count = unsafe { sxc_linux_camera_count(run.ptr()) };
        if count == 0 {
            return Ok(CameraStopOutcome::NoMedia);
        }
        let (start_ns, end_ns) = interval_bounds(&run)?;
        let (width, height, fps_num, fps_den) = shape(&run)?;
        let start = clock_instant(&run, start_ns)?;
        let end = clock_instant(&run, end_ns)?;
        let (projected, duration_ms) = project_interval(start, end, run.screen_origin)?;
        let artifact_id = artifact_id(&self.capture_directory, requested);
        let staged = StagedCameraMedia {
            artifact_id,
            video: "camera/linux_camera.mp4".into(),
            declared: DeclaredCameraMediaFacts {
                width,
                height,
                fps_num,
                fps_den,
                frame_count: u64::from(count),
                duration_ms,
            },
        };
        let writer = run
            .writer
            .take()
            .expect("active camera retains its stage writer");
        let read_only = File::open(format!("/proc/self/fd/{}", writer.as_raw_fd()))
            .map_err(|cause| error("close Linux camera media", &cause.to_string()))?;
        drop(writer);
        drop(run);
        let mut closer = ClosedStage(Some(read_only));
        let seal =
            finalize_staged_camera_media(&mut closer, &LinuxMediaProbe, &self.capture, &staged)?;
        self.observations.push(projected);
        Ok(CameraStopOutcome::Sealed(seal))
    }

    fn take_frame_observations(&mut self) -> Result<Vec<CameraFrameObservation>> {
        Ok(std::mem::take(&mut self.observations))
    }

    fn terminal_state(&self, requested: CameraTerminalState) -> CameraTerminalState {
        if self.device_lost {
            CameraTerminalState::DeviceLost
        } else {
            requested
        }
    }
}

fn selected(id: &str) -> Result<Option<LinuxCameraNode>> {
    Ok(linux_camera_devices::devices()?
        .into_iter()
        .find(|node| node.id == id))
}

fn public_device(node: &LinuxCameraNode) -> CameraDevice {
    CameraDevice {
        id: node.id.clone(),
        label: node.label.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeReason {
    Permission,
    Busy,
    Disconnected,
    Other,
}

impl NativeReason {
    fn detail(self) -> &'static str {
        match self {
            Self::Permission => "selected V4L2 camera permission was denied",
            Self::Busy => "selected V4L2 camera is busy",
            Self::Disconnected => "selected V4L2 camera disconnected",
            Self::Other => "native V4L2 source, encoder, or muxer admission failed",
        }
    }
}

fn reason_kind(diagnosis: &str) -> NativeReason {
    let lower = diagnosis.to_ascii_lowercase();
    if lower.contains("permission") || lower.contains("access denied") {
        NativeReason::Permission
    } else if lower.contains("busy") {
        NativeReason::Busy
    } else if lower.contains("no such device") || lower.contains("disconnected") {
        NativeReason::Disconnected
    } else {
        NativeReason::Other
    }
}

fn refusal_for(node: &LinuxCameraNode, diagnosis: &str) -> Option<(String, CameraReadiness)> {
    let readiness = match reason_kind(diagnosis) {
        NativeReason::Permission => CameraReadiness::PermissionDenied {
            device: public_device(node),
            detail: "Linux denied access to the selected camera device.".into(),
        },
        NativeReason::Busy => CameraReadiness::Busy {
            detail: "The selected Linux camera is busy in another capture.".into(),
        },
        NativeReason::Disconnected | NativeReason::Other => return None,
    };
    Some((node.id.clone(), readiness))
}

#[path = "linux_camera_timing.rs"]
mod timing;
use timing::*;

#[path = "linux_camera_media.rs"]
mod media;
use media::*;

fn error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
}

#[cfg(test)]
#[path = "linux_camera_tests.rs"]
mod tests;
