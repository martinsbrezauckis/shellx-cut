//! Durable projection helpers kept outside the small in-memory owner module.

use cut_core::{error_codes, CutError};
use record_capture::VoiceoverFinalize;

use super::fingerprint;
use super::session::VoiceoverPreviewBridgeClaim;
use super::{
    ActiveTake, Phase, VoiceoverCaptureControl, VoiceoverTerminalIntent, VoiceoverTimelineStatus,
};

pub(super) fn no_active_take() -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        "no active voiceover take",
        "start a voiceover from an unlocked audio track first",
    )
}

pub(super) fn phase_after_finalize(finalize: VoiceoverFinalize) -> Phase {
    match finalize {
        VoiceoverFinalize::Finalizing => Phase::Finalizing,
        VoiceoverFinalize::Finished(outcome) => Phase::Finished(outcome),
    }
}

pub(super) fn update_terminal<C: VoiceoverCaptureControl>(active: &mut ActiveTake<C>) -> bool {
    if let VoiceoverFinalize::Finished(outcome) = active.capture.status() {
        active.phase = Phase::Finished(outcome);
        return true;
    }
    false
}

pub(super) fn sync_durable<C: VoiceoverCaptureControl>(
    active: &mut ActiveTake<C>,
) -> Result<(), CutError> {
    let Some(session) = active.session.as_mut() else {
        return Ok(());
    };
    match &active.phase {
        Phase::AwaitingMicrophone => {
            session.record_phase(super::VoiceoverTakePhase::AwaitingMicrophone)
        }
        Phase::Countdown { .. } => session.record_phase(super::VoiceoverTakePhase::Countdown),
        Phase::StartProgramPlayback => {
            session.record_phase(super::VoiceoverTakePhase::StartProgramPlayback)
        }
        Phase::Recording => session.record_phase(super::VoiceoverTakePhase::Recording),
        Phase::Finalizing => session.record_phase(super::VoiceoverTakePhase::Finalizing),
        // A native finish first records Finalizing, then Sealed and Terminal.
        Phase::Finished(outcome) => session.record_finished_outcome(outcome),
    }
}

pub(super) fn record_terminal_intent<C>(
    active: &mut ActiveTake<C>,
    intent: VoiceoverTerminalIntent,
) -> Result<bool, CutError> {
    if active.terminal_intent.is_some() {
        return Ok(false);
    }
    let recorded = match active.session.as_mut() {
        Some(session) => session.record_terminal_intent(intent)?,
        None => true,
    };
    if recorded {
        active.terminal_intent = Some(intent);
    }
    Ok(recorded)
}

pub(super) fn claim_preview_bridge<C: VoiceoverCaptureControl>(
    active: &mut ActiveTake<C>,
) -> Result<VoiceoverPreviewBridgeClaim, CutError> {
    if !matches!(active.phase, Phase::StartProgramPlayback) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover playback bridge cannot be claimed before countdown",
            "wait until the accepted take requests normal program playback",
        ));
    }
    let session = active.session.as_mut().ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "voiceover take has no durable native session",
            "only a project-owned native take can claim a Preview bridge epoch",
        )
    })?;
    session.claim_preview_bridge(&active.take)
}

pub(super) fn validate_preview_ack<C>(
    active: &ActiveTake<C>,
    request_id: &str,
    request_fingerprint: &str,
    bridge_epoch: u64,
) -> Result<(), CutError> {
    match active.session.as_ref() {
        Some(session) => {
            session.validate_preview_ack(request_id, request_fingerprint, bridge_epoch)
        }
        None if request_id == active.take.request_id
            && request_fingerprint == fingerprint::for_accepted(&active.take)
            && bridge_epoch == 0 =>
        {
            Ok(())
        }
        None => Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover playback acknowledgement does not match its take",
            "acknowledge the exact request and bridge epoch currently awaiting playback",
        )),
    }
}

/// Out is accepted only from the exact Preview bridge claim that started this
/// take. A stale tab cannot turn an old request id/fingerprint/epoch into a
/// Stop after a newer bridge attempt has superseded it.
pub(super) fn validate_preview_observation<C>(
    active: &ActiveTake<C>,
    request_id: &str,
    request_fingerprint: &str,
    bridge_epoch: u64,
) -> Result<(), CutError> {
    if !matches!(active.phase, Phase::Recording) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover Out is not accepted before recording",
            "wait until Preview has acknowledged the current voiceover playback claim",
        ));
    }
    match active.session.as_ref() {
        Some(session) => {
            session.validate_preview_observation(request_id, request_fingerprint, bridge_epoch)
        }
        None if request_id == active.take.request_id
            && request_fingerprint == fingerprint::for_accepted(&active.take)
            && bridge_epoch == 0 =>
        {
            Ok(())
        }
        None => Err(CutError::new(
            error_codes::CONFLICT,
            "voiceover Out does not match the active Preview claim",
            "report Out only with the request id, fingerprint, and epoch that began recording",
        )),
    }
}

pub(super) fn project_status<C>(active: &ActiveTake<C>) -> VoiceoverTimelineStatus {
    match &active.phase {
        Phase::AwaitingMicrophone => VoiceoverTimelineStatus::AwaitingMicrophone {
            take: active.take.clone(),
        },
        Phase::Countdown { until } => VoiceoverTimelineStatus::Countdown {
            take: active.take.clone(),
            remaining_ms: until
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        },
        Phase::StartProgramPlayback => VoiceoverTimelineStatus::StartProgramPlayback {
            take: active.take.clone(),
        },
        Phase::Recording => VoiceoverTimelineStatus::Recording {
            take: active.take.clone(),
        },
        Phase::Finalizing => VoiceoverTimelineStatus::Finalizing {
            take: active.take.clone(),
        },
        Phase::Finished(outcome) => VoiceoverTimelineStatus::Finished {
            take: active.take.clone(),
            outcome: outcome.clone(),
        },
    }
}
