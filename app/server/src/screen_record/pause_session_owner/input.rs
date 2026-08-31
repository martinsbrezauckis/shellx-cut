use super::super::run_seal_coordinator::SealedRunEvidence;
use super::types::{PauseSessionOwnerError, PauseSessionWorkerAdapter};
use super::PauseSessionOwner;
use record_recovery::RecordingInputSidecarPin;

impl<J, W> PauseSessionOwner<J, W>
where
    J: super::super::run_seal_coordinator::RecordingSessionJournalSink,
    W: PauseSessionWorkerAdapter,
{
    /// Keep input publication outside the generic pause state machine while
    /// retaining its one durable journal owner and failure semantics.
    pub(crate) fn pin_recording_input_sidecar(
        &mut self,
        evidence: &SealedRunEvidence,
        pin: RecordingInputSidecarPin,
    ) -> Result<(), PauseSessionOwnerError> {
        self.seal
            .pin_recording_input_sidecar(evidence, pin)
            .map_err(PauseSessionOwnerError::RunSeal)
    }
}
