//! Authenticated server owner for Timeline voiceover admission and settlement.
//!
//! The UI may choose a track and observe progress, but it never owns a native
//! microphone, a WAV path, placement, or playback acknowledgement authority.

use crate::dispatch::parse_args;
use crate::events::Event;
use crate::state::AppState;
use crate::voiceover_timeline_owner::{
    VoiceoverCue, VoiceoverOwnerClaim, VoiceoverTimelineRequest, VoiceoverTimelineStatus,
};
use cut_core::{error_codes, Actor, CutError, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};

/// A browser may discard its non-secret retry identity only after this exact
/// code. Other start errors can still describe an active take the caller is
/// not allowed to control, so treating every `ok:false` as releasable would
/// let a stale tab overwrite an unresolved request target.
pub(crate) const START_RETRY_REJECTED: &str = "voiceover_start_retry_rejected";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    audio_track: String,
    start_ms: u64,
    out_ms: Option<u64>,
    owner_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerArgs {
    owner_session_id: String,
    owner_capability: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayheadArgs {
    owner_session_id: String,
    owner_capability: String,
    request_id: String,
    request_fingerprint: String,
    bridge_epoch: u64,
    playhead_ms: u64,
}

/// Start or read back exactly one admitted native take. `request_control`
/// removes and authenticates the retry identity before this handler runs.
pub(crate) async fn start(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let args: StartArgs = parse_args(args)?;
    let owner_session_id = args.owner_session_id.clone();
    let request = start_request(args, &actor)?;
    let _transition = state.project_transition.lock().await;
    let mut project = state.project.write().await;
    let store = project.as_mut().ok_or_else(no_project)?;
    let mut owner = state.voiceover_timeline.lock().await;
    // Probe the full request/actor/target identity before comparing the live
    // project revision. An accepted A can be reattached after ordinary edits
    // advance the revision; a new B still falls through to the guard below.
    // Do not convert an ownership refusal here: it can prove that matching A
    // is active but belongs to a different/stale Timeline caller, which is not
    // safe authority for the browser to discard A's retry identity.
    if let Some(status) = owner.retry_status(&request, &actor, owner_session_id.as_deref())? {
        let claim = owner.owner_claim()?;
        return Ok(VerbResult::ok(status_value(&status, Some(&claim))));
    }
    let current_revision = current_revision(store).map_err(start_retry_rejected)?;
    if current_revision != request.expected_revision {
        return Err(start_retry_rejected(stale_request(
            &request.expected_revision,
            &current_revision,
        )));
    }
    // Keep the existing Record workspace's explicit, capability-backed
    // admission model. A headless/unadmitted build never presents an enabled
    // voiceover path merely because a browser can draw a record icon.
    crate::screen_record::ensure_voiceover_ready().map_err(start_retry_rejected)?;
    let source =
        crate::screen_record::microphone::source_for_start().map_err(start_retry_rejected)?;
    let status = owner
        .accept_native(
            &store.dir,
            &store.project,
            &current_revision,
            request,
            source,
            actor,
        )
        .map_err(start_retry_rejected)?;
    // `accept_native` installs the active owner before returning its status.
    // Thus a received `ok:true` start projection identifies an accepted take.
    // Only `voiceover_start_retry_rejected` proves a start never became an
    // owner; other missing, malformed, or ownership-refused responses leave
    // browser-side admission deliberately unresolved.
    let claim = owner.owner_claim()?;
    Ok(VerbResult::ok(status_value(&status, Some(&claim))))
}

/// Advance readiness/countdown, make the exact correlated Preview request at
/// the playback boundary, and atomically place a sealed take when it finishes.
pub(crate) async fn tick(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let claim = owner_claim(parse_args::<OwnerArgs>(args)?);
    // Do not send two Preview commands while a prior one is still awaiting an
    // acknowledgement. The separate gate leaves Stop/Cancel free to win first.
    let _tick_gate = state.voiceover_tick_gate.lock().await;
    let status = {
        let mut owner = state.voiceover_timeline.lock().await;
        owner.authorize(&actor, &claim)?;
        owner.status()?
    };
    let status = if matches!(status, VoiceoverTimelineStatus::StartProgramPlayback { .. }) {
        let request = {
            let mut owner = state.voiceover_timeline.lock().await;
            owner.preview_playback_request()?
        };
        let acknowledgement = request.await_ack(&state.ui_bridge).await?;
        let mut owner = state.voiceover_timeline.lock().await;
        match owner.playback_started_from_preview(acknowledgement) {
            Ok(status) => status,
            Err(error) => {
                // A concurrently accepted Stop/Cancel wins the terminal race.
                // Do not revive recording from a late Preview acknowledgement.
                let current = owner.status()?;
                if matches!(
                    current,
                    VoiceoverTimelineStatus::StartProgramPlayback { .. }
                ) {
                    return Err(error);
                }
                current
            }
        }
    } else {
        status
    };
    settle(state, status, Some(&claim)).await
}

pub(crate) async fn stop(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let claim = owner_claim(parse_args::<OwnerArgs>(args)?);
    let status = {
        let mut owner = state.voiceover_timeline.lock().await;
        owner.authorize(&actor, &claim)?;
        owner.stop()?
    };
    settle(state, status, Some(&claim)).await
}

pub(crate) async fn cancel(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let claim = owner_claim(parse_args::<OwnerArgs>(args)?);
    let status = {
        let mut owner = state.voiceover_timeline.lock().await;
        owner.authorize(&actor, &claim)?;
        owner.cancel()?
    };
    settle(state, status, Some(&claim)).await
}

/// Preview reports its actual program clock, never a countdown wall clock. The
/// owner alone decides whether crossing Out is the one terminal Stop.
pub(crate) async fn observe_playhead(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let args: PlayheadArgs = parse_args(args)?;
    let claim = VoiceoverOwnerClaim {
        session_id: args.owner_session_id,
        capability: args.owner_capability,
    };
    let status = {
        let mut owner = state.voiceover_timeline.lock().await;
        owner.authorize(&actor, &claim)?;
        owner.verify_preview_observation(
            &args.request_id,
            &args.request_fingerprint,
            args.bridge_epoch,
        )?;
        owner.observe_program_playhead(args.playhead_ms)?
    };
    settle(state, status, Some(&claim)).await
}

async fn settle(
    state: &AppState,
    status: VoiceoverTimelineStatus,
    owner_claim: Option<&VoiceoverOwnerClaim>,
) -> Result<VerbResult, CutError> {
    let terminal_without_wav = matches!(
        &status,
        VoiceoverTimelineStatus::Finished {
            outcome: record_capture::VoiceoverCaptureOutcome::Cancelled
                | record_capture::VoiceoverCaptureOutcome::ZeroSamples
                | record_capture::VoiceoverCaptureOutcome::DeviceLostNoSamples,
            ..
        }
    );
    if terminal_without_wav {
        let result = status_value(&status, None);
        let placement_actor = {
            let mut owner = state.voiceover_timeline.lock().await;
            let actor = owner.placement_actor()?;
            owner.discard_unplaceable()?;
            actor
        };
        // A cancelled/empty terminal has no edit op, but it is still the
        // completion of the original controlled start. Persist the ordinary
        // response receipt under that actor so a lost Stop/Cancel response is
        // recovered by an identical start retry instead of attempting a new
        // native admission.
        let mut envelope = VerbResult::ok(result);
        crate::request_control::finalize_terminal_without_ops(
            state,
            "voiceover.start",
            &placement_actor,
            &mut envelope,
        )
        .await;
        return Ok(envelope);
    }

    let can_place = matches!(
        &status,
        VoiceoverTimelineStatus::Finished {
            outcome: record_capture::VoiceoverCaptureOutcome::Saved(_)
                | record_capture::VoiceoverCaptureOutcome::DeviceLostSavedPrefix(_),
            ..
        }
    );
    if !can_place {
        return Ok(VerbResult::ok(status_value(&status, owner_claim)));
    }

    let _transition = state.project_transition.lock().await;
    let mut project = state.project.write().await;
    let store = project.as_mut().ok_or_else(no_project)?;
    let mut owner = state.voiceover_timeline.lock().await;
    let placement = owner.place_materialization(store)?;
    let placement_actor = owner.placement_actor()?;
    let op = store
        .log
        .read_all()?
        .into_iter()
        .find(|candidate| candidate.op_id == placement.op_id)
        .ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "voiceover placement receipt is missing",
                "the atomic placement did not leave a readable durable operation",
            )
        })?;
    owner.release_after_placement()?;
    drop(owner);
    drop(project);
    if !placement.already_applied {
        state.events.publish(Event::OpApplied { op: op.clone() });
    }
    // Placement consumes the volatile owner. Do not copy its capability into
    // the durable retry receipt or return it after the active take is gone.
    let mut result = status_value(&status, None);
    result["phase"] = json!("placed");
    result["placement"] = json!({
        "asset_id": placement.asset_id,
        "clip_id": placement.clip_id,
        "op_id": placement.op_id,
        "already_applied": placement.already_applied,
    });
    let mut envelope = VerbResult::ok_with_ops(result, vec![op.op_id.clone()]);
    envelope.project_revision = Some(op.op_id);
    crate::request_control::finalize(
        state,
        "voiceover.start",
        &placement_actor,
        &mut envelope,
        true,
    )
    .await;
    Ok(envelope)
}

