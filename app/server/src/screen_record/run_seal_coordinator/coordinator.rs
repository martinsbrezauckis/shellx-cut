use super::types::{
    invalid, ActiveRun, PendingBoundary, PendingStop, RecordingSessionJournalSink,
    ResumeReadinessEvidence, RunSealCoordinatorError, SealedRunEvidence, SessionTimeOrigin,
};
use super::validation::{acknowledge_pause, acknowledge_resume};
use record_capture::{
    AcknowledgementResult, PauseStreamCoordinator, SelectedCaptureStreams, SessionPhase,
    StreamAcknowledgement,
};
use record_recovery::{RecordingSessionIntent, RecordingSessionState, TerminalDisposition};
use std::time::Instant;

/// A bounded private coordinator for one fresh, intent-synced recording journal.
///
/// Construct this only with a newly created `RecordingSessionJournalFile` (whose
/// `create_new` call already synced `Intent`) or an equivalent sink that exposes
/// that exact initial prefix. There is deliberately no reopen/recovery path here.
pub(crate) struct RunSealCoordinator<J> {
    pub(super) journal: J,
    pub(super) intent: RecordingSessionIntent,
    pub(super) selected_streams: SelectedCaptureStreams,
    pub(super) logical: PauseStreamCoordinator,
    pub(super) time_origin: Option<SessionTimeOrigin>,
    pub(super) active_run: Option<ActiveRun>,
    pub(super) pending_boundary: Option<PendingBoundary>,
    pub(super) pending_stop: Option<PendingStop>,
    pub(super) sealed_evidence: Vec<SealedRunEvidence>,
    pub(super) next_run_sequence: u64,
    pub(super) next_transition_sequence: u64,
    pub(super) last_logical_end_ms: u64,
    pub(super) failed_after_append: bool,
}

impl<J: RecordingSessionJournalSink> RunSealCoordinator<J> {
    /// Bind one immutable stream registration to an intent-synced empty journal.
    /// The caller must create and sync the journal intent before it launches any
    /// worker; this constructor rejects an already-transitioned journal.
    pub(crate) fn new(
        journal: J,
        selected_streams: SelectedCaptureStreams,
    ) -> Result<Self, RunSealCoordinatorError> {
        let snapshot = journal.journal();
        if !snapshot.transitions().is_empty()
            || !snapshot.sealed_runs().is_empty()
            || snapshot.terminal().is_some()
        {
            return Err(invalid(
                "run-seal coordinator requires an intent-only journal prefix",
            ));
        }
        if snapshot.intent().requested_streams != selected_streams.streams() {
            return Err(invalid(
                "immutable selected streams do not exactly match the journal intent",
            ));
        }

        let intent = snapshot.intent().clone();
        let budget = intent
            .active_duration_limit_ms
            .map(std::time::Duration::from_millis);
        Ok(Self {
            journal,
            intent,
            selected_streams: selected_streams.clone(),
            logical: PauseStreamCoordinator::new(selected_streams, budget),
            time_origin: None,
            active_run: None,
            pending_boundary: None,
            pending_stop: None,
            sealed_evidence: Vec::new(),
            next_run_sequence: 0,
            next_transition_sequence: 0,
            last_logical_end_ms: 0,
            failed_after_append: false,
        })
    }

    /// Durably enter `Started` at the already-observed shared backend origin.
    /// Intent was synced by the injected journal before this call, so a failed
    /// start leaves the logical coordinator in `Preparing`.
    pub(crate) fn start_after_backend_origin(
        &mut self,
        origin: SessionTimeOrigin,
    ) -> Result<(), RunSealCoordinatorError> {
        self.ensure_mutable()?;
        if self.time_origin.is_some() {
            return Err(invalid("backend origin was already observed"));
        }
        if origin.unix_ms < self.intent.created_unix_ms {
            return Err(invalid(
                "backend origin Unix time predates the immutable intent",
            ));
        }
        let transition = record_recovery::DurableStateTransition {
            sequence: self.next_transition_sequence,
            state: RecordingSessionState::Started,
            logical_offset_ms: 0,
            observed_unix_ms: origin.unix_ms,
        };
        self.ensure_transition_successor()?;
        self.preflight(&[record_recovery::RecordingSessionJournalEntry::Transition(
            transition.clone(),
        )])?;
        self.append_durable(record_recovery::RecordingSessionJournalEntry::Transition(
            transition,
        ))?;

        let result = self.logical.start_at(origin.monotonic);
        debug_assert!(result.changed());
        self.time_origin = Some(origin);
        self.next_transition_sequence += 1;
        self.active_run = Some(ActiveRun {
            generation: 0,
            sequence: self.next_run_sequence,
            observed_start_ms: 0,
            logical_start_ms: 0,
        });
        Ok(())
    }

