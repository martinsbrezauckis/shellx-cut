//! Narrow server contract between native event translation and calibrated facts.

use super::run_seal_coordinator::{SealedRunEvidence, SessionTimeOrigin};
use super::windows_pause_adapter::WindowsPauseAdapterError;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotStarted, WindowsSealedAudioRun, WindowsSealedScreenRun,
};

pub(crate) trait WindowsPauseEvidenceFactory {
    fn stage_started(
        &mut self,
        server_generation: Option<u64>,
        started: &WindowsPausePilotStarted,
    ) -> Result<(), WindowsPauseAdapterError>;

    fn discard_staged(&mut self) {}

    fn verify_discarded_stop(
        &mut self,
        _run: &WindowsSealedScreenRun,
        _post_close_observed_at: std::time::Instant,
    ) -> Result<(), WindowsPauseAdapterError> {
        Err(WindowsPauseAdapterError::EvidenceRejected)
    }

    fn set_session_origin(
        &mut self,
        origin: SessionTimeOrigin,
    ) -> Result<(), WindowsPauseAdapterError>;

    fn verify_and_build(
        &mut self,
        server_generation: u64,
        run: &WindowsSealedScreenRun,
        post_close_observed_at: std::time::Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError>;

    /// The exact selected audio leaves accompany their paired screen run. Old
    /// screen-only test factories remain valid through the empty default;
    /// a nonempty list must be implemented by a verifier that owns the same
    /// capture root and can re-open every no-replace WAV leaf.
    fn verify_and_build_with_audio(
        &mut self,
        server_generation: u64,
        run: &WindowsSealedScreenRun,
        audio: &[WindowsSealedAudioRun],
        post_close_observed_at: std::time::Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        if !audio.is_empty() {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        self.verify_and_build(server_generation, run, post_close_observed_at)
    }
}
