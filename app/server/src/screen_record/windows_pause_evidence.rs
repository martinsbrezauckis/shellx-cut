//! Private calibrated Windows pause evidence; raw ticks are provenance only.
use super::pause_projection::SealedLegacyProjectionRun;
use super::run_seal_coordinator::{SealedRunEvidence, SessionTimeOrigin};
use super::windows_pause_adapter::{WindowsPauseAdapterError, WindowsPauseEvidenceFactory};
#[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_imports))]
pub(crate) use super::windows_pause_evidence_artifacts::LocalWindowsPauseArtifactVerifier;
use super::windows_pause_evidence_artifacts::WindowsPauseArtifactVerifier;
use record_capture::windows_pause_pilot::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotStarted, WindowsSealedAudioRun,
    WindowsSealedScreenRun,
};
use record_core::{CursorCorrelation, EventTrack};
use record_recovery::{CheckpointSequenceRange, RecordingStream, SealedRun};
use std::time::Instant;

mod fragments;
mod input;
mod validation;
use fragments::fragments;
pub(crate) use input::{RecordingAudioDraft, RecordingInputDraft};
use validation::{raw_span, validate_accepted, validate_audio, validate_run};
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
    exact_monitor_id: Option<String>,
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
            exact_monitor_id: None,
        }
    }

    /// Production supplies the pre-admitted opaque monitor identity. Tests
    /// without that owner intentionally cannot fabricate sidecar evidence.
    pub(crate) fn for_exact_monitor(artifacts: V, exact_monitor_id: String) -> Self {
        let mut factory = Self::new(artifacts);
        factory.exact_monitor_id = Some(exact_monitor_id);
        factory
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
        self.build(generation, native, &[], post_close_observed_at, true)
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
        self.build(generation, native, &[], post_close_observed_at, false)
    }

    fn verify_and_build_with_audio(
        &mut self,
        generation: u64,
        native: &WindowsSealedScreenRun,
        audio: &[WindowsSealedAudioRun],
        post_close_observed_at: Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        self.build(generation, native, audio, post_close_observed_at, false)
    }
}

impl<V: WindowsPauseArtifactVerifier> CalibratedWindowsPauseEvidenceFactory<V> {
    fn build(
        &mut self,
        generation: u64,
        native: &WindowsSealedScreenRun,
        native_audio: &[WindowsSealedAudioRun],
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
        let audio = validate_audio(native, native_audio, &self.artifacts)?;
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
        let fragments = fragments(native, logical_span, &audio)?;
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
        let mut sealed_streams = vec![RecordingStream::ScreenVideo];
        sealed_streams.extend(audio.iter().map(RecordingAudioDraft::stream));
        sealed_streams.sort_unstable();
        let mut evidence = SealedRunEvidence::new(
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
            sealed_streams,
        );
        if let Some(display_id) = self.exact_monitor_id.as_ref() {
            evidence = evidence.with_recording_input(RecordingInputDraft::from_native(
                display_id.clone(),
                native.accepted.range.origin_x,
                native.accepted.range.origin_y,
                native.accepted.range.width,
                native.accepted.range.height,
                native.accepted.settings.width,
                native.accepted.settings.height,
                native.accepted.settings.fps as u32,
                staged.unix_ms,
                staged.raw_start_ms,
                native.observed_start_ms,
                native.observed_end_ms,
                audio,
            ));
        }
        self.staged = None;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        self.next_logical_start_ms = logical_end_ms;
        Ok(evidence)
    }
}

#[cfg(test)]
#[path = "windows_pause_evidence_tests.rs"]
mod tests;
