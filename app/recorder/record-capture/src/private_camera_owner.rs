//! Cross-platform explicit-use camera owner for the screen-recording server.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use record_core::{CameraArtifact, CameraTerminalState, Result};

use crate::camera_runtime::{CameraRuntime, CameraStopResult, CameraUseOutcome};
use crate::{CameraDevice, CameraReadiness, CameraRequest, CameraUseIntent, CaptureClock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateCameraUse {
    Started,
    Refused(CameraReadiness),
    Unavailable,
}

pub struct PrivateCameraOwner {
    capture_id: String,
    runtime: CameraRuntime,
}

impl PrivateCameraOwner {
    /// Reserve under an already-created capture directory. No enumeration,
    /// permission prompt, or device open occurs during construction.
    pub fn reserve(capture_directory: &Path, capture_id: &str) -> Result<Self> {
        crate::camera::validate_request_part("capture_id", capture_id)?;
        Ok(Self {
            capture_id: capture_id.to_owned(),
            runtime: platform_runtime(capture_directory)?,
        })
    }

    pub fn readiness(&self, opaque_device_id: &str) -> CameraReadiness {
        self.runtime.readiness(&self.request(opaque_device_id))
    }

    /// The sole permission-capable gateway. The screen clock and Stop signal
    /// are shared with the owning capture; a successful result includes a real
    /// first native frame rather than inferred enumeration readiness.
    pub fn use_camera(
        &mut self,
        opaque_device_id: &str,
        screen_clock: &CaptureClock,
        screen_stop: &AtomicBool,
    ) -> Result<PrivateCameraUse> {
        let intent = CameraUseIntent::from_explicit_user_action(self.request(opaque_device_id))?;
        match self.runtime.use_camera(intent, screen_clock, screen_stop)? {
            CameraUseOutcome::Started => Ok(PrivateCameraUse::Started),
            CameraUseOutcome::Unavailable { .. } => Ok(PrivateCameraUse::Unavailable),
            CameraUseOutcome::Refused(readiness) => Ok(PrivateCameraUse::Refused(readiness)),
        }
    }

    pub fn stop(&mut self, terminal: CameraTerminalState) -> Result<Option<CameraArtifact>> {
        match self
            .runtime
            .stop_for_screen_owner(&self.capture_id, terminal)?
        {
            CameraStopResult::NoMedia => Ok(None),
            CameraStopResult::Sealed(evidence) => Ok(Some(evidence.artifact().clone())),
        }
    }

    fn request(&self, opaque_device_id: &str) -> CameraRequest {
        CameraRequest {
            capture_id: self.capture_id.clone(),
            device_id: opaque_device_id.into(),
        }
    }
}

pub fn devices() -> Result<Vec<CameraDevice>> {
    platform_devices()
}

pub fn readiness(opaque_device_id: &str) -> CameraReadiness {
    platform_readiness(opaque_device_id)
}

fn platform_runtime(capture_directory: &Path) -> Result<CameraRuntime> {
    #[cfg(windows)]
    {
        return crate::windows_camera::private_runtime(capture_directory);
    }
    #[cfg(target_os = "macos")]
    {
        return crate::macos_camera::private_runtime(capture_directory);
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return crate::linux_camera::private_runtime(capture_directory);
    }
    #[allow(unreachable_code)]
    Ok(CameraRuntime::unavailable_for_build())
}

fn platform_devices() -> Result<Vec<CameraDevice>> {
    #[cfg(windows)]
    {
        return crate::windows_camera::private_devices();
    }
    #[cfg(target_os = "macos")]
    {
        return crate::macos_camera::private_devices();
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return crate::linux_camera::private_devices();
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

fn platform_readiness(opaque_device_id: &str) -> CameraReadiness {
    #[cfg(windows)]
    {
        return crate::windows_camera::private_readiness(opaque_device_id);
    }
    #[cfg(target_os = "macos")]
    {
        return crate::macos_camera::private_readiness(opaque_device_id);
    }
    #[cfg(all(target_os = "linux", feature = "capture-linux"))]
    {
        return crate::linux_camera::private_readiness(opaque_device_id);
    }
    #[allow(unreachable_code)]
    CameraReadiness::Missing {
        detail: "Camera capture is unavailable in this build.".into(),
    }
}
