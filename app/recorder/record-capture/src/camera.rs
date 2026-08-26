//! Camera backend contract and deterministic replay implementation.
//!
//! Native camera capture intentionally does not exist in this foundation slice.
//! The trait establishes the device/readiness/final-artifact boundary that every
//! future platform backend must meet, while `ReplayCamera` keeps timing and
//! artifact tests deterministic on all hosts.

use record_core::{error_codes, CameraArtifact, RecordError, Result};
use serde::{Deserialize, Serialize};

use crate::CaptureClock;

/// A host-local camera selection. `id` is opaque to callers and must never be
/// interpreted as a filesystem path or published as a portable device identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraDevice {
    pub id: String,
    pub label: String,
}

/// User-selected camera request bound to one screen-capture session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraRequest {
    pub capture_id: String,
    pub device_id: String,
}

/// Truthful readiness state for a selected camera. Enumeration is never `Ready`;
/// a real backend may return Ready only after a bounded first-frame probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CameraReadiness {
    Missing {
        detail: String,
    },
    Enumerated {
        device: CameraDevice,
        detail: String,
    },
    PermissionRequired {
        device: CameraDevice,
        detail: String,
    },
    PermissionDenied {
        device: CameraDevice,
        detail: String,
    },
    Ready {
        device: CameraDevice,
        detail: String,
    },
}

impl CameraReadiness {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// A camera implementation must expose readiness separately from capture and
/// return an already-validated `CameraArtifact@1`. The artifact's first/end
/// offsets are measured against the caller's shared screen `CaptureClock`.
pub trait CameraBackend {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness;
    fn capture(&self, request: &CameraRequest, clock: &CaptureClock) -> Result<CameraArtifact>;
}

/// Deterministic all-platform fixture backend. It returns a prebuilt immutable
/// artifact and performs no device access, permission prompt, or filesystem I/O.
#[derive(Debug, Clone)]
pub struct ReplayCamera {
    device: CameraDevice,
    artifact: CameraArtifact,
}

impl ReplayCamera {
    pub fn new(device: CameraDevice, artifact: CameraArtifact) -> Result<Self> {
        validate_request_part("device_id", &device.id)?;
        if device.label.trim().is_empty() {
            return Err(bad("camera device label must not be empty"));
        }
        artifact.validate()?;
        Ok(Self { device, artifact })
    }
}

impl CameraBackend for ReplayCamera {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness {
        if request.device_id == self.device.id {
            CameraReadiness::Ready {
                device: self.device.clone(),
                detail: "deterministic replay artifact is ready".into(),
            }
        } else {
            CameraReadiness::Missing {
                detail: "selected camera is not available in this replay fixture".into(),
            }
        }
    }

    fn capture(&self, request: &CameraRequest, _clock: &CaptureClock) -> Result<CameraArtifact> {
        validate_request_part("capture_id", &request.capture_id)?;
        validate_request_part("device_id", &request.device_id)?;
        if request.capture_id != self.artifact.capture_id {
            return Err(bad(
                "replay camera request capture_id does not match the immutable artifact",
            ));
        }
        if !self.readiness(request).is_ready() {
            return Err(RecordError::new(
                error_codes::UNIMPLEMENTED,
                "selected camera is not ready",
                "the replay fixture has no matching ready device",
            ));
        }
        self.artifact.validate()?;
        Ok(self.artifact.clone())
    }
}

fn validate_request_part(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(bad(&format!(
            "camera {label} must be 1-128 ASCII letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

fn bad(message: &str) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        message,
        "invalid camera backend contract",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_core::{
        CameraClockRange, CameraMediaFacts, CameraTerminalState, CAMERA_ARTIFACT_SCHEMA,
    };

    fn artifact() -> CameraArtifact {
        CameraArtifact {
            schema: CAMERA_ARTIFACT_SCHEMA.into(),
            capture_id: "cap_01".into(),
            artifact_id: "camera_01".into(),
            video: "camera/camera.mp4".into(),
            clock: CameraClockRange {
                first_frame_offset_ms: 250,
                end_frame_offset_ms: 1_250,
            },
            media: CameraMediaFacts {
                width: 640,
                height: 480,
                fps_num: 30,
                fps_den: 1,
                frame_count: 30,
                duration_ms: 1_000,
                sha256: "c".repeat(64),
            },
            terminal_state: CameraTerminalState::Complete,
        }
    }

    #[test]
    fn replay_camera_is_deterministic_and_bound_to_its_capture() {
        let replay = ReplayCamera::new(
            CameraDevice {
                id: "fixture_camera".into(),
                label: "Fixture camera".into(),
            },
            artifact(),
        )
        .unwrap();
        let request = CameraRequest {
            capture_id: "cap_01".into(),
            device_id: "fixture_camera".into(),
        };
        let clock = CaptureClock::new();
        clock.start();
        assert!(replay.readiness(&request).is_ready());
        assert_eq!(replay.capture(&request, &clock).unwrap(), artifact());

        let wrong_capture = CameraRequest {
            capture_id: "cap_02".into(),
            ..request
        };
        assert!(replay.capture(&wrong_capture, &clock).is_err());
    }
}
