//! Private server ownership for one Timeline voiceover take: it binds an accepted
//! target before native capture and emits Preview intents instead of playback or placement.

use cut_core::{error_codes, Actor, CutError, Project};
use record_capture::{
    MicrophoneSource, VoiceoverArtifact, VoiceoverCaptureOutcome, VoiceoverCaptureSession,
    VoiceoverFinalize,
};
use std::path::Path;
use std::time::{Duration, Instant};

mod admission;
mod fingerprint;
mod journal;
mod journal_io;
mod lifecycle;
mod materialization;
mod ownership;
mod placement;
mod session;
mod session_recovery;
pub(crate) use admission::{
    admit, AcceptedVoiceoverTimeline, VoiceoverCue, VoiceoverTimelineRequest,
};
use journal::{VoiceoverTakePhase, VoiceoverTerminalIntent};
use lifecycle::{
    claim_preview_bridge, no_active_take, phase_after_finalize, project_status,
    record_terminal_intent, sync_durable, update_terminal, validate_preview_ack,
};
use materialization::sealed_materialization;
pub(crate) use ownership::VoiceoverOwnerClaim;
use ownership::VoiceoverOwnerSession;
use session::{VoiceoverPreviewBridgeClaim, VoiceoverTakePreparation, VoiceoverTakeSession};
const DEFAULT_COUNTDOWN: Duration = Duration::from_secs(3);

pub(crate) trait VoiceoverCaptureControl: Send {
    fn microphone_ready(&self) -> bool;
    fn begin_recording(&mut self) -> Result<(), CutError>;
    fn stop(&mut self) -> VoiceoverFinalize;
    fn cancel(&mut self) -> VoiceoverFinalize;
    fn status(&mut self) -> VoiceoverFinalize;
}

impl VoiceoverCaptureControl for VoiceoverCaptureSession {
    fn microphone_ready(&self) -> bool {
        VoiceoverCaptureSession::microphone_ready(self)
    }

    fn begin_recording(&mut self) -> Result<(), CutError> {
        VoiceoverCaptureSession::begin_recording(self).map_err(crate::screen_record::record_err)
    }

    fn stop(&mut self) -> VoiceoverFinalize {
        VoiceoverCaptureSession::stop(self)
    }

    fn cancel(&mut self) -> VoiceoverFinalize {
        VoiceoverCaptureSession::cancel(self)
    }

