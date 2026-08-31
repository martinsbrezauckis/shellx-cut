use super::{WindowsPauseLifecycle, WindowsPauseSession, WindowsPauseSessionError};
use crate::screen_record::pause_session_owner::{
    PauseSessionFactOutcome, PauseSessionOwnerError, PauseSessionOwnerPhase,
};
use crate::screen_record::run_seal_coordinator::{RecordingSessionJournalSink, SealedRunEvidence};
use crate::screen_record::windows_pause_adapter::{
    WindowsPauseAdapterError, WindowsPauseEvidenceFactory,
};
use std::time::Instant;

impl<J, W, L> WindowsPauseSession<J, W, L>
where
    J: RecordingSessionJournalSink,
    W: WindowsPauseEvidenceFactory,
    L: WindowsPauseLifecycle,
{
    pub(super) fn fail_adapter(
        &mut self,
        _error: WindowsPauseAdapterError,
    ) -> WindowsPauseSessionError {
        self.fail()
    }

    pub(super) fn accept_pause_facts(
        &mut self,
        fact: super::super::pause_worker_protocol::PauseWorkerFact,
        additional_facts: Vec<super::super::pause_worker_protocol::PauseWorkerFact>,
        observed_at: Instant,
        generation: u64,
    ) -> bool {
        self.accept_facts(fact, additional_facts, observed_at, generation, true)
    }

    pub(super) fn accept_resume_facts(
        &mut self,
        fact: super::super::pause_worker_protocol::PauseWorkerFact,
        additional_facts: Vec<super::super::pause_worker_protocol::PauseWorkerFact>,
        observed_at: Instant,
        generation: u64,
    ) -> bool {
        self.accept_facts(fact, additional_facts, observed_at, generation, false)
    }

    fn accept_facts(
        &mut self,
        fact: super::super::pause_worker_protocol::PauseWorkerFact,
        additional_facts: Vec<super::super::pause_worker_protocol::PauseWorkerFact>,
        observed_at: Instant,
        generation: u64,
        pause: bool,
    ) -> bool {
        let fact_count = additional_facts.len() + 1;
        for (index, fact) in std::iter::once(fact).chain(additional_facts).enumerate() {
            let outcome = self.owner.accept_worker_fact(fact, observed_at);
            if index + 1 == fact_count {
                return if pause {
                    matches!(
                        outcome,
                        Ok(PauseSessionFactOutcome::PauseFactsComplete { generation: actual }) if actual == generation
                    )
                } else {
                    matches!(
                        outcome,
                        Ok(PauseSessionFactOutcome::ResumeFactsComplete { generation: actual }) if actual == generation
                    )
                };
            }
            if !matches!(outcome, Ok(PauseSessionFactOutcome::AwaitingWorkerFacts)) {
                return false;
            }
        }
        false
    }

    pub(super) fn owner_error(
        &mut self,
        error: PauseSessionOwnerError,
    ) -> WindowsPauseSessionError {
        if matches!(
            self.owner.phase(),
            PauseSessionOwnerPhase::Blocked | PauseSessionOwnerPhase::Stopping
        ) {
            self.fail()
        } else {
            WindowsPauseSessionError::Owner(error)
        }
    }

    pub(super) fn fail(&mut self) -> WindowsPauseSessionError {
        self.translator.discard_staged();
        self.owner.block();
        let _ = self.lifecycle.shutdown_and_join();
        self.lifecycle_closed = true;
        WindowsPauseSessionError::Lifecycle
    }

    pub(super) fn pin_recording_input_sidecar(
        &mut self,
        evidence: &SealedRunEvidence,
    ) -> Result<(), WindowsPauseSessionError> {
        let Some(sidecars) = self.input_sidecars.as_ref() else {
            return Ok(());
        };
        let readiness = self
            .owner
            .journal()
            .transitions()
            .last()
            .cloned()
            .ok_or(WindowsPauseSessionError::Lifecycle)?;
        let pin = sidecars
            .publish_or_reopen(evidence, &readiness)
            .map_err(|_| WindowsPauseSessionError::Lifecycle)?;
        self.owner
            .pin_recording_input_sidecar(evidence, pin)
            .map_err(|_| WindowsPauseSessionError::Lifecycle)
    }
}
