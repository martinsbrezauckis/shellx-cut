use super::super::windows_pause_adapter::WindowsPauseAdapterError;
use super::input::RecordingAudioDraft;
use record_capture::windows_pause_pilot::WindowsSealedScreenRun;
use record_recovery::{RecordingStream, StreamFragment, StreamFragmentFacts};

pub(super) fn fragments(
    native: &WindowsSealedScreenRun,
    logical_span: u64,
    audio: &[RecordingAudioDraft],
) -> Result<Vec<StreamFragment>, WindowsPauseAdapterError> {
    let mut fragments: Vec<StreamFragment> = native
        .checkpoints
        .iter()
        .enumerate()
        .map(|(index, checkpoint)| {
            let start_offset_ms = checkpoint
                .start_ms
                .checked_sub(native.observed_start_ms)
                .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
            let end_offset_ms = checkpoint
                .end_ms
                .checked_sub(native.observed_start_ms)
                .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
            let media = checkpoint
                .checkpoint
                .media
                .as_ref()
                .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
            if end_offset_ms > logical_span {
                return Err(WindowsPauseAdapterError::EvidenceRejected);
            }
            Ok(StreamFragment {
                stream: RecordingStream::ScreenVideo,
                checkpoint_sequence: Some(checkpoint.checkpoint.sequence),
                stream_sequence: index as u64,
                artifact: checkpoint.checkpoint.file.clone(),
                bytes: checkpoint.checkpoint.bytes,
                sha256: checkpoint.checkpoint.sha256.clone(),
                facts: StreamFragmentFacts {
                    start_offset_ms,
                    end_offset_ms,
                    media_duration_ms: media.duration_ms,
                    decoded_video_frames: Some(media.decoded_video_frames),
                    avg_frame_rate: media.avg_frame_rate,
                    r_frame_rate: media.r_frame_rate,
                },
            })
        })
        .collect::<Result<_, _>>()?;
    for source in audio {
        let start_offset_ms = source
            .raw_start_ms()
            .checked_sub(native.observed_start_ms)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let end_offset_ms = source
            .raw_end_ms()
            .checked_sub(native.observed_start_ms)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        if end_offset_ms > logical_span
            || end_offset_ms <= start_offset_ms
            || source.media_duration_ms() > end_offset_ms - start_offset_ms
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        fragments.push(StreamFragment {
            stream: source.stream(),
            checkpoint_sequence: None,
            stream_sequence: 0,
            artifact: source.artifact().into(),
            bytes: source.bytes(),
            sha256: source.sha256().into(),
            facts: StreamFragmentFacts {
                start_offset_ms,
                end_offset_ms,
                media_duration_ms: source.media_duration_ms(),
                decoded_video_frames: None,
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        });
    }
    fragments.sort_by_key(|fragment| (fragment.stream, fragment.stream_sequence));
    Ok(fragments)
}
