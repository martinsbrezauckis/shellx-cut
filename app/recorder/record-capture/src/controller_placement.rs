//! Capture-owned controller placement state.
//!
//! A native backend writes this only after it has admitted a concrete exclusion
//! or visibility action.  It deliberately contains no window handle, process
//! id, title, or source identity, so it is safe to project through the
//! short-lived `screen_record.status` response.

use std::sync::{Arc, Mutex};

/// The only public-safe conclusions a capture backend may make about recorder
/// controls while it records a display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureControllerPlacementState {
    Excluded,
    AutoHidden,
    Refused,
    Unavailable,
}

impl CaptureControllerPlacementState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Excluded => "excluded",
            Self::AutoHidden => "auto_hidden",
            Self::Refused => "refused",
            Self::Unavailable => "unavailable",
        }
    }
}

/// A bounded status plus an operator-safe reason. The reason must describe an
/// admitted capability boundary, never native implementation identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureControllerPlacementStatus {
    pub state: CaptureControllerPlacementState,
    pub reason: String,
}

/// Shared capture-owned state passed to the native backend with its admitted
/// configuration. It has no authority to hide or move a window on its own.
#[derive(Debug, Clone)]
pub struct CaptureControllerPlacement(Arc<Mutex<CaptureControllerPlacementStatus>>);

impl Default for CaptureControllerPlacement {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureControllerPlacement {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(CaptureControllerPlacementStatus {
            state: CaptureControllerPlacementState::Unavailable,
            reason: "Controller placement has not been admitted for this capture.".to_string(),
        })))
    }

    pub fn excluded(&self, reason: &'static str) {
        self.update(CaptureControllerPlacementState::Excluded, reason);
    }

    pub fn auto_hidden(&self, reason: &'static str) {
        self.update(CaptureControllerPlacementState::AutoHidden, reason);
    }

    pub fn refused(&self, reason: &'static str) {
        self.update(CaptureControllerPlacementState::Refused, reason);
    }

    pub fn unavailable(&self, reason: &'static str) {
        self.update(CaptureControllerPlacementState::Unavailable, reason);
    }

    pub fn status(&self) -> CaptureControllerPlacementStatus {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn update(&self, state: CaptureControllerPlacementState, reason: &'static str) {
        let mut status = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *status = CaptureControllerPlacementStatus {
            state,
            reason: reason.to_string(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureControllerPlacement, CaptureControllerPlacementState};

    #[test]
    fn placement_is_unavailable_until_a_native_backend_observes_an_outcome() {
        let placement = CaptureControllerPlacement::new();
        assert_eq!(
            placement.status().state,
            CaptureControllerPlacementState::Unavailable
        );

        placement.excluded("A native readback confirmed controller exclusion.");
        let status = placement.status();
        assert_eq!(status.state.as_str(), "excluded");
        assert_eq!(
            status.reason,
            "A native readback confirmed controller exclusion."
        );
    }
}
