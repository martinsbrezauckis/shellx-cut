//! What a failed recording tells the person and the agent.

use record_capture::{CaptureReadinessState, CaptureReadinessStatus};
use record_core::RecordError;
use record_recovery::{RecoveryReceipt, RecoveryState};

pub(crate) const NOTHING_RECORDED: &str = "nothing was recorded";

/// A Stop that arrives before the first screen frame is not a capture fault:
/// nothing was recorded yet. Native layers report that moment differently
/// (Media Foundation: "no samples were processed by the sink"; Linux and
/// macOS: a missing frame or an empty output), so it is recognised here from
/// the session's own facts, the same way on every platform.
pub(crate) fn explain(
    error: RecordError,
    stop_requested: bool,
    readiness: CaptureReadinessStatus,
    kept: Option<&RecoveryReceipt>,
) -> RecordError {
    let kept_segments = kept.map_or(0, |receipt| receipt.recovered_segments);
    // Media Foundation's MF_E_SINK_NO_SAMPLES_PROCESSED: a frame may have
    // reached the capture path but none reached the file before Stop.
    let no_samples_written = error.cause.contains("0xC00D4A44");
    if already_says_nothing_was_captured(&error) {
        return error;
    }
    // Stop before the first frame: a frame or a short checkpoint that the
    // native layer wrote after the Stop request is not a take the person saw
    // starting (the app still showed "Starting recording"), so it is never
    // offered. This used to end in a Recording Scenes receipt error or a
    // one-frame take on macOS.
    let stopped_before_first_frame =
        readiness.state == CaptureReadinessState::TerminalBeforeFirstScreenFrame;
    if stop_requested && (stopped_before_first_frame || (no_samples_written && kept_segments == 0))
    {
        return RecordError::new(
            record_core::error_codes::CAPTURE,
            NOTHING_RECORDED,
            format!(
                "Stop came before the first screen frame was captured, so there is no take to save (recorder detail: {}: {})",
                error.message, error.cause
            ),
        )
        .with_action("start the recording again and wait until the timer is running before you stop");
    }
    error
}

/// Backends that already name the moment ("stopped before the first frame",
/// "stopped before native start") keep their own message and platform action.
fn already_says_nothing_was_captured(error: &RecordError) -> bool {
    error.message.contains("before the first frame")
        || error.message.contains("before native start")
}

/// Whether this failure means no take exists (nothing to recover later).
pub(crate) fn nothing_recorded(error: &RecordError) -> bool {
    error.message == NOTHING_RECORDED || already_says_nothing_was_captured(error)
}

pub(crate) const KEPT_TAKE: &str =
    "the recording stopped with an error, and the part recorded before it was saved";
pub(crate) const RECOVER_KEPT_TAKE: &str = "add the saved part to the timeline with Recover recordings in Settings > Health & recovery (agents: screen_record.recover with this capture_id)";

