//! Per-capture input-hook startup evidence, independent of video admission.
//!
//! Registration acknowledges native startup only. It does not prove delivery,
//! continued hook health, or pointer coordinate accuracy.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputHookStartupState {
    #[default]
    Unobserved,
    Registered,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputHookBackend {
    RdevinWindows,
    RdevinMacos,
    RdevinX11,
    WaylandEvdev,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputHookStartupReason {
    StartupFailed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputHookStartup {
    pub state: InputHookStartupState,
    pub backend: InputHookBackend,
    pub reason: Option<InputHookStartupReason>,
    pub capture_keys: bool,
}

impl InputHookStartup {
    pub(crate) fn rdevin(registered: bool, capture_keys: bool) -> Self {
        let backend = if cfg!(windows) {
            InputHookBackend::RdevinWindows
        } else if cfg!(target_os = "macos") {
            InputHookBackend::RdevinMacos
        } else if cfg!(target_os = "linux") {
            InputHookBackend::RdevinX11
        } else {
            InputHookBackend::Unknown
        };
        Self {
            state: if registered {
                InputHookStartupState::Registered
            } else {
                InputHookStartupState::Unavailable
            },
            backend,
            reason: if registered {
                None
            } else {
                Some(InputHookStartupReason::StartupFailed)
            },
            capture_keys,
        }
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn wayland_evdev(capture_keys: bool) -> Self {
        Self {
            backend: InputHookBackend::WaylandEvdev,
            capture_keys,
            ..Self::default()
        }
    }
}

/// One startup publication per capture; clones share only this capture's evidence.
#[derive(Debug, Clone, Default)]
pub(crate) struct InputHookStartupHandle(Arc<Mutex<Option<InputHookStartup>>>);

impl InputHookStartupHandle {
    pub(crate) fn publish(&self, observation: InputHookStartup) {
        // A default/unknown projection is absence of evidence, not publication.
        if observation.backend == InputHookBackend::Unknown {
            return;
        }
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            *slot = Some(observation);
        }
    }

    pub(crate) fn get(&self) -> InputHookStartup {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CaptureReadiness;

    #[test]
    fn startup_evidence_is_shared_once_and_independent_of_screen_admission() {
        let capture = CaptureReadiness::default();
        let clone = capture.clone();
        let other_capture = CaptureReadiness::default();
        capture.publish_input_hook_startup(InputHookStartup::default());
        capture.publish_input_hook_startup(InputHookStartup {
            capture_keys: true,
            ..InputHookStartup::default()
        });
        let unavailable = InputHookStartup::rdevin(false, true);
        capture.publish_input_hook_startup(unavailable);
        clone.publish_input_hook_startup(InputHookStartup::rdevin(true, false));
        assert_eq!(clone.input_hook_startup(), unavailable);
        assert_eq!(
            other_capture.input_hook_startup(),
            InputHookStartup::default()
        );
        assert!(!capture.status().ready);
        capture.mark_first_screen_frame_delivered();
        assert!(
            capture.status().ready,
            "optional hook failure cannot block delivered video"
        );
        capture.mark_terminal();
        assert_eq!(capture.input_hook_startup(), unavailable);
        assert!(!capture.status().ready);
    }

    #[test]
    fn wayland_is_explicitly_unobserved_and_cannot_be_replaced_by_rdevin_claim() {
        let capture = CaptureReadiness::default();
        let wayland = InputHookStartup::wayland_evdev(true);
        capture.publish_input_hook_startup(wayland);
        capture.publish_input_hook_startup(InputHookStartup::rdevin(true, true));
        assert_eq!(capture.input_hook_startup(), wayland);
        assert_eq!(wayland.state, InputHookStartupState::Unobserved);
        assert_eq!(wayland.reason, None);
    }

    #[test]
    fn public_serialization_is_structured_and_legacy_absence_is_unobserved() {
        let value = serde_json::to_value(InputHookStartup::rdevin(false, true)).unwrap();
        assert_eq!(value["state"], "unavailable");
        assert_eq!(value["reason"], "startup_failed");
        assert_eq!(value["capture_keys"], true);
        assert_eq!(
            serde_json::from_str::<InputHookStartup>("{}").unwrap(),
            InputHookStartup::default()
        );
    }
}
