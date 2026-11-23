//! Fixed build/system facts for system.doctor. Never enumerate or open devices.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemAvailability {
    pub pause: bool,
    pub camera: bool,
    pub live_timer: bool,
    pub microphone_choice: bool,
    pub source_preview: bool,
    pub managed_motion: bool,
}

pub(crate) fn fixed() -> SystemAvailability {
    SystemAvailability {
        pause: super::recording_controls::capability()["supported"] == true,
        camera: super::camera_public::supported(),
        live_timer: super::recording_scenes::capability()["supported"] == true,
        microphone_choice: super::microphone::choice_supported(),
        // This native capability function is pure: only compile-time cfg branches.
        source_preview: matches!(
            record_capture::source_preview_native::capability(),
            record_capture::source_preview_native::SourcePreviewCapability::Available { .. }
        ),
        managed_motion: crate::motion_discovery::managed_distribution_available(),
    }
}
