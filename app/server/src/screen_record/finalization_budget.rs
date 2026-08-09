//! Bounded, capture-work-derived wait policy for `screen_record.stop`.
//!
//! Finalization can transcode sparse native checkpoints and flush independent
//! audio. A fixed 30-second poll falsely reports failure for a valid short 4K
//! recording. The wait remains finite, but scales from declared or observed
//! capture work rather than pretending every capture has the same cost.

/// Do not make an operator wait indefinitely when a native finalizer is stuck.
pub(crate) const MAX_FINALIZATION_WAIT_MS: u64 = 15 * 60 * 1_000;
const MIN_FINALIZATION_WAIT_MS: u64 = 45 * 1_000;
const FIXED_FINALIZATION_MS: u64 = 15 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FinalizationWaitBudget {
    /// Capture-clock work used to size the bounded finalization wait.
    pub(crate) capture_work_ms: u64,
    /// Explicit maximum elapsed time from the Stop signal to project publication.
    pub(crate) wait_ms: u64,
}

/// Derive a finite wait from the marker's upper bound or the durable journal's
/// observed elapsed time. For an open-ended capture, the journal time wins; a
/// malformed/clock-skewed journal safely falls back to the minimum budget.
///
/// Two capture spans cover native checkpoint transcode work plus a fixed grace
/// for audio-device teardown and atomic publication. The cap makes a truly stuck
/// native finalizer an explicit `sidecar` timeout rather than an endless request.
pub(crate) fn finalization_wait_budget(
    declared_duration_ms: Option<u64>,
    started_unix_ms: Option<u64>,
    now_unix_ms: u64,
) -> FinalizationWaitBudget {
    let observed_work_ms = started_unix_ms
        .filter(|started| *started <= now_unix_ms)
        .map(|started| now_unix_ms.saturating_sub(started));
    let capture_work_ms = match (declared_duration_ms, observed_work_ms) {
        (Some(declared), Some(observed)) => declared.max(observed),
        (Some(declared), None) => declared,
        (None, Some(observed)) => observed,
        (None, None) => 0,
    };
    let wait_ms = capture_work_ms
        .saturating_mul(2)
        .saturating_add(FIXED_FINALIZATION_MS)
        .clamp(MIN_FINALIZATION_WAIT_MS, MAX_FINALIZATION_WAIT_MS);
    FinalizationWaitBudget {
        capture_work_ms,
        wait_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::{finalization_wait_budget, MAX_FINALIZATION_WAIT_MS};

    #[test]
    fn open_ended_capture_uses_observed_journal_work() {
        let budget = finalization_wait_budget(None, Some(1_000), 27_728);

        assert_eq!(budget.capture_work_ms, 26_728);
        assert_eq!(budget.wait_ms, 68_456);
    }

    #[test]
    fn declared_upper_bound_remains_conservative_after_early_stop() {
        let budget = finalization_wait_budget(Some(60_000), Some(1_000), 6_000);

        assert_eq!(budget.capture_work_ms, 60_000);
        assert_eq!(budget.wait_ms, 135_000);
    }

    #[test]
    fn invalid_or_future_journal_time_uses_explicit_minimum() {
        let budget = finalization_wait_budget(None, Some(10_000), 1_000);

        assert_eq!(budget.capture_work_ms, 0);
        assert_eq!(budget.wait_ms, 45_000);
    }

    #[test]
    fn cap_keeps_a_stuck_finalizer_bounded() {
        let budget = finalization_wait_budget(Some(u64::MAX), None, 0);

        assert_eq!(budget.wait_ms, MAX_FINALIZATION_WAIT_MS);
    }
}
