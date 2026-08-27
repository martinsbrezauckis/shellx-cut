//! Pure projection from a completed pause-aware session to legacy root outputs.
//!
//! This is intentionally not an executable recovery path. It selects the exact
//! compact source and event plans a later server coordinator must write as the
//! legacy `source.mp4` and `events.json` roots after every input has been sealed.
//! No file is opened, no encoder is started, and no public verb calls this model.

use cut_core::{error_codes, CutError};
use record_core::{
    merge_sealed_event_tracks, CaptureCadence, MergedEventTrack, SealedEventTrackRun, Settings,
};
use record_recovery::{
    plan_run_aware_stitch, RecordingSessionJournal, RecordingSessionJournalEntry,
    RunAwareStitchPlan, RunAwareStitchSpan, TerminalDisposition,
};

pub(crate) const LEGACY_ROOT_SOURCE_OUTPUT: &str = "source.mp4";
pub(crate) const LEGACY_ROOT_EVENTS_OUTPUT: &str = "events.json";

/// One already-sealed legacy event stream matched to one compact journal run.
///
/// The explicit half-open range makes the ownership boundary checkable before
/// `EventTrack::duration_ms` is trusted. The stored values are private so the
/// projection plan cannot later be changed into a different run identity.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SealedLegacyProjectionRun {
    run_sequence: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    settings: Settings,
    events: record_core::EventTrack,
}

impl SealedLegacyProjectionRun {
    pub(crate) fn new(
        run_sequence: u64,
        logical_start_ms: u64,
        logical_end_ms: u64,
        settings: Settings,
        events: record_core::EventTrack,
    ) -> Self {
        Self {
            run_sequence,
            logical_start_ms,
            logical_end_ms,
            settings,
            events,
        }
    }

    /// Return the immutable run identity without lending the event payload for
    /// mutation. The private run-seal coordinator uses this before it appends
    /// the matching durable `SealedRun`.
    pub(crate) fn identity(&self) -> (u64, u64, u64) {
        (
            self.run_sequence,
            self.logical_start_ms,
            self.logical_end_ms,
        )
    }

    /// Clone the event input in the exact form consumed by the pure projection
    /// validator. This remains an in-memory contract; it never opens an output
    /// path or writes a legacy root.
    pub(crate) fn event_input(&self) -> SealedEventTrackRun {
        SealedEventTrackRun {
            logical_offset_ms: self.logical_start_ms,
            settings: self.settings,
            events: self.events.clone(),
        }
    }
}

/// Immutable description of the legacy root outputs a future executor may make.
///
/// The target names are fixed rather than caller-provided paths. Accessors only
/// lend the plans, preventing a later caller from mutating the qualified result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LegacyRootProjectionPlan {
    source_output: &'static str,
    events_output: &'static str,
    source_stitch: RunAwareStitchPlan,
    merged_events: MergedEventTrack,
    /// An optional immutable request/probe record forwarded from the journal.
    /// This remains projection metadata only; it does not imply an executable
    /// pause/resume path for the legacy renderer.
    capture_cadence: Option<CaptureCadence>,
}

impl LegacyRootProjectionPlan {
    pub(crate) fn source_output(&self) -> &'static str {
        self.source_output
    }

    pub(crate) fn events_output(&self) -> &'static str {
        self.events_output
    }

    pub(crate) fn source_stitch(&self) -> &RunAwareStitchPlan {
        &self.source_stitch
    }

    pub(crate) fn merged_events(&self) -> &MergedEventTrack {
        &self.merged_events
    }

    pub(crate) fn capture_cadence(&self) -> Option<&CaptureCadence> {
        self.capture_cadence.as_ref()
    }
}

/// Build a legacy-root projection from one replay-valid completed session.
///
/// The journal is replayed defensively before it is used, even though its type
/// normally originates from validated in-process transitions. That protects this
/// boundary when a future JSONL reader or transport reconstruction supplies it.
pub(crate) fn plan_legacy_root_projection(
    journal: &RecordingSessionJournal,
    event_runs: &[SealedLegacyProjectionRun],
) -> Result<LegacyRootProjectionPlan, CutError> {
    plan_legacy_root_projection_entries(journal.entries(), event_runs)
}

fn plan_legacy_root_projection_entries(
    entries: impl IntoIterator<Item = RecordingSessionJournalEntry>,
    event_runs: &[SealedLegacyProjectionRun],
) -> Result<LegacyRootProjectionPlan, CutError> {
    let journal = RecordingSessionJournal::replay(entries).map_err(|error| {
        invalid_projection(format!(
            "recording session journal failed replay validation: {error}"
        ))
    })?;
    validate_completed_session(&journal)?;
    let event_runs = validate_event_run_identity(&journal, event_runs)?;
    let source_stitch = plan_run_aware_stitch(&journal).map_err(|error| {
        invalid_projection(format!(
            "could not plan compact screen source stitch: {error}"
        ))
    })?;
    validate_compact_source_plan(&journal, &source_stitch)?;
    let merged_events = merge_sealed_event_tracks(&event_runs).map_err(|error| {
        invalid_projection(format!(
            "could not merge compact sealed event tracks: {error}"
        ))
    })?;

    Ok(LegacyRootProjectionPlan {
        source_output: LEGACY_ROOT_SOURCE_OUTPUT,
        events_output: LEGACY_ROOT_EVENTS_OUTPUT,
        source_stitch,
        merged_events,
        capture_cadence: journal.intent().capture_cadence.clone(),
    })
}