    fn status(&mut self) -> VoiceoverFinalize {
        VoiceoverCaptureSession::status(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoiceoverTimelineStatus {
    AwaitingMicrophone {
        take: AcceptedVoiceoverTimeline,
    },
    Countdown {
        take: AcceptedVoiceoverTimeline,
        remaining_ms: u64,
    },
    StartProgramPlayback {
        take: AcceptedVoiceoverTimeline,
    },
    Recording {
        take: AcceptedVoiceoverTimeline,
    },
    Finalizing {
        take: AcceptedVoiceoverTimeline,
    },
    Finished {
        take: AcceptedVoiceoverTimeline,
        outcome: VoiceoverCaptureOutcome,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceoverMaterialization {
    pub(crate) take: AcceptedVoiceoverTimeline,
    pub(crate) artifact: VoiceoverArtifact,
    pub(crate) device_lost_after_prefix: bool,
}

#[derive(Debug)]
enum Phase {
    AwaitingMicrophone,
    Countdown { until: Instant },
    StartProgramPlayback,
    Recording,
    Finalizing,
    Finished(VoiceoverCaptureOutcome),
}

#[derive(Debug)]
struct ActiveTake<C> {
    take: AcceptedVoiceoverTimeline,
    /// Memory-only capability and caller binding. It must never enter the
    /// durable take journal, request receipt, or project operation.
    owner: VoiceoverOwnerSession,
    /// Preserve the accepted caller's request controls for the later atomic
    /// placement, so a lost terminal response can replay exactly once.
    placement_actor: Actor,
    capture: C,
    phase: Phase,
    terminal_intent: Option<VoiceoverTerminalIntent>,
    session: Option<VoiceoverTakeSession>,
}

/// Private Stop/Cancel ownership until one atomic core mutation places the WAV.
#[derive(Debug)]
pub(crate) struct VoiceoverTimelineOwner<C = VoiceoverCaptureSession> {
    countdown: Duration,
    active: Option<ActiveTake<C>>,
}

impl<C> VoiceoverTimelineOwner<C> {
    pub(crate) fn with_countdown(countdown: Duration) -> Self {
        Self {
            countdown,
            active: None,
        }
    }
}

impl Default for VoiceoverTimelineOwner<VoiceoverCaptureSession> {
    fn default() -> Self {
        Self::with_countdown(DEFAULT_COUNTDOWN)
    }
}

impl<C: VoiceoverCaptureControl> VoiceoverTimelineOwner<C> {
    #[cfg(test)]
    pub(crate) fn accept(
        &mut self,
        project: &Project,
        current_revision: &str,
        request: VoiceoverTimelineRequest,
        capture: C,
    ) -> Result<VoiceoverTimelineStatus, CutError> {
        self.accept_bound(project, current_revision, request, capture, Actor::system())
    }

    fn ensure_inactive(&self) -> Result<(), CutError> {
        if self.active.is_some() {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "a voiceover take is already active",
                "Stop or Cancel the existing take before starting another",
            ));
        }
        Ok(())
    }

    /// A real callback starts the visual countdown; no monitoring or guessed readiness.
    pub(crate) fn status(&mut self) -> Result<VoiceoverTimelineStatus, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        if update_terminal(active) {
            sync_durable(active)?;
            return Ok(project_status(active));
        }
        if matches!(active.phase, Phase::AwaitingMicrophone) && active.capture.microphone_ready() {
            active.phase = Phase::Countdown {
                until: Instant::now() + self.countdown,
            };
        }
        sync_durable(active)?;
        if matches!(active.phase, Phase::Countdown { until } if Instant::now() >= until) {
            active.phase = Phase::StartProgramPlayback;
        }
        update_terminal(active);
        sync_durable(active)?;
        Ok(project_status(active))
    }

    /// A future Preview bridge acknowledges normal Timeline playback at the accepted cue.
    pub(super) fn playback_started(
        &mut self,
        request_id: &str,
        request_fingerprint: &str,
        bridge_epoch: u64,
    ) -> Result<VoiceoverTimelineStatus, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        if update_terminal(active) {
            sync_durable(active)?;
            return Ok(project_status(active));
        }
        if !matches!(active.phase, Phase::StartProgramPlayback) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "voiceover recording cannot begin before the countdown",
                "wait for microphone readiness and the visual pre-roll",
            ));
        }
        validate_preview_ack(active, request_id, request_fingerprint, bridge_epoch)?;
        active.capture.begin_recording()?;
        active.phase = Phase::Recording;
        sync_durable(active)?;
        Ok(project_status(active))
    }

    /// Reserve one durable acknowledgement epoch before a future Preview bridge
    /// asks the renderer to seek and start normal program playback.
    pub(super) fn claim_preview_bridge(&mut self) -> Result<VoiceoverPreviewBridgeClaim, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        claim_preview_bridge(active)
    }

    /// Stop only when observed program playhead crosses Out; wall-clock drifts.
    pub(crate) fn observe_program_playhead(
        &mut self,
        playhead_ms: u64,
    ) -> Result<VoiceoverTimelineStatus, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        if update_terminal(active) {
            sync_durable(active)?;
            return Ok(project_status(active));
        }
        if matches!(active.phase, Phase::Recording)
            && active
                .take
                .out_ms
                .is_some_and(|out_ms| playhead_ms >= out_ms)
            && record_terminal_intent(active, VoiceoverTerminalIntent::Out)?
        {
            active.phase = phase_after_finalize(active.capture.stop());
        }
        update_terminal(active);
        sync_durable(active)?;
        Ok(project_status(active))
    }

    pub(crate) fn stop(&mut self) -> Result<VoiceoverTimelineStatus, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        if record_terminal_intent(active, VoiceoverTerminalIntent::Stop)? {
            active.phase = phase_after_finalize(active.capture.stop());
        }
        update_terminal(active);
        sync_durable(active)?;
        Ok(project_status(active))
    }

    pub(crate) fn cancel(&mut self) -> Result<VoiceoverTimelineStatus, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        if record_terminal_intent(active, VoiceoverTerminalIntent::Cancel)? {
            active.phase = phase_after_finalize(active.capture.cancel());
        }
        update_terminal(active);
        sync_durable(active)?;
        Ok(project_status(active))
    }

    /// Safe recovery after an unplaceable terminal needs no project mutation.
    pub(crate) fn discard_unplaceable(&mut self) -> Result<(), CutError> {
        let can_discard = {
            let active = self.active.as_mut().ok_or_else(no_active_take)?;
            update_terminal(active);
            sync_durable(active)?;
            matches!(
                active.phase,
                Phase::Finished(
                    VoiceoverCaptureOutcome::Cancelled
                        | VoiceoverCaptureOutcome::ZeroSamples
                        | VoiceoverCaptureOutcome::DeviceLostNoSamples
                )
            )
        };
        if can_discard {
            self.active = None;
            return Ok(());
        }
        Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover terminal outcome still requires resolution",
            "place a sealed prefix atomically or retain the failed take for recovery",
        ))
    }

    /// A future commit owner receives this only after matching the revision.
    pub(crate) fn materialization(
        &mut self,
        current_revision: &str,
    ) -> Result<VoiceoverMaterialization, CutError> {
        let active = self.active.as_mut().ok_or_else(no_active_take)?;
        update_terminal(active);
        sync_durable(active)?;
        sealed_materialization(active, current_revision)
    }
}

impl VoiceoverTimelineOwner<VoiceoverCaptureSession> {
    /// The sole native-entry seam persists a derived private session before it
    /// reserves a microphone. It never accepts a caller-selected path.
    pub(crate) fn accept_native(
        &mut self,
        project_dir: &Path,
        project: &Project,
        current_revision: &str,
        request: VoiceoverTimelineRequest,
        source: MicrophoneSource,
        placement_actor: Actor,
    ) -> Result<VoiceoverTimelineStatus, CutError> {
        self.ensure_inactive()?;
        let take = admit(project, current_revision, request)?;
        let owner = VoiceoverOwnerSession::issue(&placement_actor, &take)?;
        let mut session = match VoiceoverTakeSession::prepare(project_dir, &take)? {
            VoiceoverTakePreparation::New(session) => *session,
            VoiceoverTakePreparation::Existing(_) => {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "voiceover request was already durably admitted",
                    "the recorder will not resume an orphaned microphone session; inspect its private recovery state instead",
                ))
            }
        };
        let capture = match VoiceoverCaptureSession::start(session.wav_path().to_path_buf(), source)
        {
            Ok(capture) => capture,
            Err(error) => {
                session.record_start_failure()?;
                return Err(crate::screen_record::record_err(error));
            }
        };
        Ok(self.install(take, capture, Some(session), owner, placement_actor))
    }
}
#[cfg(test)]
#[path = "voiceover_timeline_owner_tests.rs"]
mod tests;