fn start_request(args: StartArgs, actor: &Actor) -> Result<VoiceoverTimelineRequest, CutError> {
    let request = actor.request.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "voiceover start requires request_id and expected_revision",
            "refresh the Timeline and submit this recording with a new retry identity",
        )
    })?;
    let expected_revision = request.expected_revision.clone().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "voiceover start requires expected_revision",
            "bind the selected track and range to the current project revision",
        )
    })?;
    let cue = match args.out_ms {
        Some(out_ms) => VoiceoverCue::InOut {
            in_ms: args.start_ms,
            out_ms,
        },
        None => VoiceoverCue::Playhead {
            at_ms: args.start_ms,
        },
    };
    Ok(VoiceoverTimelineRequest {
        request_id: request.request_id.clone(),
        expected_revision,
        audio_track: args.audio_track,
        cue,
    })
}

fn owner_claim(args: OwnerArgs) -> VoiceoverOwnerClaim {
    VoiceoverOwnerClaim {
        session_id: args.owner_session_id,
        capability: args.owner_capability,
    }
}

fn current_revision(store: &cut_core::ProjectStore) -> Result<String, CutError> {
    store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "voiceover requires a durable project revision",
            "save or reopen the current project before recording voiceover",
        )
    })
}

fn no_project() -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        "no project is open for voiceover",
        "open a project with an unlocked audio track before recording",
    )
}

