//! Marks the microphone test and the system-audio test as running, so a
//! recording start requested while one of them holds the native capture is
//! refused with a plain reason instead of racing it. The Record panel refuses
//! the same start on its own; this covers every other route (an agent calling
//! the start verb included).

use std::sync::atomic::{AtomicUsize, Ordering};

use cut_core::{error_codes, CutError};

static RUNNING_TESTS: AtomicUsize = AtomicUsize::new(0);

/// Held for the whole native work of one audio test; dropping it ends the test.
pub(crate) struct AudioTestGuard(());

impl AudioTestGuard {
    pub(crate) fn enter() -> Self {
        RUNNING_TESTS.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for AudioTestGuard {
    fn drop(&mut self) {
        RUNNING_TESTS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Refuse a recording start while an audio test is running.
pub(crate) fn refuse_start_during_audio_test() -> Result<(), CutError> {
    if RUNNING_TESTS.load(Ordering::SeqCst) == 0 {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "An audio test is still running, so the recording cannot start yet.",
        "the microphone test or the system-audio test holds the audio capture",
    )
    .with_suggested_action(
        "Wait a few seconds for the audio test to finish, then start the recording again.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test owns the shared counter so parallel tests cannot disturb it.
    #[test]
    fn a_start_is_refused_while_a_test_runs_and_allowed_after() {
        assert!(refuse_start_during_audio_test().is_ok());
        let first = AudioTestGuard::enter();
        let second = AudioTestGuard::enter();
        let error = refuse_start_during_audio_test().unwrap_err();
        assert_eq!(error.code, error_codes::CONFLICT);
        assert!(
            error.message.contains("audio test is still running"),
            "{}",
            error.message
        );
        let action = error.suggested_action.expect("a next step");
        assert!(action.starts_with("Wait a few seconds"), "{action}");
        drop(first);
        assert!(
            refuse_start_during_audio_test().is_err(),
            "one test still runs"
        );
        drop(second);
        assert!(refuse_start_during_audio_test().is_ok());
    }
}
