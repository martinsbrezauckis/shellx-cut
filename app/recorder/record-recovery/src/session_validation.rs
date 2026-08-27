//! Fail-closed validation for the recording-session journal contract.

use std::collections::{BTreeMap, BTreeSet};

use record_core::{backend_requested_v1, CaptureCadence, FrameRate, CAPTURE_CADENCE_SCHEMA};

use crate::session_contract::{
    RecordingSessionIntent, RecordingSessionState, RecordingStream, SealedRun, SessionJournalError,
    StreamFragment, RECORDING_SESSION_JOURNAL_SCHEMA,
};

pub(crate) fn validate_intent(intent: &RecordingSessionIntent) -> Result<(), SessionJournalError> {
    if intent.schema != RECORDING_SESSION_JOURNAL_SCHEMA
        || !safe_identifier(&intent.session_id)
        || intent.checkpoint_interval_ms == 0
        || !intent.fps.is_finite()
        || !(1.0..=240.0).contains(&intent.fps)
        || intent.active_duration_limit_ms == Some(0)
        || !safe_target_descriptor(&intent.target_descriptor)
        || intent.requested_streams.is_empty()
        || !intent
            .requested_streams
            .contains(&RecordingStream::ScreenVideo)
        || !strictly_sorted(&intent.requested_streams)
    {
        return Err(invalid("malformed immutable intent"));
    }
    if let Some(cadence) = &intent.capture_cadence {
        validate_capture_cadence(intent.fps, cadence)?;
    }
    Ok(())
}

/// Cadence evidence is optional for old journals, but once present it must be
/// an exact, self-consistent rendering of the immutable decimal request. This
/// validates request policy only; completed fragment probe facts stay separate.
fn validate_capture_cadence(fps: f64, cadence: &CaptureCadence) -> Result<(), SessionJournalError> {
    let requested = FrameRate::from_server_decimal(fps)
        .map_err(|_| invalid("immutable intent FPS cannot form exact cadence evidence"))?;
    if cadence.schema != CAPTURE_CADENCE_SCHEMA
        || cadence.requested != requested
        || cadence.backend_requested != backend_requested_v1(requested)
        || cadence.probed_media.is_some()
    {
        return Err(invalid(
            "capture cadence does not match immutable request or backend policy",
        ));
    }
    Ok(())
}

pub(crate) fn validate_run(
    intent: &RecordingSessionIntent,
    previous: Option<&SealedRun>,
    run: &SealedRun,
) -> Result<(), SessionJournalError> {
    let expected_sequence = next_run_sequence(previous)?;
    let expected_logical_start = previous.map_or(0, |previous| previous.logical_end_ms);
    let expected_checkpoint_first = next_checkpoint_sequence(previous)?;
    if run.sequence != expected_sequence
        || run.logical_start_ms != expected_logical_start
        || run.logical_end_ms <= run.logical_start_ms
        || run.observed_end_ms <= run.observed_start_ms
        || run.checkpoints.first != expected_checkpoint_first
        || run.checkpoints.last < run.checkpoints.first
        || previous.is_some_and(|previous| run.observed_start_ms < previous.observed_end_ms)
        || intent
            .active_duration_limit_ms
            .is_some_and(|limit| run.logical_end_ms > limit)
        || run.fragments.is_empty()
    {
        return Err(invalid("sealed run ordering or intervals are invalid"));
    }

    let run_span = run.logical_end_ms - run.logical_start_ms;
    let mut expected_stream_sequences = BTreeMap::new();
    let mut expected_screen_checkpoint = run.checkpoints.first;
    let mut artifacts = BTreeSet::new();
    let mut previous_fragment: Option<&StreamFragment> = None;
    for fragment in &run.fragments {
        validate_fragment_shape(intent, run_span, fragment)?;
        validate_fragment_order(
            fragment,
            previous_fragment,
            &mut expected_stream_sequences,
            &mut expected_screen_checkpoint,
        )?;
        if !artifacts.insert(&fragment.artifact) {
            return Err(invalid("stream fragments reuse an artifact path"));
        }
        validate_cross_stream_interval(run, fragment)?;
        previous_fragment = Some(fragment);
    }
    if expected_screen_checkpoint != checked_successor(run.checkpoints.last, "checkpoint sequence")?
    {
        return Err(invalid(
            "screen video does not cover the checkpoint sequence range",
        ));
    }
    Ok(())
}

pub(crate) fn transition_allowed(
    previous: Option<RecordingSessionState>,
    next: RecordingSessionState,
) -> bool {
    matches!(
        (previous, next),
        (None, RecordingSessionState::Started)
            | (
                Some(RecordingSessionState::Started),
                RecordingSessionState::Paused
            )
            | (
                Some(RecordingSessionState::Started),
                RecordingSessionState::Stopping
            )
            | (
                Some(RecordingSessionState::Paused),
                RecordingSessionState::Resumed
            )
            | (
                Some(RecordingSessionState::Paused),
                RecordingSessionState::Stopping
            )
            | (
                Some(RecordingSessionState::Resumed),
                RecordingSessionState::Paused
            )
            | (
                Some(RecordingSessionState::Resumed),
                RecordingSessionState::Stopping
            )
    )
}