fn stale_request(expected: &str, current: &str) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "voiceover target is stale",
        format!(
            "the request accepted '{expected}' but the current project revision is '{current}'"
        ),
    )
    .with_suggested_action("refresh the Timeline and start a new voiceover take")
}

fn start_retry_rejected(mut error: CutError) -> CutError {
    error.code = START_RETRY_REJECTED.into();
    error
}

fn status_value(status: &VoiceoverTimelineStatus, owner: Option<&VoiceoverOwnerClaim>) -> Value {
    let (phase, take, remaining_ms, terminal) = match status {
        VoiceoverTimelineStatus::AwaitingMicrophone { take } => {
            ("waiting_for_microphone", take, None, None)
        }
        VoiceoverTimelineStatus::Countdown { take, remaining_ms } => {
            ("countdown", take, Some(*remaining_ms), None)
        }
        VoiceoverTimelineStatus::StartProgramPlayback { take } => {
            ("starting_playback", take, None, None)
        }
        VoiceoverTimelineStatus::Recording { take } => ("recording", take, None, None),
        VoiceoverTimelineStatus::Finalizing { take } => ("finishing", take, None, None),
        VoiceoverTimelineStatus::Finished { take, outcome } => {
            ("finished", take, None, Some(outcome_name(outcome)))
        }
    };
    let mut value = json!({
        "schema": "shellx-cut/voiceover-timeline/1",
        "phase": phase,
        "request_id": take.request_id,
        "accepted_revision": take.revision,
        "audio_track": take.audio_track,
        "start_ms": take.start_ms,
        "out_ms": take.out_ms,
        "countdown_remaining_ms": remaining_ms,
        "direct_monitoring": "off",
        "terminal": terminal,
    });
    if let Some(owner) = owner {
        value["owner_claim"] = json!({
            "session_id": owner.session_id,
            "capability": owner.capability,
        });
    }
    value
}

fn outcome_name(outcome: &record_capture::VoiceoverCaptureOutcome) -> &'static str {
    match outcome {
        record_capture::VoiceoverCaptureOutcome::Saved(_) => "saved",
        record_capture::VoiceoverCaptureOutcome::DeviceLostSavedPrefix(_) => {
            "device_lost_saved_prefix"
        }
        record_capture::VoiceoverCaptureOutcome::DeviceLostNoSamples => "device_lost_no_samples",
        record_capture::VoiceoverCaptureOutcome::ZeroSamples => "zero_samples",
        record_capture::VoiceoverCaptureOutcome::Cancelled => "cancelled",
        record_capture::VoiceoverCaptureOutcome::CancelCleanupFailed(_) => "cancel_cleanup_failed",
        record_capture::VoiceoverCaptureOutcome::Failed(_) => "failed",
    }
}

#[cfg(test)]
#[path = "voiceover_timeline_coordinator_tests.rs"]
mod tests;
