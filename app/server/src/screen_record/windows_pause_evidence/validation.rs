use super::{RecordingAudioDraft, WindowsPauseArtifactVerifier};
use crate::screen_record::windows_pause_adapter::WindowsPauseAdapterError;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotAcceptedCapture, WindowsSealedAudioRun, WindowsSealedScreenRun,
};
use record_core::Settings;
use record_recovery::RecordingStream;

pub(super) fn validate_accepted(
    accepted: &WindowsPausePilotAcceptedCapture,
) -> Result<(), WindowsPauseAdapterError> {
    let Settings {
        width,
        height,
        fps,
        audio_rate,
    } = accepted.settings;
    if width == 0
        || height == 0
        || audio_rate == 0
        || accepted.range.width != width
        || accepted.range.height != height
        || !fps.is_finite()
        || !(1.0..=240.0).contains(&fps)
        || fps.fract() != 0.0
    {
        return Err(WindowsPauseAdapterError::EvidenceRejected);
    }
    Ok(())
}

pub(super) fn validate_run<V: WindowsPauseArtifactVerifier>(
    native: &WindowsSealedScreenRun,
    artifacts: &V,
) -> Result<(), WindowsPauseAdapterError> {
    validate_accepted(&native.accepted)?;
    let first = native
        .checkpoints
        .first()
        .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
    let last = native
        .checkpoints
        .last()
        .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
    if raw_span(native)? == 0
        || first.start_ms != native.observed_start_ms
        || last.end_ms != native.observed_end_ms
        || first.physical_generation != native.range.first_physical_generation
        || last.physical_generation != native.range.last_physical_generation
        || first.checkpoint.sequence != native.range.first_checkpoint_sequence
        || last.checkpoint.sequence != native.range.last_checkpoint_sequence
    {
        return Err(WindowsPauseAdapterError::EvidenceRejected);
    }
    for (index, checkpoint) in native.checkpoints.iter().enumerate() {
        let expected_physical = native
            .range
            .first_physical_generation
            .checked_add(index as u64)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let expected_sequence = native
            .range
            .first_checkpoint_sequence
            .checked_add(index as u64)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let media = checkpoint
            .checkpoint
            .media
            .as_ref()
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        if checkpoint.physical_generation != expected_physical
            || checkpoint.checkpoint.sequence != expected_sequence
            || checkpoint.start_ms >= checkpoint.end_ms
            || checkpoint.checkpoint.facts.start_ms != checkpoint.start_ms
            || checkpoint.checkpoint.facts.end_ms != checkpoint.end_ms
            || checkpoint.checkpoint.facts.event_offset_ms != checkpoint.start_ms
            || checkpoint.checkpoint.facts.audio_offset_ms.is_some()
            || media.has_audio
            || media.duration_ms == 0
            || media.duration_ms > checkpoint.end_ms - checkpoint.start_ms
            || media.decoded_video_frames == 0
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        if index > 0 && checkpoint.start_ms < native.checkpoints[index - 1].end_ms {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        artifacts.verify(&checkpoint.checkpoint)?;
    }
    Ok(())
}

pub(super) fn validate_audio<V: WindowsPauseArtifactVerifier>(
    screen: &WindowsSealedScreenRun,
    audio: &[WindowsSealedAudioRun],
    artifacts: &V,
) -> Result<Vec<RecordingAudioDraft>, WindowsPauseAdapterError> {
    let mut drafts = Vec::with_capacity(audio.len());
    for item in audio {
        if !matches!(
            item.stream,
            RecordingStream::MicrophoneAudio | RecordingStream::SystemAudio
        ) || item.source_generation != screen.range.first_physical_generation
            || item.raw_start_ms != screen.observed_start_ms
            || item.raw_end_ms != screen.observed_end_ms
            || item.native_ready_raw_ms < item.raw_start_ms
            || item.native_ready_raw_ms > item.raw_end_ms
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        artifacts.verify_audio(item)?;
        drafts.push(
            RecordingAudioDraft::from_native(item)
                .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?,
        );
    }
    drafts.sort_by_key(RecordingAudioDraft::stream);
    if drafts
        .windows(2)
        .any(|items| items[0].stream() == items[1].stream())
    {
        return Err(WindowsPauseAdapterError::EvidenceRejected);
    }
    Ok(drafts)
}

pub(super) fn raw_span(native: &WindowsSealedScreenRun) -> Result<u64, WindowsPauseAdapterError> {
    native
        .observed_end_ms
        .checked_sub(native.observed_start_ms)
        .ok_or(WindowsPauseAdapterError::EvidenceRejected)
}
