//! Private Windows camera adapter backed by Media Foundation Capture Engine.
//!
//! Device discovery and the native recording owner feed the shared camera API.
//! Recording runs use the existing `CameraRuntime` after their owner reserves
//! the capture directory; server verbs and UI controls live in their own modules.

#[path = "windows_camera_devices.rs"]
mod windows_camera_devices;
#[path = "windows_camera_record.rs"]
mod windows_camera_record;
#[path = "windows_camera_run.rs"]
mod windows_camera_run;
#[path = "windows_camera_signals.rs"]
mod windows_camera_signals;

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use record_core::{error_codes, CameraTerminalState, RecordError, Result};

use crate::camera_runtime::{CameraRuntime, CameraRuntimeAdapter};
use crate::camera_session::{CameraSessionBackend, CameraStopOutcome};
use crate::{
    CameraDevice, CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent,
};
use windows_camera_devices::{enumerate_devices, find_device};
use windows_camera_run::NativeCameraRun;

/// Build the registry only for a recording owner that has already reserved its
/// capture directory. Constructing the registry performs no enumeration and no
/// privacy/permission work.
pub(crate) fn private_runtime(capture_directory: &Path) -> Result<CameraRuntime> {
    if capture_directory.as_os_str().is_empty() {
        return Err(camera_error(
            "reserve Windows camera runtime",
            "capture directory is empty",
        ));
    }
    Ok(CameraRuntime::with_adapter(Box::new(
        WindowsCameraAdapter {
            capture_directory: capture_directory.to_path_buf(),
            active: None,
            pending_observations: Vec::new(),
            last_terminal_device_lost: false,
            refusal: Mutex::new(None),
        },
    )))
}

pub(crate) fn private_devices() -> Result<Vec<CameraDevice>> {
    enumerate_devices().map(|devices| {
        devices
            .into_iter()
            .enumerate()
            .map(|(index, mut source)| {
                source.device.label = format!("Camera {}", index + 1);
                source.device
            })
            .collect()
    })
}

pub(crate) fn private_readiness(device_id: &str) -> CameraReadiness {
    match find_device(device_id) {
        Ok(Some(source)) => CameraReadiness::PermissionRequired {
            device: source.device,
            detail:
                "Camera is available. Starting a camera recording may ask for Windows permission."
                    .into(),
        },
        Ok(None) => CameraReadiness::Missing {
            detail: "The selected camera is no longer connected.".into(),
        },
        Err(_) => CameraReadiness::Missing {
            detail: "Windows camera enumeration is unavailable.".into(),
        },
    }
}

struct WindowsCameraAdapter {
    capture_directory: std::path::PathBuf,
    active: Option<NativeCameraRun>,
    pending_observations: Vec<CameraFrameObservation>,
    last_terminal_device_lost: bool,
    /// A post-intent state only. `readiness` never creates it by trying to
    /// open the device, which would make passive UI state prompt-capable.
    refusal: Mutex<Option<CameraReadiness>>,
}

impl CameraRuntimeAdapter for WindowsCameraAdapter {
    fn enumerate(&self) -> Result<Vec<CameraDevice>> {
        enumerate_devices().map(|devices| devices.into_iter().map(|source| source.device).collect())
    }
}

impl CameraSessionBackend for WindowsCameraAdapter {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness {
        if let Some(refusal) = self
            .refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|state| readiness_device_id(state) == Some(request.device_id.as_str()))
        {
            return refusal.clone();
        }
        match find_device(&request.device_id) {
            Ok(Some(source)) => CameraReadiness::PermissionRequired {
                device: source.device,
                detail: "selected Windows camera is enumerated; explicit use is required before Media Foundation opens it".into(),
            },
            Ok(None) => CameraReadiness::Missing {
                detail: "selected Windows camera is no longer enumerated".into(),
            },
            Err(_) => CameraReadiness::Missing {
                detail: "Windows camera enumeration did not return a usable selected device".into(),
            },
        }
    }

    fn start(&mut self, intent: &CameraUseIntent, screen_origin: Instant) -> Result<()> {
        if self.active.is_some() {
            return Err(camera_error(
                "start Windows camera capture",
                "the adapter already owns a camera run",
            ));
        }
        self.pending_observations.clear();
        self.last_terminal_device_lost = false;
        let device = CameraDevice {
            id: intent.request().device_id.clone(),
            label: "Windows camera".into(),
        };
        match NativeCameraRun::start(
            &self.capture_directory,
            intent.request().capture_id.as_str(),
            intent.request().device_id.as_str(),
            screen_origin,
        ) {
            Ok(run) => {
                *self
                    .refusal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                self.active = Some(run);
                Ok(())
            }
            Err(error) => {
                let refusal = if is_access_denied(&error) {
                    CameraReadiness::PermissionDenied {
                        device,
                        detail: "Windows denied explicit camera use".into(),
                    }
                } else if error.message.contains("first delivered camera frame") {
                    CameraReadiness::NoFrame {
                        device,
                        detail: "Windows Camera Capture Engine started without a bounded first delivered camera frame".into(),
                    }
                } else {
                    return Err(error);
                };
                *self
                    .refusal
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(refusal);
                Err(error)
            }
        }
    }

    fn stop(&mut self, _terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        let run = self.active.take().ok_or_else(|| {
            camera_error(
                "stop Windows camera capture",
                "the adapter has no active camera run",
            )
        })?;
        let stopped = run.stop()?;
        self.last_terminal_device_lost = stopped.device_lost;
        self.pending_observations = stopped.observations;
        match stopped.seal {
            Some(seal) => Ok(CameraStopOutcome::Sealed(seal)),
            None => Ok(CameraStopOutcome::NoMedia),
        }
    }

    fn take_frame_observations(&mut self) -> Result<Vec<CameraFrameObservation>> {
        Ok(std::mem::take(&mut self.pending_observations))
    }

    fn terminal_state(&self, requested: CameraTerminalState) -> CameraTerminalState {
        if self.last_terminal_device_lost {
            CameraTerminalState::DeviceLost
        } else {
            requested
        }
    }
}

fn readiness_device_id(readiness: &CameraReadiness) -> Option<&str> {
    match readiness {
        CameraReadiness::PermissionDenied { device, .. }
        | CameraReadiness::NoFrame { device, .. } => Some(device.id.as_str()),
        CameraReadiness::Missing { .. }
        | CameraReadiness::Enumerated { .. }
        | CameraReadiness::PermissionRequired { .. }
        | CameraReadiness::Busy { .. }
        | CameraReadiness::Ready { .. } => None,
    }
}

pub(super) fn is_access_denied(error: &RecordError) -> bool {
    let combined = format!("{} {}", error.message, error.cause).to_ascii_lowercase();
    combined.contains("access denied") || combined.contains("80070005")
}

pub(super) fn windows_error(stage: &'static str, error: impl std::fmt::Display) -> RecordError {
    camera_error(stage, error.to_string())
}

pub(super) fn camera_error(stage: &'static str, cause: impl Into<String>) -> RecordError {
    RecordError::new(error_codes::CAPTURE, stage, cause.into())
}
