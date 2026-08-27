use super::coordinator::RunSealCoordinator;
use super::types::{
    invalid, PendingPause, PendingResume, PendingStop, RecordingSessionJournalSink,
    ResumeReadinessEvidence, RunSealCoordinatorError, SealedRunEvidence,
};
use record_recovery::{
    RecordingSessionJournalEntry, RecordingSessionState, SessionTerminal, TerminalDisposition,
};
use std::time::Instant;

impl<J: RecordingSessionJournalSink> RunSealCoordinator<J> {
    pub(super) fn seal_pause_inner(
        &mut self,
        pending: &PendingPause,
        evidence: &SealedRunEvidence,
        sealed_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        self.validate_evidence(evidence, &pending.expected, pending.floor, sealed_at)?;
        let transition = self.transition(
            RecordingSessionState::Paused,
            evidence.run.logical_end_ms,
            sealed_at,
        )?;
        self.ensure_run_successor()?;
        self.ensure_transition_successor()?;
        let entries = vec![
            RecordingSessionJournalEntry::Run(evidence.run.clone()),
            RecordingSessionJournalEntry::Transition(transition),
        ];
        self.preflight(&entries)?;
        self.append_durable(entries[0].clone())?;
        self.append_durable(entries[1].clone())?;
        self.next_transition_sequence += 1;
        Ok(())
    }

    pub(super) fn seal_resume_inner(
        &mut self,
        pending: &PendingResume,
        readiness: &ResumeReadinessEvidence,
        ready_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        self.validate_ready_streams(readiness, &pending.boundary)?;
        let transition = self.transition(
            RecordingSessionState::Resumed,
            self.last_logical_end_ms,
            ready_at,
        )?;
        self.ensure_transition_successor()?;
        let entry = RecordingSessionJournalEntry::Transition(transition);
        self.preflight(std::slice::from_ref(&entry))?;
        self.append_durable(entry)?;
        self.next_transition_sequence += 1;
        Ok(())
    }

    pub(super) fn seal_stop_inner(
        &mut self,
        stop: &PendingStop,
        evidence: Option<&SealedRunEvidence>,
        disposition: TerminalDisposition,
        terminal_at: Instant,
    ) -> Result<(), RunSealCoordinatorError> {
        match (&stop.expected, stop.floor, evidence) {
            (Some(expected), Some(floor), Some(evidence)) => {
                self.validate_evidence(evidence, expected, floor, terminal_at)?
            }
            (None, None, None) => {}
            (Some(_), None, Some(_)) => {
                return Err(invalid("stop active run is missing command correlation"))
            }
            (Some(_), None, None) => {
                return Err(invalid("stop requires evidence for its active run"))
            }
            (None, Some(_), Some(_)) => {
                return Err(invalid("stop has no active run for supplied evidence"))
            }
            (None, Some(_), None) => {
                return Err(invalid(
                    "stop has stale run correlation without an active run",
                ))
            }
            (None, None, Some(_)) => {
                return Err(invalid("stop has no active run for supplied evidence"))
            }
            (Some(_), Some(_), None) => {
                return Err(invalid("stop requires evidence for its active run"))
            }
        }

        let logical_end_ms = evidence
            .map(|evidence| evidence.run.logical_end_ms)
            .unwrap_or(stop.terminal_logical_end_ms);

        let stopping =
            self.transition(RecordingSessionState::Stopping, logical_end_ms, terminal_at)?;
        let terminal = SessionTerminal {
            disposition,
            logical_end_ms,
            observed_unix_ms: self.origin()?.unix_at(terminal_at)?,
        };
        self.ensure_transition_successor()?;
        if evidence.is_some() {
            self.ensure_run_successor()?;
        }
        let mut entries = Vec::with_capacity(3);
        if let Some(evidence) = evidence {
            entries.push(RecordingSessionJournalEntry::Run(evidence.run.clone()));
        }
        entries.push(RecordingSessionJournalEntry::Transition(stopping));
        entries.push(RecordingSessionJournalEntry::Terminal(terminal));
        self.preflight(&entries)?;
        for entry in entries {
            self.append_durable(entry)?;
        }
        self.next_transition_sequence += 1;
        Ok(())
    }
}