/// Add what was kept to a failure that happened after recording had started.
/// The person reads what happened and where the saved part is; the native
/// layer's own advice (for example "ensure this is a Windows desktop session")
/// is about starting a capture and does not fit a take that already exists.
pub(crate) fn with_kept_take(error: RecordError, receipt: Option<&RecoveryReceipt>) -> RecordError {
    if nothing_recorded(&error) {
        return error;
    }
    let kept = receipt.is_some_and(|receipt| {
        matches!(
            receipt.state,
            RecoveryState::Recovered | RecoveryState::Quarantined
        ) && receipt.source.is_some()
    });
    if !kept {
        return error;
    }
    RecordError::new(
        &error.code,
        KEPT_TAKE,
        format!(
            "recorder detail: {}: {}; the part recorded before this failure was saved and can be added to the timeline with screen_record.recover",
            error.message, error.cause
        ),
    )
    .with_action(RECOVER_KEPT_TAKE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: CaptureReadinessState) -> CaptureReadinessStatus {
        CaptureReadinessStatus {
            ready: state == CaptureReadinessState::Ready,
            terminal: matches!(
                state,
                CaptureReadinessState::TerminalBeforeFirstScreenFrame
                    | CaptureReadinessState::TerminalAfterFirstScreenFrame
            ),
            state,
            first_screen_frame_unix_ms: None,
        }
    }

    fn sink_error() -> RecordError {
        RecordError::new(
            record_core::error_codes::CAPTURE,
            "finalize WGC checkpoint",
            "The operation failed because no samples were processed by the sink. (0xC00D4A44)",
        )
        .with_action("ensure this is a Windows desktop session with Graphics Capture available")
    }

    #[test]
    fn stop_before_the_first_frame_says_nothing_was_recorded() {
        let error = explain(
            sink_error(),
            true,
            status(CaptureReadinessState::TerminalBeforeFirstScreenFrame),
            None,
        );
        assert_eq!(error.message, NOTHING_RECORDED);
        assert!(error.cause.contains("before the first screen frame"));
        assert_eq!(
            error.suggested_action.as_deref(),
            Some("start the recording again and wait until the timer is running before you stop")
        );
    }

    #[test]
    fn stop_with_no_samples_written_after_a_first_frame_is_also_nothing_recorded() {
        let error = explain(
            sink_error(),
            true,
            status(CaptureReadinessState::TerminalAfterFirstScreenFrame),
            None,
        );
        assert_eq!(error.message, NOTHING_RECORDED);
    }

    /// macOS early Stop (0.2 to 0.3 s): ScreenCaptureKit wrote one to five
    /// frames after the Stop request, a checkpoint was kept, and the Recording
    /// Scenes owner refused because the scene timer never started.
    #[test]
    fn frames_written_after_an_early_stop_are_still_nothing_recorded() {
        let receipt = RecoveryReceipt {
            state: RecoveryState::Recovered,
            recovered_segments: 1,
            lost_tail_ms: None,
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: None,
            audio_first_packet_offset_ms: None,
            source: Some("recovered.mp4".into()),
            note: String::new(),
        };
        let scenes = RecordError::new(
            record_core::error_codes::IO,
            "public Recording Scenes receipt could not be published",
            "seal public Recording Scenes projection: a public Recording Scenes capture reached terminalization without a durable projection",
        );
        let error = explain(
            scenes,
            true,
            status(CaptureReadinessState::TerminalBeforeFirstScreenFrame),
            Some(&receipt),
        );
        assert_eq!(error.message, NOTHING_RECORDED);
        assert!(nothing_recorded(&error));
        let error = with_kept_take(error, Some(&receipt));
        assert!(!error.cause.contains("screen_record.recover"));
    }

    #[test]
    fn a_real_failure_after_frames_keeps_its_own_message() {
        let other = || {
            RecordError::new(
                record_core::error_codes::CAPTURE,
                "finalize WGC checkpoint",
                "The device was removed. (0x887A0005)",
            )
        };
        let error = explain(
            other(),
            true,
            status(CaptureReadinessState::TerminalAfterFirstScreenFrame),
            None,
        );
        assert_eq!(error.message, "finalize WGC checkpoint");
        let error = explain(
            sink_error(),
            false,
            status(CaptureReadinessState::TerminalBeforeFirstScreenFrame),
            None,
        );
        assert_eq!(error.message, "finalize WGC checkpoint");
    }

    #[test]
    fn a_backend_that_already_names_the_early_stop_keeps_its_action() {
        let portal = RecordError::new(
            record_core::error_codes::CAPTURE,
            "screen capture stopped before the first frame",
            "portal",
        )
        .with_action("dismiss the portal picker if it remains visible, then retry recording");
        let error = explain(
            portal,
            true,
            status(CaptureReadinessState::TerminalBeforeFirstScreenFrame),
            None,
        );
        assert_eq!(
            error.message,
            "screen capture stopped before the first frame"
        );
        assert!(nothing_recorded(&error));
    }

    #[test]
    fn a_failure_with_saved_checkpoints_says_the_take_was_kept() {
        let receipt = RecoveryReceipt {
            state: RecoveryState::Recovered,
            recovered_segments: 2,
            lost_tail_ms: None,
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: None,
            audio_first_packet_offset_ms: None,
            source: Some("recovered.mp4".into()),
            note: String::new(),
        };
        let error = explain(
            sink_error(),
            true,
            status(CaptureReadinessState::TerminalAfterFirstScreenFrame),
            Some(&receipt),
        );
        assert_eq!(error.message, "finalize WGC checkpoint");
        let error = with_kept_take(error, Some(&receipt));
        assert_eq!(error.message, KEPT_TAKE);
        assert!(error.cause.contains("finalize WGC checkpoint"));
        assert!(error.cause.contains("screen_record.recover"));
        assert_eq!(error.suggested_action.as_deref(), Some(RECOVER_KEPT_TAKE));
        let error = with_kept_take(sink_error(), None);
        assert!(!error.cause.contains("screen_record.recover"));
    }
}
