use super::coordinator::RunSealCoordinator;
use super::types::{
    invalid, ExpectedRun, RecordingSessionJournalSink, ResumeReadinessEvidence, RunBoundaryFloor,
    RunSealCoordinatorError, SealedRunEvidence, SessionTimeOrigin,
};
use record_capture::{
    AcknowledgementResult, PauseStreamCoordinator, SessionPhase, StreamAcknowledgement,
    StreamBoundary,
};
use record_core::{merge_sealed_event_tracks, SealedEventTrackRun};
use record_recovery::{
    DurableStateTransition, RecordingSessionJournalEntry, RecordingSessionState, RecordingStream,
};
use std::collections::BTreeSet;
use std::time::Instant;

impl<J: RecordingSessionJournalSink> RunSealCoordinator<J> {
    pub(super) fn validate_evidence(
        &self,
        evidence: &SealedRunEvidence,
        expected: &ExpectedRun,
        floor: RunBoundaryFloor,
        sealed_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        if evidence.generation != expected.generation
            || evidence.run.sequence != expected.sequence
            || evidence.run.observed_start_ms != expected.observed_start_ms
            || evidence.run.logical_start_ms != expected.logical_start_ms
        {
            return Err(invalid(
                "sealed evidence does not match the active generation and exact run start",
            ));
        }
        let observed_span = evidence
            .run
            .observed_end_ms
            .checked_sub(evidence.run.observed_start_ms)
            .ok_or_else(|| invalid("sealed evidence has an inverted observed interval"))?;
        let logical_span = evidence
            .run
            .logical_end_ms
            .checked_sub(evidence.run.logical_start_ms)
            .ok_or_else(|| invalid("sealed evidence has an inverted logical interval"))?;
        if observed_span == 0
            || logical_span == 0
            || evidence.run.observed_end_ms < floor.observed_ms
            || evidence.run.logical_end_ms < floor.logical_ms
            || logical_span > observed_span
        {
            return Err(invalid(
                "sealed evidence endpoint predates command correlation or exceeds observed capture",
            ));
        }
        if evidence.run.observed_end_ms > self.origin()?.elapsed_ms(sealed_at)? {
            return Err(invalid(
                "sealed evidence endpoint was not observed before durable sealing",
            ));
        }
        if evidence
            .run
            .fragments
            .iter()
            .any(|fragment| fragment.facts.media_duration_ms > logical_span)
        {
            return Err(invalid(
                "sealed media duration exceeds its accepted logical run span",
            ));
        }
        self.validate_exact_stream_set(&evidence.sealed_streams, "sealed evidence")?;
        let fragment_streams = evidence
            .run
            .fragments
            .iter()
            .map(|fragment| fragment.stream)
            .collect::<BTreeSet<_>>();
        if fragment_streams != self.selected_stream_set() {
            return Err(invalid(
                "sealed run fragments do not cover exactly the selected stream set",
            ));
        }
        let (sequence, logical_start_ms, logical_end_ms) =
            evidence.legacy_projection_run.identity();
        if sequence != evidence.run.sequence
            || logical_start_ms != evidence.run.logical_start_ms
            || logical_end_ms != evidence.run.logical_end_ms
        {
            return Err(invalid(
                "legacy projection input does not match the sealed run identity",
            ));
        }

        let event_input = evidence.legacy_projection_run.event_input();
        if event_input.events.duration_ms != logical_span {
            return Err(invalid(
                "legacy projection event duration does not match its sealed run interval",
            ));
        }
        if !self
            .selected_streams
            .streams()
            .contains(&RecordingStream::InputEvents)
            && (!event_input.events.cursor.is_empty()
                || !event_input.events.clicks.is_empty()
                || !event_input.events.scrolls.is_empty()
                || !event_input.events.keys.is_empty())
        {
            return Err(invalid(
                "legacy projection contains input events although InputEvents was not selected",
            ));
        }
        // ARCH-TIME-01: Settings.fps is the legacy f32 renderer timebase.
        // It cannot validate a fractional immutable request; optional
        // CaptureCadence@1 remains journal metadata and this private future
        // seam still does not create an executable pause/resume path.
        if !self.intent.record_keys && !event_input.events.keys.is_empty() {
            return Err(invalid(
                "legacy projection contains key events although intent disabled keys",
            ));
        }
        let mut event_runs: Vec<SealedEventTrackRun> = self
            .sealed_evidence
            .iter()
            .map(|sealed| sealed.legacy_projection_run.event_input())
            .collect();
        event_runs.push(event_input);
        merge_sealed_event_tracks(&event_runs).map_err(|error| {
            invalid(format!(
                "sealed legacy event input is incompatible with prior durable evidence: {error}"
            ))
        })?;
        Ok(())
    }

    pub(super) fn validate_ready_streams(
        &self,
        readiness: &ResumeReadinessEvidence,
        boundary: &StreamBoundary,
    ) -> Result<(), RunSealCoordinatorError> {
        if readiness.generation != boundary.generation {
            return Err(invalid("resume readiness generation is stale or future"));
        }
        self.validate_exact_stream_set(&readiness.ready_streams, "resume readiness")
    }