fn validate_fragment_shape(
    intent: &RecordingSessionIntent,
    run_span: u64,
    fragment: &StreamFragment,
) -> Result<(), SessionJournalError> {
    let facts = &fragment.facts;
    if !intent.requested_streams.contains(&fragment.stream)
        || !safe_relative_artifact(&fragment.artifact)
        || fragment.bytes == 0
        || !valid_sha256(&fragment.sha256)
        || facts.end_offset_ms <= facts.start_offset_ms
        || facts.end_offset_ms > run_span
        || facts.media_duration_ms == 0
        || facts.media_duration_ms > facts.end_offset_ms - facts.start_offset_ms
        || (fragment.stream.carries_video_frames()
            && facts.decoded_video_frames.is_none_or(|frames| frames == 0))
        || (!fragment.stream.carries_video_frames() && facts.decoded_video_frames.is_some())
        || (!fragment.stream.carries_video_frames()
            && (facts.avg_frame_rate.is_some() || facts.r_frame_rate.is_some()))
        || (fragment.stream == RecordingStream::ScreenVideo
            && fragment.checkpoint_sequence.is_none())
        || (fragment.stream != RecordingStream::ScreenVideo
            && fragment.checkpoint_sequence.is_some())
    {
        return Err(invalid("malformed stream fragment facts"));
    }
    Ok(())
}

fn validate_fragment_order(
    fragment: &StreamFragment,
    previous: Option<&StreamFragment>,
    expected_stream_sequences: &mut BTreeMap<RecordingStream, u64>,
    expected_screen_checkpoint: &mut u64,
) -> Result<(), SessionJournalError> {
    let expected_stream_sequence = expected_stream_sequences
        .get(&fragment.stream)
        .copied()
        .unwrap_or(0);
    if fragment.stream_sequence != expected_stream_sequence {
        return Err(invalid("stream fragment sequence is not contiguous"));
    }
    expected_stream_sequences.insert(
        fragment.stream,
        checked_successor(fragment.stream_sequence, "stream fragment sequence")?,
    );
    if let Some(previous) = previous {
        if fragment.stream < previous.stream
            || (fragment.stream == previous.stream
                && fragment.facts.start_offset_ms < previous.facts.end_offset_ms)
        {
            return Err(invalid("stream fragment facts are out of order or overlap"));
        }
    }
    if fragment.stream == RecordingStream::ScreenVideo {
        if fragment.checkpoint_sequence != Some(*expected_screen_checkpoint) {
            return Err(invalid(
                "screen video checkpoint sequence is not contiguous",
            ));
        }
        *expected_screen_checkpoint =
            checked_successor(*expected_screen_checkpoint, "checkpoint sequence")?;
    }
    Ok(())
}

/// Every stream may overlap another stream (for example, audio under video), but
/// each declared interval must map safely into the same sealed logical run.
fn validate_cross_stream_interval(
    run: &SealedRun,
    fragment: &StreamFragment,
) -> Result<(), SessionJournalError> {
    let start = run
        .logical_start_ms
        .checked_add(fragment.facts.start_offset_ms)
        .ok_or_else(|| invalid("stream fragment logical start overflows"))?;
    let end = run
        .logical_start_ms
        .checked_add(fragment.facts.end_offset_ms)
        .ok_or_else(|| invalid("stream fragment logical end overflows"))?;
    if start < run.logical_start_ms || end > run.logical_end_ms || end <= start {
        return Err(invalid(
            "stream fragment falls outside the sealed run interval",
        ));
    }
    Ok(())
}

fn next_run_sequence(previous: Option<&SealedRun>) -> Result<u64, SessionJournalError> {
    previous
        .map(|previous| checked_successor(previous.sequence, "run sequence"))
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn next_checkpoint_sequence(previous: Option<&SealedRun>) -> Result<u64, SessionJournalError> {
    previous
        .map(|previous| checked_successor(previous.checkpoints.last, "checkpoint sequence"))
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn checked_successor(value: u64, name: &str) -> Result<u64, SessionJournalError> {
    value
        .checked_add(1)
        .ok_or_else(|| invalid(format!("{name} overflow")))
}

fn strictly_sorted(values: &[RecordingStream]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn safe_target_descriptor(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains(['/', '\\'])
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
}

fn safe_relative_artifact(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.contains(['\\', ':'])
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.as_bytes().contains(&0)
                && !segment.bytes().any(|byte| byte.is_ascii_control())
        })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn invalid(detail: impl Into<String>) -> SessionJournalError {
    SessionJournalError::Invalid(detail.into())
}
