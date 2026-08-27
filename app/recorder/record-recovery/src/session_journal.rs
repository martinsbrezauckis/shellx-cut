//! Append-only recording-session state machine for future pause/resume writers.

use crate::session_contract::{
    DurableStateTransition, RecordingSessionIntent, RecordingSessionJournalEntry,
    RecordingSessionState, SealedRun, SessionJournalError, SessionTerminal,
};
use crate::session_validation::{transition_allowed, validate_intent, validate_run};

#[derive(Debug, Clone, PartialEq)]
pub struct RecordingSessionJournal {
    intent: RecordingSessionIntent,
    transitions: Vec<DurableStateTransition>,
    runs: Vec<SealedRun>,
    terminal: Option<SessionTerminal>,
    /// The transition that admitted the last sealed run. This preserves the
    /// requirement that a later run follows a durable resume, not just a reused
    /// active state value.
    last_run_transition_sequence: Option<u64>,
}

impl RecordingSessionJournal {
    pub fn new(intent: RecordingSessionIntent) -> Result<Self, SessionJournalError> {
        validate_intent(&intent)?;
        Ok(Self {
            intent,
            transitions: Vec::new(),
            runs: Vec::new(),
            terminal: None,
            last_run_transition_sequence: None,
        })
    }

    pub fn replay(
        entries: impl IntoIterator<Item = RecordingSessionJournalEntry>,
    ) -> Result<Self, SessionJournalError> {
        let mut entries = entries.into_iter();
        let Some(RecordingSessionJournalEntry::Intent(intent)) = entries.next() else {
            return Err(invalid("journal must begin with immutable intent"));
        };
        let mut journal = Self::new(intent)?;
        for entry in entries {
            match entry {
                RecordingSessionJournalEntry::Intent(_) => {
                    return Err(invalid("journal contains duplicate intent"))
                }
                RecordingSessionJournalEntry::Transition(transition) => {
                    journal.append_transition(transition)?
                }
                RecordingSessionJournalEntry::Run(run) => journal.seal_run(run)?,
                RecordingSessionJournalEntry::Terminal(terminal) => {
                    journal.seal_terminal(terminal)?
                }
            }
        }
        Ok(journal)
    }

    pub fn intent(&self) -> &RecordingSessionIntent {
        &self.intent
    }

    pub fn transitions(&self) -> &[DurableStateTransition] {
        &self.transitions
    }

    pub fn sealed_runs(&self) -> &[SealedRun] {
        &self.runs
    }

    pub fn terminal(&self) -> Option<&SessionTerminal> {
        self.terminal.as_ref()
    }

    pub fn entries(&self) -> Vec<RecordingSessionJournalEntry> {
        let mut entries = Vec::with_capacity(1 + self.transitions.len() + self.runs.len() + 1);
        entries.push(RecordingSessionJournalEntry::Intent(self.intent.clone()));
        let mut transitions = self.transitions.iter().peekable();
        for run in &self.runs {
            while let Some(transition) = transitions.peek() {
                if transition.logical_offset_ms > run.logical_start_ms {
                    break;
                }
                entries.push(RecordingSessionJournalEntry::Transition(
                    (**transition).clone(),
                ));
                transitions.next();
            }
            entries.push(RecordingSessionJournalEntry::Run(run.clone()));
        }
        for transition in transitions {
            entries.push(RecordingSessionJournalEntry::Transition(transition.clone()));
        }
        if let Some(terminal) = &self.terminal {
            entries.push(RecordingSessionJournalEntry::Terminal(terminal.clone()));
        }
        entries
    }

    pub fn append_transition(
        &mut self,
        transition: DurableStateTransition,
    ) -> Result<(), SessionJournalError> {
        if self.terminal.is_some() {
            return Err(invalid("cannot append after terminal disposition"));
        }
        let previous = self.transitions.last();
        if transition.sequence != self.transitions.len() as u64 {
            return Err(invalid("transition sequence is not contiguous"));
        }
        if previous.is_some_and(|previous| transition.observed_unix_ms < previous.observed_unix_ms)
        {
            return Err(invalid("transition wall time is non-monotonic"));
        }
        let state = previous.map(|value| value.state);
        if !transition_allowed(state, transition.state) {
            return Err(invalid("state transition is out of order"));
        }
        let logical_end = self.runs.last().map(|run| run.logical_end_ms).unwrap_or(0);
        match transition.state {
            RecordingSessionState::Started if transition.logical_offset_ms != 0 => {
                return Err(invalid("started transition must begin at logical zero"))
            }
            RecordingSessionState::Paused | RecordingSessionState::Stopping
                if transition.logical_offset_ms != logical_end =>
            {
                return Err(invalid("transition does not seal at the latest run end"))
            }
            RecordingSessionState::Resumed
                if previous.is_none_or(|previous| {
                    previous.state != RecordingSessionState::Paused
                        || previous.logical_offset_ms != transition.logical_offset_ms
                }) =>
            {
                return Err(invalid(
                    "resume must continue from the paused logical offset",
                ))
            }
            _ => {}
        }
        self.transitions.push(transition);
        Ok(())
    }

    pub fn seal_run(&mut self, run: SealedRun) -> Result<(), SessionJournalError> {
        if self.terminal.is_some() {
            return Err(invalid("cannot seal a run after terminal disposition"));
        }
        let Some(active_transition) = self.transitions.last() else {
            return Err(invalid("sealed runs require an active recording state"));
        };
        if !matches!(
            active_transition.state,
            RecordingSessionState::Started | RecordingSessionState::Resumed
        ) {
            return Err(invalid("sealed runs require an active recording state"));
        }
        if self.last_run_transition_sequence.is_some()
            && active_transition.state != RecordingSessionState::Resumed
        {
            return Err(invalid(
                "a later sealed run requires a pause and resume transition",
            ));
        }
        if self.last_run_transition_sequence == Some(active_transition.sequence) {
            return Err(invalid("an active transition already sealed a run"));
        }
        validate_run(&self.intent, self.runs.last(), &run)?;
        self.last_run_transition_sequence = Some(active_transition.sequence);
        self.runs.push(run);
        Ok(())
    }

    pub fn seal_terminal(&mut self, terminal: SessionTerminal) -> Result<(), SessionJournalError> {
        if self.terminal.is_some()
            || self.transitions.last().map(|transition| transition.state)
                != Some(RecordingSessionState::Stopping)
        {
            return Err(invalid(
                "terminal disposition requires a stopping transition",
            ));
        }
        let logical_end = self.runs.last().map(|run| run.logical_end_ms).unwrap_or(0);
        let stopping = self.transitions.last().expect("checked above");
        if terminal.logical_end_ms != logical_end
            || terminal.observed_unix_ms < stopping.observed_unix_ms
        {
            return Err(invalid(
                "terminal facts are not monotonic with the stopped session",
            ));
        }
        self.terminal = Some(terminal);
        Ok(())
    }
}

fn invalid(detail: &str) -> SessionJournalError {
    SessionJournalError::Invalid(detail.into())
}
