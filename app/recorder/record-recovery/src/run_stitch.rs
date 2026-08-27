//! Pure planning for pause-aware source stitching.
//!
//! v1 `stitch_complete` remains the executable checkpoint stitcher. This model
//! only describes how a future session-aware stitcher must preserve encoder gaps
//! inside one sealed run while omitting wall-clock pauses between sealed runs.

use crate::{RecordingSessionJournal, RecordingStream, SessionJournalError, StreamFragment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAwareStitchPlan {
    pub duration_ms: u64,
    pub spans: Vec<RunAwareStitchSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAwareStitchSpan {
    Source {
        run_sequence: u64,
        checkpoint_sequence: u64,
        artifact: String,
        sha256: String,
        logical_offset_ms: u64,
        source_duration_ms: u64,
    },
    EncoderGapPadding {
        run_sequence: u64,
        logical_start_ms: u64,
        logical_end_ms: u64,
    },
}

/// Plan compact source-timeline stitching from a sealed terminal journal.
///
/// The journal has already rejected malformed or overlapping facts. This method
/// nevertheless replays its entries first, preserving fail-closed behavior if a
/// caller receives a forged in-memory value through future deserialization code.
pub fn plan_run_aware_stitch(
    journal: &RecordingSessionJournal,
) -> Result<RunAwareStitchPlan, SessionJournalError> {
    let journal = RecordingSessionJournal::replay(journal.entries())?;
    if journal.terminal().is_none() {
        return Err(SessionJournalError::Invalid(
            "run-aware stitching requires a terminal disposition".into(),
        ));
    }
    let mut spans = Vec::new();
    for run in journal.sealed_runs() {
        let mut cursor = run.logical_start_ms;
        let video = run
            .fragments
            .iter()
            .filter(|fragment| fragment.stream == RecordingStream::ScreenVideo)
            .collect::<Vec<_>>();
        if video.is_empty() {
            return Err(SessionJournalError::Invalid(
                "sealed run has no video fragments to stitch".into(),
            ));
        }
        for fragment in video {
            let start = run.logical_start_ms + fragment.facts.start_offset_ms;
            let end = run.logical_start_ms + fragment.facts.end_offset_ms;
            append_padding(&mut spans, run.sequence, cursor, start);
            append_source(&mut spans, run.sequence, fragment, start)?;
            let source_end = start + fragment.facts.media_duration_ms;
            append_padding(&mut spans, run.sequence, source_end, end);
            cursor = end;
        }
        append_padding(&mut spans, run.sequence, cursor, run.logical_end_ms);
    }
    let duration_ms = journal
        .sealed_runs()
        .last()
        .map(|run| run.logical_end_ms)
        .unwrap_or(0);
    Ok(RunAwareStitchPlan { duration_ms, spans })
}

fn append_source(
    spans: &mut Vec<RunAwareStitchSpan>,
    run_sequence: u64,
    fragment: &StreamFragment,
    logical_offset_ms: u64,
) -> Result<(), SessionJournalError> {
    let checkpoint_sequence = fragment.checkpoint_sequence.ok_or_else(|| {
        SessionJournalError::Invalid("screen video fragment lacks checkpoint sequence".into())
    })?;
    spans.push(RunAwareStitchSpan::Source {
        run_sequence,
        checkpoint_sequence,
        artifact: fragment.artifact.clone(),
        sha256: fragment.sha256.clone(),
        logical_offset_ms,
        source_duration_ms: fragment.facts.media_duration_ms,
    });
    Ok(())
}

fn append_padding(
    spans: &mut Vec<RunAwareStitchSpan>,
    run_sequence: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
) {
    if logical_end_ms > logical_start_ms {
        spans.push(RunAwareStitchSpan::EncoderGapPadding {
            run_sequence,
            logical_start_ms,
            logical_end_ms,
        });
    }
}
