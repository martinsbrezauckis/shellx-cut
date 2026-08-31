//! Narrow server-only owner for the reviewed Windows Camera Capture Engine.
//!
//! This deliberately exposes neither enumeration nor device labels.  A Cut
//! server owner receives one already-reserved capture directory and may make
//! one explicit-use request for an opaque device id.  `Started` means that the
//! native adapter has already observed its bounded first frame; it is never a
//! promise inferred from enumeration or permission state.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use record_core::{CameraArtifact, CameraTerminalState, Result};

use crate::camera_runtime::{CameraRuntime, CameraStopResult, CameraUseOutcome};
use crate::{CameraReadiness, CameraRequest, CameraUseIntent, CaptureClock};

/// Label-free passive readiness suitable for the private server boundary.
/// Device ids and labels remain inside the owner and are never projected to a
/// verb, Doctor card, receipt, or UI model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateWindowsCameraReadiness {
    Missing,
    AwaitingExplicitUse,
    PermissionDenied,
    Busy,
    NoFrame,
    Ready,
}

/// Result of the only permission-capable operation in this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateWindowsCameraUse {
    Started,
    Refused(PrivateWindowsCameraReadiness),
    Unavailable,
}

/// A single camera sidecar bound to one screen recording reservation.
///
/// The server must retain this owner from its exact project/revision/capture
/// admission through `stop`.  `stop` returns a sealed `CameraArtifact@1` only
/// after native Close, immutable measurement, full-file hashing, and anchored
/// no-replace publication have all succeeded. `None` is a truthful no-media
/// terminal and must never be turned into a placeholder camera asset.
pub struct PrivateWindowsCameraOwner {
    capture_id: String,
    runtime: CameraRuntime,
}

impl PrivateWindowsCameraOwner {
    /// Construction is permission-neutral. The supplied directory is owned by
    /// the server's capture reservation, not selected from a caller path.
    pub fn reserve(capture_directory: &Path, capture_id: &str) -> Result<Self> {
        crate::camera::validate_request_part("capture_id", capture_id)?;
        Ok(Self {
            capture_id: capture_id.to_owned(),
            runtime: crate::windows_camera::private_runtime(capture_directory)?,
        })
    }

    /// Passive state only. It neither opens a camera nor requests permission.
    pub fn readiness(&self, opaque_device_id: &str) -> PrivateWindowsCameraReadiness {
        self.runtime
            .readiness(&self.request(opaque_device_id))
            .into()
    }

    /// The sole explicit-use gateway. A successful return proves the reviewed
    /// native adapter observed a first delivered frame before this sidecar was
    /// admitted. A later Stop returns the adapter to this owner, permitting a
    /// distinct explicit retry after denied/no-frame/device-loss outcomes.
    pub fn use_camera(
        &mut self,
        opaque_device_id: &str,
        screen_clock: &CaptureClock,
        screen_stop: &AtomicBool,
    ) -> Result<PrivateWindowsCameraUse> {
        let intent = CameraUseIntent::from_explicit_user_action(self.request(opaque_device_id))?;
        match self.runtime.use_camera(intent, screen_clock, screen_stop)? {
            CameraUseOutcome::Started => Ok(PrivateWindowsCameraUse::Started),
            CameraUseOutcome::Unavailable { .. } => Ok(PrivateWindowsCameraUse::Unavailable),
            CameraUseOutcome::Refused(readiness) => {
                Ok(PrivateWindowsCameraUse::Refused(readiness.into()))
            }
        }
    }

    /// Stop only for this owner. The runtime preserves a DeviceLost terminal
    /// state when Media Foundation reports it, even when the requested screen
    /// terminal state was Complete. The retained adapter makes a future
    /// explicit retry possible; this method never retries native Stop itself.
    pub fn stop(&mut self, terminal: CameraTerminalState) -> Result<Option<CameraArtifact>> {
        match self
            .runtime
            .stop_for_screen_owner(&self.capture_id, terminal)?
        {
            CameraStopResult::NoMedia => Ok(None),
            CameraStopResult::Sealed(evidence) => Ok(Some(evidence.artifact().clone())),
        }
    }

    /// Bounded owner-unwind terminalization. This uses the same native Stop
    /// path (including its bounded RecordStopped wait, Close, and join/release
    /// ordering) but records that the screen owner was cancelled. Callers must
    /// retain their exclusive reservation until this method has returned.
    pub fn abort_for_unwind(&mut self) -> Result<Option<CameraArtifact>> {
        self.stop(CameraTerminalState::Cancelled)
    }

    fn request(&self, opaque_device_id: &str) -> CameraRequest {
        CameraRequest {
            capture_id: self.capture_id.clone(),
            device_id: opaque_device_id.to_owned(),
        }
    }
}

impl From<CameraReadiness> for PrivateWindowsCameraReadiness {
    fn from(readiness: CameraReadiness) -> Self {
        match readiness {
            CameraReadiness::Missing { .. } | CameraReadiness::Enumerated { .. } => Self::Missing,
            CameraReadiness::PermissionRequired { .. } => Self::AwaitingExplicitUse,
            CameraReadiness::PermissionDenied { .. } => Self::PermissionDenied,
            CameraReadiness::Busy { .. } => Self::Busy,
            CameraReadiness::NoFrame { .. } => Self::NoFrame,
            CameraReadiness::Ready { .. } => Self::Ready,
        }
    }
}