fn validate_completed_session(journal: &RecordingSessionJournal) -> Result<(), CutError> {
    match journal.terminal() {
        Some(terminal) if terminal.disposition == TerminalDisposition::Completed => {}
        Some(terminal) => {
            return Err(invalid_projection(format!(
                "legacy root projection only supports completed sessions, not {:?}",
                terminal.disposition
            )))
        }
        None => {
            return Err(invalid_projection(
                "legacy root projection requires a terminal completed session",
            ))
        }
    }
    if journal.sealed_runs().is_empty() {
        return Err(invalid_projection(
            "legacy root projection requires at least one sealed recording run",
        ));
    }
    Ok(())
}

fn validate_event_run_identity(
    journal: &RecordingSessionJournal,
    event_runs: &[SealedLegacyProjectionRun],
) -> Result<Vec<SealedEventTrackRun>, CutError> {
    let journal_runs = journal.sealed_runs();
    if event_runs.len() != journal_runs.len() {
        return Err(invalid_projection(format!(
            "legacy root projection requires exactly one event input for each sealed run (expected {}, got {})",
            journal_runs.len(),
            event_runs.len()
        )));
    }

    journal_runs
        .iter()
        .zip(event_runs)
        .enumerate()
        .map(|(index, (journal_run, event_run))| {
            if event_run.run_sequence != journal_run.sequence {
                return Err(invalid_projection(format!(
                    "event input {index} names run {} but journal requires run {}",
                    event_run.run_sequence, journal_run.sequence
                )));
            }
            if event_run.logical_start_ms != journal_run.logical_start_ms
                || event_run.logical_end_ms != journal_run.logical_end_ms
            {
                return Err(invalid_projection(format!(
                    "event input {index} compact interval does not match journal run {}",
                    journal_run.sequence
                )));
            }
            let expected_duration = journal_run
                .logical_end_ms
                .checked_sub(journal_run.logical_start_ms)
                .ok_or_else(|| {
                    invalid_projection(format!(
                        "journal run {} has an inverted compact interval",
                        journal_run.sequence
                    ))
                })?;
            if event_run.events.duration_ms != expected_duration {
                return Err(invalid_projection(format!(
                    "event input {index} duration does not match journal run {} compact interval",
                    journal_run.sequence
                )));
            }
            // ARCH-TIME-01: `Settings.fps` is the legacy f32 render timebase.
            // It cannot prove or reject fractional request intent; optional
            // CaptureCadence@1 stays on the journal/projection metadata path.
            if !journal.intent().record_keys && !event_run.events.keys.is_empty() {
                return Err(invalid_projection(format!(
                    "event input {index} contains key events although immutable journal intent disabled them"
                )));
            }
            Ok(SealedEventTrackRun {
                logical_offset_ms: event_run.logical_start_ms,
                settings: event_run.settings,
                events: event_run.events.clone(),
            })
        })
        .collect()
}

fn validate_compact_source_plan(
    journal: &RecordingSessionJournal,
    plan: &RunAwareStitchPlan,
) -> Result<(), CutError> {
    let terminal_end = journal
        .terminal()
        .ok_or_else(|| invalid_projection("compact source plan requires a terminal session"))?
        .logical_end_ms;
    if plan.duration_ms != terminal_end {
        return Err(invalid_projection(
            "compact source plan duration does not match completed session duration",
        ));
    }

    for span in &plan.spans {
        let (run_sequence, start, end) = match span {
            RunAwareStitchSpan::Source {
                run_sequence,
                logical_offset_ms,
                source_duration_ms,
                ..
            } => (
                *run_sequence,
                *logical_offset_ms,
                logical_offset_ms
                    .checked_add(*source_duration_ms)
                    .ok_or_else(|| {
                        invalid_projection("source span duration overflows the compact timeline")
                    })?,
            ),
            RunAwareStitchSpan::EncoderGapPadding {
                run_sequence,
                logical_start_ms,
                logical_end_ms,
            } => (*run_sequence, *logical_start_ms, *logical_end_ms),
        };
        let Some(run) = journal
            .sealed_runs()
            .iter()
            .find(|run| run.sequence == run_sequence)
        else {
            return Err(invalid_projection(
                "compact source plan references an unknown sealed run",
            ));
        };
        if start < run.logical_start_ms || end > run.logical_end_ms || end <= start {
            return Err(invalid_projection(
                "compact source plan contains a span outside one sealed run",
            ));
        }
    }
    Ok(())
}

fn invalid_projection(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot project pause-aware recording to legacy root outputs",
        cause,
    )
    .with_suggested_action(
        "complete every sealed run with matching compact event facts before legacy projection",
    )
}

#[cfg(test)]
#[path = "pause_projection_tests.rs"]
mod tests;
