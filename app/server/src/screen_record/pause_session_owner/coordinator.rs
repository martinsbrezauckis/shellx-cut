use super::super::pause_projection::SealedLegacyProjectionRun;
use super::super::pause_worker_protocol::{
    PauseWorkerCommand, PauseWorkerCoordinator, PauseWorkerFact, PauseWorkerFactResult,
    WorkerBoundaryKind,
};
use super::super::run_seal_coordinator::{
    PauseSealRequest, RecordingSessionJournalSink, ResumeReadinessEvidence, RunSealCoordinator,
    SealedRunEvidence, SessionTimeOrigin,
};
use super::types::{
    PauseSessionFactOutcome, PauseSessionOwnerError, PauseSessionOwnerPhase,
    PauseSessionProjectionExecutor, PauseSessionStopRequest, PauseSessionWorkerAdapter,
};
use record_capture::SelectedCaptureStreams;
use record_recovery::{RecordingSessionJournal, RecordingStream, TerminalDisposition};
use std::time::Instant;

pub(crate) struct PauseSessionOwner<J, W>
where
    J: RecordingSessionJournalSink,
    W: PauseSessionWorkerAdapter,
{
    seal: RunSealCoordinator<J>,
    workers: PauseWorkerCoordinator,
    adapter: W,
    streams: SelectedCaptureStreams,
    phase: PauseSessionOwnerPhase,
    projection_completed: bool,
}