    /// Preflight one complete paused run, append `Run` then `Paused`, and only
    /// then accept the matching logical `PauseSealed` acknowledgements.
    pub(crate) fn seal_pause_at(
        &mut self,
        evidence: SealedRunEvidence,
        sealed_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        let Some(PendingBoundary::Pause(pending)) = self.pending_boundary.take() else {
            return Err(self.pending_error());
        };

        let result = self.seal_pause_inner(&pending, &evidence, sealed_at);
        if let Err(error) = result {
            self.pending_boundary = Some(PendingBoundary::Pause(pending));
            return Err(error);
        }

        acknowledge_pause(&mut self.logical, &pending.boundary, sealed_at);
        if !self
            .logical
            .accept_post_close_elapsed(std::time::Duration::from_millis(
                evidence.run.logical_end_ms,
            ))
        {
            self.failed_after_append = true;
            return Err(invalid(
                "sealed pause evidence could not update the frozen logical endpoint",
            ));
        }
        self.last_logical_end_ms = evidence.run.logical_end_ms;
        self.next_run_sequence += 1;
        self.active_run = None;
        self.sealed_evidence.push(evidence);
        Ok(())
    }

    /// Safely abandon an uncommitted resume after one selected worker refuses
    /// its exact pending generation. No durable `Resumed` transition is ever
    /// written: the journal remains at the already durable `Paused` boundary
    /// and logical timestamps stay frozen. A later retry must issue a fresh
    /// generation rather than reuse this one.
    pub(crate) fn refuse_resume_at(
        &mut self,
        stream: record_recovery::RecordingStream,
        generation: u64,
        at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        let Some(PendingBoundary::Resume(pending)) = self.pending_boundary.take() else {
            return Err(self.pending_error());
        };
        if pending.boundary.generation != generation {
            self.pending_boundary = Some(PendingBoundary::Resume(pending));
            return Err(invalid("resume refusal generation is stale or future"));
        }
        match self.logical.acknowledge_at(
            StreamAcknowledgement::ResumeRefused { stream, generation },
            at,
        ) {
            AcknowledgementResult::ResumeRefused(_) => Ok(()),
            AcknowledgementResult::Rejected(_) | AcknowledgementResult::Accepted { .. } => {
                self.pending_boundary = Some(PendingBoundary::Resume(pending));
                Err(invalid(
                    "resume refusal did not match the pending worker boundary",
                ))
            }
        }
    }

    /// Append `Resumed` only after every selected stream has provided exact
    /// generation-matching readiness. The logical coordinator receives the
    /// corresponding acknowledgements after that append succeeds.
    pub(crate) fn seal_resume_at(
        &mut self,
        readiness: ResumeReadinessEvidence,
        ready_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        let Some(PendingBoundary::Resume(pending)) = self.pending_boundary.take() else {
            return Err(self.pending_error());
        };
        let observed_start_ms = match self.origin().and_then(|origin| origin.elapsed_ms(ready_at)) {
            Ok(observed_start_ms) => observed_start_ms,
            Err(error) => {
                self.pending_boundary = Some(PendingBoundary::Resume(pending));
                return Err(error);
            }
        };

        let result = self.seal_resume_inner(&pending, &readiness, ready_at);
        if let Err(error) = result {
            self.pending_boundary = Some(PendingBoundary::Resume(pending));
            return Err(error);
        }

        acknowledge_resume(&mut self.logical, &pending.boundary, ready_at);
        self.active_run = Some(ActiveRun {
            generation: pending.boundary.generation,
            sequence: self.next_run_sequence,
            observed_start_ms,
            logical_start_ms: self.last_logical_end_ms,
        });
        Ok(())
    }

    /// Finish the terminal durable sequence exactly once. A live active run must
    /// first validate and append as `Run`; only then can `Stopping` and one of
    /// the three fixed terminal dispositions be appended.
    pub(crate) fn seal_stop_at(
        &mut self,
        evidence: Option<SealedRunEvidence>,
        disposition: TerminalDisposition,
        terminal_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        if self.failed_after_append {
            return Err(RunSealCoordinatorError::FailedAfterDurableAppend);
        }
        let Some(stop) = self.pending_stop.take() else {
            return Err(RunSealCoordinatorError::Stopped);
        };

        let result = self.seal_stop_inner(&stop, evidence.as_ref(), disposition, terminal_at);
        if let Err(error) = result {
            self.pending_stop = Some(stop);
            return Err(error);
        }

        if let Some(evidence) = evidence {
            if !self
                .logical
                .accept_post_close_elapsed(std::time::Duration::from_millis(
                    evidence.run.logical_end_ms,
                ))
            {
                self.failed_after_append = true;
                return Err(invalid(
                    "sealed stop evidence could not update the frozen logical endpoint",
                ));
            }
            self.last_logical_end_ms = evidence.run.logical_end_ms;
            self.next_run_sequence += 1;
            self.sealed_evidence.push(evidence);
        }
        self.active_run = None;
        Ok(())
    }

    pub(crate) fn phase(&self) -> SessionPhase {
        self.logical.phase()
    }

    pub(crate) fn sealed_evidence(&self) -> &[SealedRunEvidence] {
        &self.sealed_evidence
    }

    pub(crate) fn journal_sink(&self) -> &J {
        &self.journal
    }
}
