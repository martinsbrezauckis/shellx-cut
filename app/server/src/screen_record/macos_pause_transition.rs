//! Durable acknowledgement helpers for the macOS pause owner.

use super::macos_pause_control::{PauseAction, PauseReceipt, PauseRequest};
use super::windows_pause_evidence::{
    CalibratedWindowsPauseEvidenceFactory, LocalWindowsPauseArtifactVerifier,
};
use super::windows_pause_session::{WindowsPauseLifecycle, WindowsPauseSession};
use record_core::{error_codes, RecordError};
use record_recovery::RecordingSessionJournalFile;

/// The terminal journal transition is the only public logical timestamp.
pub(super) fn logical_time<L>(
    session: &WindowsPauseSession<
        RecordingSessionJournalFile,
        CalibratedWindowsPauseEvidenceFactory<LocalWindowsPauseArtifactVerifier>,
        L,
    >,
) -> u64
where
    L: WindowsPauseLifecycle,
{
    session
        .journal()
        .transitions()
        .last()
        .map(|transition| transition.logical_offset_ms)
        .unwrap_or(0)
}

/// A live button changes only after its matching journal transition is sealed.
pub(super) fn complete_transition(
    request: Option<PauseRequest>,
    expected: PauseAction,
    receipt: PauseReceipt,
) -> record_core::Result<()> {
    let Some(request) = request else {
        return Err(capture_error(
            "macOS pause lifecycle changed without a public command",
        ));
    };
    if request.action() != expected {
        return Err(capture_error(
            "macOS pause lifecycle acknowledged the wrong public transition",
        ));
    }
    request.complete(Ok(receipt));
    Ok(())
}

fn capture_error(message: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        message,
        "the durable capture journal retained the incomplete native evidence",
    )
}