impl<J, W> PauseSessionOwner<J, W>
where
    J: RecordingSessionJournalSink,
    W: PauseSessionWorkerAdapter,
{
    pub(crate) fn new(
        journal: J,
        streams: SelectedCaptureStreams,
        adapter: W,
    ) -> Result<Self, PauseSessionOwnerError> {
        let seal = RunSealCoordinator::new(journal, streams.clone())?;
        Ok(Self {
            seal,
            workers: PauseWorkerCoordinator::new(streams.clone()),
            adapter,
            streams,
            phase: PauseSessionOwnerPhase::Preparing,
            projection_completed: false,
        })
    }

    pub(crate) fn start_after_backend_origin(
        &mut self,
        origin: SessionTimeOrigin,
    ) -> Result<(), PauseSessionOwnerError> {
        self.require_phase("start", PauseSessionOwnerPhase::Preparing)?;
        self.seal.start_after_backend_origin(origin)?;
        self.phase = PauseSessionOwnerPhase::Recording;
        Ok(())
    }

    pub(crate) fn request_pause_at(
        &mut self,
        at: Instant,
    ) -> Result<PauseSealRequest, PauseSessionOwnerError> {
        self.require_phase("pause", PauseSessionOwnerPhase::Recording)?;
        let request = self.seal.request_pause_at(at)?;
        let command = self
            .workers
            .issue_pause_at(at)
            .map_err(|error| self.blocked_worker_command(error))?;
        self.match_generation(request.boundary().generation, &command)
            .map_err(|error| self.blocked(error))?;
        self.phase = PauseSessionOwnerPhase::PauseAwaitingFacts {
            generation: request.boundary().generation,
        };
        self.dispatch(command)
            .map_err(|error| self.blocked(error))?;
        Ok(request)
    }

    pub(crate) fn accept_worker_fact(
        &mut self,
        fact: PauseWorkerFact,
        observed_at: Instant,
    ) -> Result<PauseSessionFactOutcome, PauseSessionOwnerError> {
        let phase = self.phase;
        // The worker protocol intentionally retains its pending generation so
        // `Stop` can invalidate it. Once this owner has blocked or started
        // stopping, never delegate another fact to that protocol: an adapter
        // delivery failure must not let a late fact mutate hidden state.
        if !matches!(
            phase,
            PauseSessionOwnerPhase::PauseAwaitingFacts { .. }
                | PauseSessionOwnerPhase::ResumeAwaitingFacts { .. }
        ) {
            return Err(self.wrong_phase("accept worker fact"));
        }
        let result = self.workers.accept_fact(fact);
        match result {
            PauseWorkerFactResult::Rejected(rejection) => {
                Err(PauseSessionOwnerError::WorkerFact(rejection))
            }
            PauseWorkerFactResult::Accepted {
                completed_epoch: false,
                ..
            } => match phase {
                PauseSessionOwnerPhase::PauseAwaitingFacts { .. }
                | PauseSessionOwnerPhase::ResumeAwaitingFacts { .. } => {
                    Ok(PauseSessionFactOutcome::AwaitingWorkerFacts)
                }
                _ => Err(self.wrong_phase("accept worker fact")),
            },
            PauseWorkerFactResult::Accepted {
                completed_epoch: true,
                ..
            } => self.complete_worker_epoch(phase),
            PauseWorkerFactResult::Refused {
                command,
                generation,
                stream,
                ..
            } => self.handle_refusal(phase, command, generation, stream, observed_at),
            PauseWorkerFactResult::Failed { .. } => {
                self.phase = PauseSessionOwnerPhase::Blocked;
                Ok(PauseSessionFactOutcome::Blocked)
            }
        }
    }

    pub(crate) fn seal_pause_at(
        &mut self,
        evidence: SealedRunEvidence,
        sealed_at: Instant,
    ) -> Result<(), PauseSessionOwnerError> {
        let PauseSessionOwnerPhase::PauseAwaitingSeal { generation } = self.phase else {
            return Err(self.wrong_phase("seal pause"));
        };
        if evidence.generation() != generation {
            return Err(PauseSessionOwnerError::MismatchedGeneration {
                expected: generation,
                received: evidence.generation(),
            });
        }
        self.seal.seal_pause_at(evidence, sealed_at)?;
        self.phase = PauseSessionOwnerPhase::Paused;
        Ok(())
    }

    pub(crate) fn request_resume_at(&mut self, at: Instant) -> Result<(), PauseSessionOwnerError> {
        self.require_phase("resume", PauseSessionOwnerPhase::Paused)?;
        let boundary = self.seal.request_resume_at(at)?;
        let command = self
            .workers
            .issue_resume_at(at)
            .map_err(|error| self.blocked_worker_command(error))?;
        self.match_generation(boundary.generation, &command)
            .map_err(|error| self.blocked(error))?;
        self.phase = PauseSessionOwnerPhase::ResumeAwaitingFacts {
            generation: boundary.generation,
        };
        self.dispatch(command).map_err(|error| self.blocked(error))
    }

    pub(crate) fn seal_resume_at(
        &mut self,
        ready_at: Instant,
    ) -> Result<(), PauseSessionOwnerError> {
        let PauseSessionOwnerPhase::ResumeAwaitingDurability { generation } = self.phase else {
            return Err(self.wrong_phase("seal resume"));
        };
        self.seal.seal_resume_at(
            ResumeReadinessEvidence::new(generation, self.streams.streams().to_vec()),
            ready_at,
        )?;
        self.phase = PauseSessionOwnerPhase::Recording;
        Ok(())
    }

    /// Stop invalidates every worker epoch before command delivery. A late fact
    /// cannot reopen the session even if native delivery later reports an error.
    pub(crate) fn request_stop_at(
        &mut self,
        at: Instant,
    ) -> Result<PauseSessionStopRequest, PauseSessionOwnerError> {
        if matches!(
            self.phase,
            PauseSessionOwnerPhase::Stopping | PauseSessionOwnerPhase::Stopped
        ) {
            return Err(self.wrong_phase("stop"));
        }
        let request = self.seal.request_stop_at(at)?;
        // Stop is terminal inside the owner before any worker operation. Even
        // a failed command issue/delivery therefore cannot leave a stale
        // pause/resume state that accepts or advertises later facts.
        self.phase = PauseSessionOwnerPhase::Stopping;
        let command = self
            .workers
            .issue_stop_at(at)
            .map_err(PauseSessionOwnerError::WorkerCommand)?;
        let epoch = command.epoch();
        self.dispatch(command)?;
        Ok(PauseSessionStopRequest::new(request, epoch))
    }

    pub(crate) fn seal_stop_at(
        &mut self,
        evidence: Option<SealedRunEvidence>,
        disposition: TerminalDisposition,
        terminal_at: Instant,
    ) -> Result<(), PauseSessionOwnerError> {
        self.require_phase("seal stop", PauseSessionOwnerPhase::Stopping)?;
        self.seal.seal_stop_at(evidence, disposition, terminal_at)?;
        self.phase = PauseSessionOwnerPhase::Stopped;
        Ok(())
    }

    pub(crate) fn execute_completed_projection<P>(
        &mut self,
        projection: &mut P,
    ) -> Result<(), PauseSessionOwnerError>
    where
        P: PauseSessionProjectionExecutor,
    {
        self.require_phase("project", PauseSessionOwnerPhase::Stopped)?;
        if self.projection_completed
            || self
                .seal
                .journal_sink()
                .journal()
                .terminal()
                .map(|terminal| terminal.disposition)
                != Some(TerminalDisposition::Completed)
        {
            return Err(PauseSessionOwnerError::ProjectionUnavailable);
        }
        let event_runs: Vec<SealedLegacyProjectionRun> = self
            .seal
            .sealed_evidence()
            .iter()
            .map(SealedRunEvidence::legacy_projection_run)
            .collect();
        projection
            .execute(self.seal.journal_sink().journal(), &event_runs)
            .map_err(PauseSessionOwnerError::Projection)?;
        self.projection_completed = true;
        Ok(())
    }

    pub(crate) fn phase(&self) -> PauseSessionOwnerPhase {
        self.phase
    }
    pub(crate) fn block(&mut self) {
        if self.phase != PauseSessionOwnerPhase::Stopped {
            self.phase = PauseSessionOwnerPhase::Blocked;
        }
    }
    pub(crate) fn journal(&self) -> &RecordingSessionJournal {
        self.seal.journal_sink().journal()
    }
    pub(crate) fn into_worker_adapter(self) -> W {
        self.adapter
    }

    fn complete_worker_epoch(
        &mut self,
        phase: PauseSessionOwnerPhase,
    ) -> Result<PauseSessionFactOutcome, PauseSessionOwnerError> {
        match phase {
            PauseSessionOwnerPhase::PauseAwaitingFacts { generation } => {
                self.phase = PauseSessionOwnerPhase::PauseAwaitingSeal { generation };
                Ok(PauseSessionFactOutcome::PauseFactsComplete { generation })
            }
            PauseSessionOwnerPhase::ResumeAwaitingFacts { generation } => {
                self.phase = PauseSessionOwnerPhase::ResumeAwaitingDurability { generation };
                Ok(PauseSessionFactOutcome::ResumeFactsComplete { generation })
            }
            _ => Err(self.wrong_phase("complete worker facts")),
        }
    }

    fn handle_refusal(
        &mut self,
        phase: PauseSessionOwnerPhase,
        command: WorkerBoundaryKind,
        generation: u64,
        stream: RecordingStream,
        observed_at: Instant,
    ) -> Result<PauseSessionFactOutcome, PauseSessionOwnerError> {
        if command == WorkerBoundaryKind::Resume
            && matches!(phase, PauseSessionOwnerPhase::ResumeAwaitingFacts { generation: expected } if expected == generation)
        {
            self.seal
                .refuse_resume_at(stream, generation, observed_at)
                .map_err(|error| self.blocked(PauseSessionOwnerError::RunSeal(error)))?;
            self.phase = PauseSessionOwnerPhase::Paused;
            return Ok(PauseSessionFactOutcome::ResumeRefused { generation });
        }
        self.phase = PauseSessionOwnerPhase::Blocked;
        Ok(PauseSessionFactOutcome::Blocked)
    }

    fn dispatch(&mut self, command: PauseWorkerCommand) -> Result<(), PauseSessionOwnerError> {
        self.adapter
            .dispatch(&command)
            .map_err(PauseSessionOwnerError::WorkerDispatch)
    }

    fn match_generation(
        &self,
        expected: u64,
        command: &PauseWorkerCommand,
    ) -> Result<(), PauseSessionOwnerError> {
        let received = command
            .generation()
            .ok_or_else(|| self.wrong_phase("match terminal worker command"))?;
        (expected == received)
            .then_some(())
            .ok_or(PauseSessionOwnerError::MismatchedGeneration { expected, received })
    }

    fn require_phase(
        &self,
        action: &'static str,
        expected: PauseSessionOwnerPhase,
    ) -> Result<(), PauseSessionOwnerError> {
        (self.phase == expected)
            .then_some(())
            .ok_or_else(|| self.wrong_phase(action))
    }

    fn wrong_phase(&self, action: &'static str) -> PauseSessionOwnerError {
        PauseSessionOwnerError::WrongPhase {
            action,
            actual: self.phase,
        }
    }

    fn blocked_worker_command(
        &mut self,
        error: super::super::pause_worker_protocol::PauseWorkerCommandRejection,
    ) -> PauseSessionOwnerError {
        self.blocked(PauseSessionOwnerError::WorkerCommand(error))
    }

    fn blocked(&mut self, error: PauseSessionOwnerError) -> PauseSessionOwnerError {
        self.phase = PauseSessionOwnerPhase::Blocked;
        error
    }
}
