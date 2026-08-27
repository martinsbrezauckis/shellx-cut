//! Private calibrated Windows pause evidence; raw ticks are provenance only.
use super::pause_projection::SealedLegacyProjectionRun;
use super::run_seal_coordinator::{SealedRunEvidence, SessionTimeOrigin};
use super::windows_pause_adapter::{WindowsPauseAdapterError, WindowsPauseEvidenceFactory};
#[cfg(windows)]
pub(crate) use super::windows_pause_evidence_artifacts::LocalWindowsPauseArtifactVerifier;
use super::windows_pause_evidence_artifacts::WindowsPauseArtifactVerifier;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotStarted, WindowsSealedScreenRun,
};
use record_core::{CursorCorrelation, EventTrack, Settings};
use record_recovery::{
    CheckpointSequenceRange, RecordingStream, SealedRun, StreamFragment, StreamFragmentFacts,
};
use std::time::Instant;
#[derive(Clone)]
struct RawStartCalibration {
    server_generation: Option<u64>,
    raw_start_ms: u64,
    monotonic_at: Instant,
    unix_ms: u64,
    accepted: WindowsPausePilotAcceptedCapture,
}
pub(crate) struct CalibratedWindowsPauseEvidenceFactory<V> {
    artifacts: V,
    origin: Option<SessionTimeOrigin>,
    baseline: Option<WindowsPausePilotAcceptedCapture>,
    staged: Option<RawStartCalibration>,
    next_sequence: u64,
    next_logical_start_ms: u64,
}

impl<V> CalibratedWindowsPauseEvidenceFactory<V> {
    pub(crate) fn new(artifacts: V) -> Self {
        Self {
            artifacts,
            origin: None,
            baseline: None,
            staged: None,
            next_sequence: 0,
            next_logical_start_ms: 0,
        }
    }
}

impl<V: WindowsPauseArtifactVerifier> WindowsPauseEvidenceFactory
    for CalibratedWindowsPauseEvidenceFactory<V>
{
    fn stage_started(
        &mut self,
        server_generation: Option<u64>,
        started: &WindowsPausePilotStarted,
    ) -> Result<(), WindowsPauseAdapterError> {
        validate_accepted(&started.accepted)?;
        if self.staged.is_some()
            || self
                .baseline
                .as_ref()
                .is_some_and(|baseline| baseline != &started.accepted)
        {
            return Err(WindowsPauseAdapterError::CalibrationRejected);
        }
        if self.baseline.is_none() {
            self.baseline = Some(started.accepted.clone());
        }
        self.staged = Some(RawStartCalibration {
            server_generation,
            raw_start_ms: started.observed_start_ms,
            monotonic_at: started.monotonic_at,
            unix_ms: started.unix_ms,
            accepted: started.accepted.clone(),
        });
        Ok(())
    }

    fn discard_staged(&mut self) {
        self.staged = None;
    }

    fn verify_discarded_stop(
        &mut self,
        native: &WindowsSealedScreenRun,
        post_close_observed_at: Instant,
    ) -> Result<(), WindowsPauseAdapterError> {
        let generation = self
            .staged
            .as_ref()
            .and_then(|staged| staged.server_generation)
            .unwrap_or(0);
        self.build(generation, native, post_close_observed_at, true)
            .map(|_| ())
    }

    fn set_session_origin(
        &mut self,
        origin: SessionTimeOrigin,
    ) -> Result<(), WindowsPauseAdapterError> {
        let Some(staged) = self.staged.as_ref() else {
            return Err(WindowsPauseAdapterError::CalibrationRejected);
        };
        if self.origin.is_some() || !origin.matches(staged.monotonic_at, staged.unix_ms) {
            return Err(WindowsPauseAdapterError::CalibrationRejected);
        }
        self.origin = Some(origin);
        Ok(())
    }

    fn verify_and_build(
        &mut self,
        generation: u64,
        native: &WindowsSealedScreenRun,
        post_close_observed_at: Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        self.build(generation, native, post_close_observed_at, false)
    }
}

impl<V: WindowsPauseArtifactVerifier> CalibratedWindowsPauseEvidenceFactory<V> {
    fn build(
        &mut self,
        generation: u64,
        native: &WindowsSealedScreenRun,
        post_close_observed_at: Instant,
        discarded: bool,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        let origin = self
            .origin
            .ok_or(WindowsPauseAdapterError::CalibrationRejected)?;
        let staged = self
            .staged
            .as_ref()
            .ok_or(WindowsPauseAdapterError::CalibrationRejected)?;
        let expected_generation = (self.next_sequence != 0).then_some(generation);
        if (!discarded && staged.server_generation != expected_generation)
            || native.observed_start_ms != staged.raw_start_ms
            || native.accepted != staged.accepted
        {
            return Err(WindowsPauseAdapterError::CalibrationRejected);
        }
        validate_run(native, &self.artifacts)?;
        let observed_start_ms = if self.next_sequence == 0 {
            if !origin.matches(staged.monotonic_at, staged.unix_ms) {
                return Err(WindowsPauseAdapterError::CalibrationRejected);
            }
            0
        } else {
            origin
                .elapsed_ms(staged.monotonic_at)
                .map_err(|_| WindowsPauseAdapterError::CalibrationRejected)?
        };
        let observed_end_ms = origin
            .elapsed_ms(post_close_observed_at)
            .map_err(|_| WindowsPauseAdapterError::EvidenceRejected)?;
        let observed_span = observed_end_ms
            .checked_sub(observed_start_ms)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let logical_span = raw_span(native)?;
        if logical_span == 0 || logical_span > observed_span {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let logical_end_ms = self
            .next_logical_start_ms
            .checked_add(logical_span)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let sequence = self.next_sequence;
        let fragments = fragments(native, logical_span)?;
        let events = EventTrack {
            duration_ms: logical_span,
            screen_w: native.accepted.settings.width,
            screen_h: native.accepted.settings.height,
            monitors: Vec::new(),
            cursor: Vec::new(),
            clicks: Vec::new(),
            scrolls: Vec::new(),
            keys: Vec::new(),
            cursor_correlation: CursorCorrelation::default(),
        };
        let evidence = SealedRunEvidence::new(
            generation,
            SealedRun {
                sequence,
                observed_start_ms,
                observed_end_ms,
                logical_start_ms: self.next_logical_start_ms,
                logical_end_ms,
                checkpoints: CheckpointSequenceRange {
                    first: native.range.first_checkpoint_sequence,
                    last: native.range.last_checkpoint_sequence,
                },
                fragments,
            },
            SealedLegacyProjectionRun::new(
                sequence,
                self.next_logical_start_ms,
                logical_end_ms,
                native.accepted.settings,
                events,
            ),
            vec![RecordingStream::ScreenVideo],
        );
        self.staged = None;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        self.next_logical_start_ms = logical_end_ms;
        Ok(evidence)
    }
}

fn validate_accepted(
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

fn validate_run<V: WindowsPauseArtifactVerifier>(
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

fn raw_span(native: &WindowsSealedScreenRun) -> Result<u64, WindowsPauseAdapterError> {
    native
        .observed_end_ms
        .checked_sub(native.observed_start_ms)
        .ok_or(WindowsPauseAdapterError::EvidenceRejected)
}

fn fragments(
    native: &WindowsSealedScreenRun,
    logical_span: u64,
) -> Result<Vec<StreamFragment>, WindowsPauseAdapterError> {
    native
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
        .collect()
}

#[cfg(test)]
#[path = "windows_pause_evidence_tests.rs"]
mod tests;
