//! Capture-owned selected-source lifecycle evidence.
//!
//! This is intentionally narrower than general capture failure: a backend can
//! report loss only after it has armed one concrete source-close callback, and
//! a Cut-initiated close wins the same atomic transition before status sees it.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::CaptureReadiness;

const UNAVAILABLE: u8 = 0;
const OBSERVING: u8 = 1;
const EXPECTED_SEGMENT_CLOSE: u8 = 2;
const EXPECTED_TERMINAL_CLOSE: u8 = 3;
const SOURCE_LOST: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSourceLifecycleState {
    Unavailable,
    Observing,
    SourceLost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSourceLifecycleStatus {
    pub state: CaptureSourceLifecycleState,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CaptureSourceLifecycle {
    transition: Arc<AtomicU8>,
    reason: Arc<Mutex<String>>,
    readiness: Option<CaptureReadiness>,
}

impl Default for CaptureSourceLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureSourceLifecycle {
    pub fn new() -> Self {
        Self::with_optional_readiness(None)
    }

    /// Binds selected-source loss to the same readiness admission that is
    /// exposed by `screen_record.status`.
    pub fn with_readiness(readiness: CaptureReadiness) -> Self {
        Self::with_optional_readiness(Some(readiness))
    }

    fn with_optional_readiness(readiness: Option<CaptureReadiness>) -> Self {
        Self {
            transition: Arc::new(AtomicU8::new(UNAVAILABLE)),
            reason: Arc::new(Mutex::new(
                "The admitted native source does not expose a selected-source close signal."
                    .to_string(),
            )),
            readiness,
        }
    }

    /// Arm the first native callback for one exact selected source. It can run
    /// once only: a terminal Cut close cannot be rearmed as a new capture.
    pub fn arm_initial_selected_source(&self, reason: &'static str) {
        self.arm(UNAVAILABLE, reason);
    }

    /// Arm a new physical segment after this handle itself recorded an expected
    /// checkpoint rotation. It cannot revive an expected terminal close or a
    /// prior source-loss conclusion.
    pub fn arm_next_owned_segment(&self, reason: &'static str) {
        self.arm(EXPECTED_SEGMENT_CLOSE, reason);
    }

    /// Set immediately before Cut rotates one physical native checkpoint.
    pub fn expect_segment_close(&self) {
        let _ = self.transition.compare_exchange(
            OBSERVING,
            EXPECTED_SEGMENT_CLOSE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Stop and all server-owned terminal paths use this terminal transition.
    /// It cannot overwrite a source loss that already won.
    pub fn expect_terminal_close(&self) {
        loop {
            let current = self.transition.load(Ordering::Acquire);
            match current {
                // A backend/source that never armed a selected-source signal
                // remains unavailable after Stop. Terminalizing the capture
                // must not invent lifecycle coverage that was never present.
                UNAVAILABLE | SOURCE_LOST | EXPECTED_TERMINAL_CLOSE => return,
                OBSERVING | EXPECTED_SEGMENT_CLOSE => {
                    if self
                        .transition
                        .compare_exchange(
                            current,
                            EXPECTED_TERMINAL_CLOSE,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return;
                    }
                }
                _ => return,
            }
        }
    }

    /// Returns true exactly once when an armed source callback wins before any
    /// Cut-owned close. It simultaneously revokes ready admission, so callers
    /// cannot observe this source loss as an active capture.
    pub fn selected_source_closed(&self, reason: &'static str) -> bool {
        // Hold the projected reason lock across the transition so a status read
        // cannot observe source_lost with the prior arm reason.
        let mut projected_reason = self.lock_reason();
        let Ok(_) = self.transition.compare_exchange(
            OBSERVING,
            SOURCE_LOST,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) else {
            return false;
        };
        if let Some(readiness) = self.readiness.as_ref() {
            readiness.mark_terminal();
        }
        *projected_reason = reason.to_string();
        true
    }

    pub fn status(&self) -> CaptureSourceLifecycleStatus {
        let state = match self.transition.load(Ordering::Acquire) {
            UNAVAILABLE => CaptureSourceLifecycleState::Unavailable,
            SOURCE_LOST => CaptureSourceLifecycleState::SourceLost,
            OBSERVING | EXPECTED_SEGMENT_CLOSE | EXPECTED_TERMINAL_CLOSE => {
                CaptureSourceLifecycleState::Observing
            }
            _ => CaptureSourceLifecycleState::Unavailable,
        };
        CaptureSourceLifecycleStatus {
            state,
            reason: self.lock_reason().clone(),
        }
    }

    fn arm(&self, expected: u8, reason: &'static str) {
        // Keep the reason lock across the arm transition too: status must not
        // publish Observing together with the prior unavailable explanation.
        let mut projected_reason = self.lock_reason();
        if self
            .transition
            .compare_exchange(expected, OBSERVING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            *projected_reason = reason.to_string();
        }
    }

    fn lock_reason(&self) -> std::sync::MutexGuard<'_, String> {
        self.reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::{CaptureSourceLifecycle, CaptureSourceLifecycleState};
    use crate::CaptureReadiness;

    #[test]
    fn only_an_armed_unexpected_close_becomes_source_loss() {
        let lifecycle = CaptureSourceLifecycle::new();
        assert!(!lifecycle.selected_source_closed("unarmed"));
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        assert!(lifecycle.selected_source_closed("selected window closed."));
        assert_eq!(
            lifecycle.status().state,
            CaptureSourceLifecycleState::SourceLost
        );
    }

    #[test]
    fn terminal_stop_does_not_invent_monitoring_for_an_unarmed_source() {
        let lifecycle = CaptureSourceLifecycle::new();
        lifecycle.expect_terminal_close();
        assert_eq!(
            lifecycle.status().state,
            CaptureSourceLifecycleState::Unavailable
        );
        assert!(!lifecycle.selected_source_closed("unarmed late callback"));
    }

    #[test]
    fn expected_terminal_close_wins_against_a_late_native_callback() {
        let lifecycle = Arc::new(CaptureSourceLifecycle::new());
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        let expected_ready = Arc::new(Barrier::new(2));
        let allow_callback = Arc::new(Barrier::new(2));
        let close_lifecycle = lifecycle.clone();
        let close_ready = expected_ready.clone();
        let close_callback = allow_callback.clone();
        let callback = thread::spawn(move || {
            close_ready.wait();
            close_callback.wait();
            close_lifecycle.selected_source_closed("selected window closed.")
        });

        lifecycle.expect_terminal_close();
        expected_ready.wait();
        allow_callback.wait();
        assert!(!callback.join().unwrap());
        assert_eq!(
            lifecycle.status().state,
            CaptureSourceLifecycleState::Observing
        );
    }

    #[test]
    fn source_callback_wins_before_a_later_terminal_close() {
        let lifecycle = Arc::new(CaptureSourceLifecycle::new());
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        let callback_ready = Arc::new(Barrier::new(2));
        let allow_terminal = Arc::new(Barrier::new(2));
        let close_lifecycle = lifecycle.clone();
        let ready = callback_ready.clone();
        let terminal = allow_terminal.clone();
        let callback = thread::spawn(move || {
            ready.wait();
            let won = close_lifecycle.selected_source_closed("selected window closed.");
            terminal.wait();
            won
        });

        callback_ready.wait();
        allow_terminal.wait();
        lifecycle.expect_terminal_close();
        assert!(callback.join().unwrap());
        assert_eq!(
            lifecycle.status().state,
            CaptureSourceLifecycleState::SourceLost
        );
    }

    #[test]
    fn expected_segment_close_blocks_its_own_native_callback() {
        let lifecycle = CaptureSourceLifecycle::new();
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        lifecycle.expect_segment_close();

        assert!(!lifecycle.selected_source_closed("selected window closed."));
        assert_eq!(
            lifecycle.status().state,
            CaptureSourceLifecycleState::Observing
        );
    }

    #[test]
    fn terminal_stop_cannot_rearm_a_source_for_a_later_callback() {
        let lifecycle = CaptureSourceLifecycle::new();
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        lifecycle.expect_terminal_close();
        lifecycle.arm_next_owned_segment("this must not revive a stopped capture.");

        assert!(!lifecycle.selected_source_closed("selected window closed."));
        assert_eq!(
            lifecycle.status().state,
            CaptureSourceLifecycleState::Observing
        );
    }

    #[test]
    fn source_loss_revokes_an_already_ready_capture() {
        let readiness = CaptureReadiness::default();
        readiness.mark_first_screen_frame_delivered();
        let lifecycle = CaptureSourceLifecycle::with_readiness(readiness.clone());
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        assert!(lifecycle.selected_source_closed("selected window closed."));
        assert!(!readiness.status().ready);
        assert!(readiness.status().terminal);
    }

    #[test]
    fn only_a_new_owned_segment_can_rearm_after_rotation() {
        let lifecycle = CaptureSourceLifecycle::new();
        lifecycle.arm_initial_selected_source("exact native window close is observed.");
        lifecycle.expect_segment_close();
        lifecycle.arm_next_owned_segment("new exact native window segment is observed.");
        assert!(lifecycle.selected_source_closed("selected window closed."));
    }
}
