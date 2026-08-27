use super::types::{
    PauseWorkerCommand, PauseWorkerCommandRejection, PauseWorkerFact, PauseWorkerFactResult,
    PauseWorkerProtocolPhase, WorkerBoundaryKind, WorkerEpoch,
};
use super::validation::{validate_fact, PendingWorkerBoundary};
use record_capture::SelectedCaptureStreams;
use std::time::Instant;

/// Pure command/fact coordinator for one immutable selection of capture workers.
///
/// It never sends a command, starts a worker, writes a journal, or equates a
/// logical request with physical completion. A future registry supplies the
/// command to workers and returns their independently observed facts here.
#[derive(Debug)]
pub(crate) struct PauseWorkerCoordinator {
    streams: SelectedCaptureStreams,
    phase: PauseWorkerProtocolPhase,
    latest_generation: u64,
    next_epoch: u64,
    pending: Option<PendingWorkerBoundary>,
}

impl PauseWorkerCoordinator {
    /// Create a protocol ready to issue a pause for workers already recording.
    /// Construction itself neither observes nor starts any native worker.
    pub(crate) fn new(streams: SelectedCaptureStreams) -> Self {
        Self {
            streams,
            phase: PauseWorkerProtocolPhase::Recording,
            latest_generation: 0,
            next_epoch: 1,
            pending: None,
        }
    }

    /// Issue a private pause command for every selected stream.
    pub(crate) fn issue_pause_at(
        &mut self,
        issued_at: Instant,
    ) -> Result<PauseWorkerCommand, PauseWorkerCommandRejection> {
        self.issue_boundary_at(WorkerBoundaryKind::Pause, issued_at)
    }

    /// Issue a private resume command for every selected stream.
    pub(crate) fn issue_resume_at(
        &mut self,
        issued_at: Instant,
    ) -> Result<PauseWorkerCommand, PauseWorkerCommandRejection> {
        self.issue_boundary_at(WorkerBoundaryKind::Resume, issued_at)
    }

    /// Issue the terminal private stop command and invalidate every pending
    /// boundary epoch before any later worker fact can be considered.
    pub(crate) fn issue_stop_at(
        &mut self,
        _issued_at: Instant,
    ) -> Result<PauseWorkerCommand, PauseWorkerCommandRejection> {
        if self.phase == PauseWorkerProtocolPhase::Stopped {
            return Err(PauseWorkerCommandRejection::Stopped);
        }
        let epoch = self.take_epoch()?;
        self.pending = None;
        self.phase = PauseWorkerProtocolPhase::Stopped;
        Ok(PauseWorkerCommand::stop(epoch, self.streams.clone()))
    }

    /// Consider an independently observed physical worker fact.
    pub(crate) fn accept_fact(&mut self, fact: PauseWorkerFact) -> PauseWorkerFactResult {
        if let Err(rejection) = validate_fact(
            fact,
            &self.streams,
            self.phase,
            self.pending.as_ref(),
            self.latest_generation,
        ) {
            return PauseWorkerFactResult::Rejected(rejection);
        }

        let pending = self
            .pending
            .as_ref()
            .expect("validated physical fact requires a pending boundary");
        let command = pending.kind;
        let generation = pending.generation;
        let epoch = pending.epoch;
        let stream = fact.stream();

        if fact.is_refused() {
            self.pending = None;
            // A resume refusal is recoverable: logical time was already frozen
            // at the durable pause edge, so the owner may report Paused and
            // retry through a fresh generation. A pause refusal remains
            // blocked because the active run's seal ownership is unknown.
            self.phase = match command {
                WorkerBoundaryKind::Resume => PauseWorkerProtocolPhase::Paused,
                WorkerBoundaryKind::Pause => PauseWorkerProtocolPhase::Blocked,
            };
            return PauseWorkerFactResult::Refused {
                stream,
                command,
                generation,
                epoch,
            };
        }
        if fact.is_failed() {
            self.pending = None;
            self.phase = PauseWorkerProtocolPhase::Blocked;
            return PauseWorkerFactResult::Failed {
                stream,
                command,
                generation,
                epoch,
            };
        }

        let completed_epoch = {
            let pending = self
                .pending
                .as_mut()
                .expect("validated physical fact requires a pending boundary");
            let inserted = pending.acknowledged.insert(stream);
            debug_assert!(inserted, "duplicate facts are rejected before mutation");
            pending.complete(&self.streams)
        };
        if completed_epoch {
            self.pending = None;
            self.phase = match command {
                WorkerBoundaryKind::Pause => PauseWorkerProtocolPhase::Paused,
                WorkerBoundaryKind::Resume => PauseWorkerProtocolPhase::Recording,
            };
        }
        PauseWorkerFactResult::Accepted {
            phase: self.phase,
            completed_epoch,
        }
    }

    pub(crate) fn phase(&self) -> PauseWorkerProtocolPhase {
        self.phase
    }

    fn issue_boundary_at(
        &mut self,
        kind: WorkerBoundaryKind,
        issued_at: Instant,
    ) -> Result<PauseWorkerCommand, PauseWorkerCommandRejection> {
        self.ensure_boundary_phase(kind)?;
        let generation = self
            .latest_generation
            .checked_add(1)
            .ok_or(PauseWorkerCommandRejection::GenerationExhausted)?;
        let epoch = self.take_epoch()?;
        self.latest_generation = generation;
        self.phase = match kind {
            WorkerBoundaryKind::Pause => PauseWorkerProtocolPhase::PausePending,
            WorkerBoundaryKind::Resume => PauseWorkerProtocolPhase::ResumePending,
        };
        self.pending = Some(PendingWorkerBoundary {
            kind,
            generation,
            epoch,
            issued_at,
            acknowledged: Default::default(),
        });
        Ok(PauseWorkerCommand::boundary(
            kind,
            generation,
            epoch,
            self.streams.clone(),
        ))
    }

    fn ensure_boundary_phase(
        &self,
        kind: WorkerBoundaryKind,
    ) -> Result<(), PauseWorkerCommandRejection> {
        if self.phase == PauseWorkerProtocolPhase::Stopped {
            return Err(PauseWorkerCommandRejection::Stopped);
        }
        if self.phase == PauseWorkerProtocolPhase::Blocked {
            return Err(PauseWorkerCommandRejection::Blocked);
        }
        if self.pending.is_some() {
            return Err(PauseWorkerCommandRejection::BoundaryPending);
        }
        let expected = match kind {
            WorkerBoundaryKind::Pause => PauseWorkerProtocolPhase::Recording,
            WorkerBoundaryKind::Resume => PauseWorkerProtocolPhase::Paused,
        };
        if self.phase != expected {
            return Err(PauseWorkerCommandRejection::WrongPhase {
                expected,
                actual: self.phase,
            });
        }
        Ok(())
    }

    fn take_epoch(&mut self) -> Result<WorkerEpoch, PauseWorkerCommandRejection> {
        let next_epoch = self
            .next_epoch
            .checked_add(1)
            .ok_or(PauseWorkerCommandRejection::EpochExhausted)?;
        let epoch = WorkerEpoch::new(self.next_epoch);
        self.next_epoch = next_epoch;
        Ok(epoch)
    }
}
