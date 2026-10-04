//! Passive public camera capability and explicit-start admission.
//!
//! Doctor may enumerate and inspect permission state, but it never opens a
//! device or asks for permission. A selected `camera_id` reaches the native
//! owner only through `screen_record.start`, which is the user's explicit
//! **Use camera** action.

use cut_core::{error_codes, CutError};
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use record_capture::{CameraDevice, CameraReadiness};
use serde_json::{json, Value};

pub(super) fn capability() -> Value {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        match record_capture::private_camera_owner::devices() {
            Ok(devices) => {
                let rows = devices
                    .iter()
                    .map(|device| device_projection(device, readiness(&device.id)))
                    .collect::<Vec<_>>();
                json!({
                    "supported": true,
                    "devices": rows,
                    "detail": if devices.is_empty() {
                        "No camera is currently connected."
                    } else if cfg!(target_os = "linux") {
                        "Choose a camera, then start recording. Cut opens and checks the selected device only at Start."
                    } else {
                        "Choose a camera, then start recording. Permission is requested only when needed."
                    },
                })
            }
            Err(error) => json!({
                "supported": true,
                "devices": [],
                "detail": format!("Camera discovery is unavailable: {}", error.message),
            }),
        }
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    json!({
        "supported": false,
        "devices": [],
        "detail": "Camera recording is unavailable in this build.",
    })
}

pub(super) fn admit_selected(device_id: &str) -> Result<(), CutError> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        let device_id = device_id.trim();
        let devices = record_capture::private_camera_owner::devices().map_err(record_error)?;
        let device = devices
            .iter()
            .find(|device| device.id == device_id)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "the selected camera is no longer available",
                    "choose a camera from the current Recorder device list",
                )
            })?;
        match readiness(&device.id) {
            CameraReadiness::Ready { .. } | CameraReadiness::PermissionRequired { .. } => Ok(()),
            state => Err(readiness_error(device, &state)),
        }
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = device_id;
        Err(CutError::new(
            error_codes::UNIMPLEMENTED,
            "camera recording is unavailable on this platform",
            "record without Camera or use a Cut build with native camera capture",
        ))
    }
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn readiness(device_id: &str) -> CameraReadiness {
    record_capture::private_camera_owner::readiness(device_id)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn device_projection(device: &CameraDevice, readiness: CameraReadiness) -> Value {
    let (state, detail) = readiness_parts(&readiness);
    json!({
        "id": device.id,
        "label": device.label,
        "state": state,
        "detail": detail,
    })
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn readiness_error(device: &CameraDevice, readiness: &CameraReadiness) -> CutError {
    let (state, detail) = readiness_parts(readiness);
    CutError::new(
        error_codes::INVALID_ARGS,
        format!("{} is not ready for recording", device.label),
        detail,
    )
    .with_suggested_action(match state {
        "permission_denied" => "allow Camera access in system privacy settings, then retry",
        "busy" => "close the other app or recording that is using the camera, then retry",
        _ => "reconnect or reselect the camera, then retry",
    })
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn readiness_parts(readiness: &CameraReadiness) -> (&'static str, String) {
    match readiness {
        CameraReadiness::Missing { detail } => ("missing", detail.clone()),
        CameraReadiness::Enumerated { detail, .. } => ("enumerated", detail.clone()),
        CameraReadiness::PermissionRequired { detail, .. } => {
            ("permission_required", detail.clone())
        }
        CameraReadiness::PermissionDenied { detail, .. } => ("permission_denied", detail.clone()),
        CameraReadiness::Busy { detail } => ("busy", detail.clone()),
        CameraReadiness::NoFrame { detail, .. } => ("no_frame", detail.clone()),
        CameraReadiness::Ready { detail, .. } => ("ready", detail.clone()),
    }
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn record_error(error: record_core::RecordError) -> CutError {
    super::record_err(error)
}
