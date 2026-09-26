//! AVFoundation camera adapter behind the shared explicit-use runtime.

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use record_core::{error_codes, CameraTerminalState, RecordError, Result};
use sha2::{Digest, Sha256};

use crate::camera_runtime::{CameraRuntime, CameraRuntimeAdapter};
use crate::camera_session::{CameraSessionBackend, CameraStopOutcome};
use crate::macos_camera_native::{authorization, devices, NativeCameraDevice, NativeCameraRun};
use crate::{
    CameraDevice, CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent,
};

pub(crate) fn private_runtime(capture_directory: &Path) -> Result<CameraRuntime> {
    if capture_directory.as_os_str().is_empty() {
        return Err(camera_error(
            "reserve macOS camera runtime",
            "capture directory is empty",
        ));
    }
    Ok(CameraRuntime::with_adapter(Box::new(MacosCameraAdapter {
        capture_directory: capture_directory.to_path_buf(),
        active: None,
        pending_observations: Vec::new(),
        device_lost: false,
        refusal: Mutex::new(None),
    })))
}

pub(crate) fn private_devices() -> Result<Vec<CameraDevice>> {
    devices().map(|devices| devices.into_iter().map(public_device).collect())
}

pub(crate) fn private_readiness(device_id: &str) -> CameraReadiness {
    let authorization = authorization();
    match find_device(device_id) {
        Ok(Some(device)) if authorization == 1 || authorization == 3 => {
            CameraReadiness::PermissionDenied {
                device: public_device(device),
                detail: "Camera access is denied in System Settings → Privacy & Security → Camera"
                    .into(),
            }
        }
        Ok(Some(device)) if authorization == 2 => CameraReadiness::Ready {
            device: public_device(device),
            detail: "Camera access is available.".into(),
        },
        Ok(Some(device)) => CameraReadiness::PermissionRequired {
            device: public_device(device),
            detail: "Camera is available. Starting a camera recording may ask for permission."
                .into(),
        },
        Ok(None) => CameraReadiness::Missing {
            detail: "The selected camera is no longer connected.".into(),
        },
        Err(_) => CameraReadiness::Missing {
            detail: "macOS camera enumeration is unavailable.".into(),
        },
    }
}

struct MacosCameraAdapter {
    capture_directory: std::path::PathBuf,
    active: Option<NativeCameraRun>,
    pending_observations: Vec<CameraFrameObservation>,
    device_lost: bool,
    refusal: Mutex<Option<CameraReadiness>>,
}

impl CameraRuntimeAdapter for MacosCameraAdapter {
    fn enumerate(&self) -> Result<Vec<CameraDevice>> {
        private_devices()
    }
}

impl CameraSessionBackend for MacosCameraAdapter {
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
        private_readiness(&request.device_id)
    }

    fn start(&mut self, intent: &CameraUseIntent, screen_origin: Instant) -> Result<()> {
        if self.active.is_some() {
            return Err(camera_error(
                "start macOS camera capture",
                "the adapter already owns a camera run",
            ));
        }
        self.pending_observations.clear();
        self.device_lost = false;
        let selected = find_device(&intent.request().device_id)?.ok_or_else(|| {
            camera_error(
                "start macOS camera capture",
                "the selected camera is no longer connected",
            )
        })?;
        match NativeCameraRun::start(
            &self.capture_directory,
            &intent.request().capture_id,
            &selected.uid,
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
                let detail = format!("{} {}", error.message, error.cause).to_ascii_lowercase();
                let state = if detail.contains("denied") || detail.contains("permission") {
                    Some(CameraReadiness::PermissionDenied {
                        device: public_device(selected),
                        detail: "macOS denied camera access. Enable ShellX Cut in System Settings → Privacy & Security → Camera.".into(),
                    })
                } else if detail.contains("without a camera frame") {
                    Some(CameraReadiness::NoFrame {
                        device: public_device(selected),
                        detail: "The camera opened but did not deliver a frame in time.".into(),
                    })
                } else {
                    None
                };
                if let Some(state) = state {
                    *self
                        .refusal
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state);
                }
                Err(error)
            }
        }
    }

    fn stop(&mut self, _terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        let run = self.active.take().ok_or_else(|| {
            camera_error(
                "stop macOS camera capture",
                "the adapter has no active camera run",
            )
        })?;
        let stopped = run.stop()?;
        self.pending_observations = stopped.observations;
        self.device_lost = stopped.device_lost;
        Ok(CameraStopOutcome::Sealed(stopped.seal))
    }

    fn take_frame_observations(&mut self) -> Result<Vec<CameraFrameObservation>> {
        Ok(std::mem::take(&mut self.pending_observations))
    }

    fn terminal_state(&self, requested: CameraTerminalState) -> CameraTerminalState {
        if self.device_lost {
            CameraTerminalState::DeviceLost
        } else {
            requested
        }
    }
}

fn find_device(device_id: &str) -> Result<Option<NativeCameraDevice>> {
    Ok(devices()?
        .into_iter()
        .find(|device| opaque_id(&device.uid) == device_id))
}

fn public_device(device: NativeCameraDevice) -> CameraDevice {
    CameraDevice {
        id: opaque_id(&device.uid),
        label: sanitize_label(&device.label),
    }
}

fn opaque_id(uid: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"shellx-cut/macos-camera-device/v1\0");
    digest.update(uid.as_bytes());
    format!("mac_camera_{:x}", digest.finalize())
}

fn sanitize_label(label: &str) -> String {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        "Camera".into()
    } else {
        label.chars().take(80).collect()
    }
}

fn readiness_device_id(readiness: &CameraReadiness) -> Option<&str> {
    match readiness {
        CameraReadiness::PermissionDenied { device, .. }
        | CameraReadiness::NoFrame { device, .. } => Some(&device.id),
        _ => None,
    }
}

fn camera_error(stage: &str, cause: impl Into<String>) -> RecordError {
    RecordError::new(error_codes::CAPTURE, stage, cause.into())
}