    fn validate_exact_stream_set(
        &self,
        streams: &[RecordingStream],
        label: &str,
    ) -> Result<(), RunSealCoordinatorError> {
        let actual_len = streams.len();
        let streams = streams.iter().copied().collect::<BTreeSet<_>>();
        if actual_len != self.selected_streams.streams().len()
            || streams.len() != actual_len
            || streams != self.selected_stream_set()
        {
            return Err(invalid(format!(
                "{label} must be an exact duplicate-free selected stream set"
            )));
        }
        Ok(())
    }

    fn selected_stream_set(&self) -> BTreeSet<RecordingStream> {
        self.selected_streams.streams().iter().copied().collect()
    }

    pub(super) fn transition(
        &self,
        state: RecordingSessionState,
        logical_offset_ms: u64,
        at: Instant,
    ) -> Result<DurableStateTransition, RunSealCoordinatorError> {
        Ok(DurableStateTransition {
            sequence: self.next_transition_sequence,
            state,
            logical_offset_ms,
            observed_unix_ms: self.origin()?.unix_at(at)?,
        })
    }

    pub(super) fn preflight(
        &self,
        entries: &[RecordingSessionJournalEntry],
    ) -> Result<(), RunSealCoordinatorError> {
        let mut journal = self.journal.journal().clone();
        for entry in entries {
            match entry {
                RecordingSessionJournalEntry::Intent(_) => {
                    return Err(invalid("duplicate intent cannot be appended"))
                }
                RecordingSessionJournalEntry::Transition(transition) => {
                    journal.append_transition(transition.clone())?;
                }
                RecordingSessionJournalEntry::InputSidecar(pin) => {
                    journal.append_input_sidecar(pin.clone())?;
                }
                RecordingSessionJournalEntry::Run(run) => journal.seal_run(run.clone())?,
                RecordingSessionJournalEntry::Terminal(terminal) => {
                    journal.seal_terminal(terminal.clone())?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn append_durable(
        &mut self,
        entry: RecordingSessionJournalEntry,
    ) -> Result<(), RunSealCoordinatorError> {
        match self.journal.append_entry(entry) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.failed_after_append = true;
                Err(RunSealCoordinatorError::Journal(error))
            }
        }
    }

    pub(super) fn ensure_mutable(&self) -> Result<(), RunSealCoordinatorError> {
        if self.failed_after_append {
            return Err(RunSealCoordinatorError::FailedAfterDurableAppend);
        }
        if self.pending_stop.is_some() || self.logical.phase().is_terminal() {
            return Err(RunSealCoordinatorError::Stopped);
        }
        if self.pending_boundary.is_some() {
            return Err(RunSealCoordinatorError::BoundaryPending);
        }
        Ok(())
    }

    pub(super) fn pending_error(&self) -> RunSealCoordinatorError {
        if self.failed_after_append {
            RunSealCoordinatorError::FailedAfterDurableAppend
        } else if self.pending_stop.is_some() || self.logical.phase().is_terminal() {
            RunSealCoordinatorError::Stopped
        } else {
            RunSealCoordinatorError::Invalid("no matching pending boundary".into())
        }
    }

    pub(super) fn origin(&self) -> Result<SessionTimeOrigin, RunSealCoordinatorError> {
        self.time_origin
            .ok_or_else(|| invalid("backend clock origin has not been observed"))
    }

    pub(super) fn logical_timestamp_ms(&self, at: Instant) -> Result<u64, RunSealCoordinatorError> {
        let timestamp = self
            .logical
            .timestamp_at(at)
            .ok_or_else(|| invalid("logical timestamps are not enabled"))?;
        u64::try_from(timestamp.as_millis())
            .map_err(|_| invalid("logical elapsed time does not fit in milliseconds"))
    }

    pub(super) fn ensure_run_successor(&self) -> Result<(), RunSealCoordinatorError> {
        self.next_run_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("run sequence overflow"))?;
        Ok(())
    }

    pub(super) fn ensure_transition_successor(&self) -> Result<(), RunSealCoordinatorError> {
        self.next_transition_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("transition sequence overflow"))?;
        Ok(())
    }
}

pub(super) fn acknowledge_pause(
    coordinator: &mut PauseStreamCoordinator,
    boundary: &StreamBoundary,
    at: Instant,
) {
    for stream in boundary.streams() {
        let result = coordinator.acknowledge_at(
            StreamAcknowledgement::PauseSealed {
                stream: *stream,
                generation: boundary.generation,
            },
            at,
        );
        debug_assert!(matches!(result, AcknowledgementResult::Accepted { .. }));
    }
    debug_assert_eq!(coordinator.phase(), SessionPhase::Paused);
}

pub(super) fn acknowledge_resume(
    coordinator: &mut PauseStreamCoordinator,
    boundary: &StreamBoundary,
    at: Instant,
) {
    for stream in boundary.streams() {
        let result = coordinator.acknowledge_at(
            StreamAcknowledgement::ResumeReady {
                stream: *stream,
                generation: boundary.generation,
            },
            at,
        );
        debug_assert!(matches!(result, AcknowledgementResult::Accepted { .. }));
    }
    debug_assert_eq!(coordinator.phase(), SessionPhase::Recording);
}
