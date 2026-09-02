//! Screen-record dispatch adapters.
//!
//! Keeps the recorder-specific routes out of the top-level schema dispatcher.
//! The stop and polish adapters deliberately call the parent-private handlers:
//! those handlers coordinate existing dispatch sub-verbs, while low-level
//! recorder operations remain in `crate::screen_record`.

use super::{screen_record_polish, screen_record_stop};
use crate::registry::verb_contract::DispatchTarget;
use crate::state::AppState;
use cut_core::{Actor, VerbResult};
use serde_json::Value;

pub(super) async fn dispatch(
    target: DispatchTarget,
    state: &AppState,
    args: Value,
    actor: Actor,
) -> VerbResult {
    match target {
        DispatchTarget::ScreenRecordDoctor => doctor(args).await,
        DispatchTarget::ScreenRecordMicrophoneSelection => microphone_selection(args).await,
        DispatchTarget::ScreenRecordSystemAudioProbe => system_audio_probe(args).await,
        DispatchTarget::ScreenRecordRehearsalStart => rehearsal_start(args).await,
        DispatchTarget::ScreenRecordRehearsalDiscard => rehearsal_discard(args).await,
        DispatchTarget::ScreenRecordStart => start(state, args).await,
        DispatchTarget::ScreenRecordSceneActivate => scene_activate(state, args).await,
        DispatchTarget::ScreenRecordSceneTimer => scene_timer(state, args).await,
        DispatchTarget::ScreenRecordPause => pause(args).await,
        DispatchTarget::ScreenRecordResume => resume(args).await,
        DispatchTarget::ScreenRecordRecoveryStatus => recovery_status(state, args).await,
        DispatchTarget::ScreenRecordStatus => status(args).await,
        DispatchTarget::ScreenRecordPreviewCapability => preview_capability(args),
        DispatchTarget::ScreenRecordPreviewStart => preview_start(args),
        DispatchTarget::ScreenRecordPreviewStatus => preview_status(args),
        DispatchTarget::ScreenRecordPreviewFrame => preview_frame(args),
        DispatchTarget::ScreenRecordPreviewPause => preview_pause(args),
        DispatchTarget::ScreenRecordPreviewResume => preview_resume(args),
        DispatchTarget::ScreenRecordPreviewHide => preview_hide(args),
        DispatchTarget::ScreenRecordPreviewStop => preview_stop(args),
        DispatchTarget::ScreenRecordStop => stop(state, args, actor).await,
        DispatchTarget::ScreenRecordStudioEvent => studio_event(state, args).await,
        DispatchTarget::ScreenRecordAutoedit => autoedit(state, args).await,
        DispatchTarget::ScreenRecordPolish => polish(state, args, actor).await,
        DispatchTarget::ScreenRecordExport => export(state, args).await,
        _ => unreachable!("top-level dispatcher admitted a non-recording target"),
    }
}

pub(super) async fn doctor(args: Value) -> VerbResult {
    crate::screen_record::screen_record_doctor(args)
        .await
        .into()
}

pub(super) async fn microphone_selection(args: Value) -> VerbResult {
    crate::screen_record::microphone::selection_handler(args)
        .await
        .into()
}

pub(super) async fn system_audio_probe(args: Value) -> VerbResult {
    crate::screen_record::system_audio_capture::probe_handler(args)
        .await
        .into()
}

pub(super) async fn rehearsal_start(args: Value) -> VerbResult {
    crate::screen_record::rehearsal::start(args).await.into()
}

pub(super) async fn rehearsal_discard(args: Value) -> VerbResult {
    crate::screen_record::rehearsal::discard(args).into()
}

pub(super) async fn start(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record::screen_record_start(state, args)
        .await
        .into()
}

pub(super) async fn scene_activate(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record::live_controls::scene_activate(state, args)
        .await
        .into()
}

pub(super) async fn scene_timer(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record::live_controls::scene_timer(state, args)
        .await
        .into()
}

pub(super) async fn pause(args: Value) -> VerbResult {
    crate::screen_record::live_controls::pause(args)
        .await
        .into()
}

pub(super) async fn resume(args: Value) -> VerbResult {
    crate::screen_record::live_controls::resume(args)
        .await
        .into()
}

pub(super) async fn recovery_status(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record::recovery_status_handler(state, args)
        .await
        .into()
}

pub(super) async fn status(args: Value) -> VerbResult {
    crate::screen_record::readiness_status_handler(args)
        .await
        .into()
}

pub(super) fn preview_capability(args: Value) -> VerbResult {
    crate::screen_record::source_preview_capability_handler(args).into()
}

pub(super) fn preview_start(args: Value) -> VerbResult {
    crate::screen_record::source_preview_start_handler(args).into()
}

pub(super) fn preview_status(args: Value) -> VerbResult {
    crate::screen_record::source_preview_status_handler(args).into()
}

pub(super) fn preview_frame(args: Value) -> VerbResult {
    crate::screen_record::source_preview_frame_handler(args).into()
}

pub(super) fn preview_pause(args: Value) -> VerbResult {
    crate::screen_record::source_preview_pause_handler(args).into()
}

pub(super) fn preview_resume(args: Value) -> VerbResult {
    crate::screen_record::source_preview_resume_handler(args).into()
}

pub(super) fn preview_hide(args: Value) -> VerbResult {
    crate::screen_record::source_preview_hide_handler(args).into()
}

pub(super) fn preview_stop(args: Value) -> VerbResult {
    crate::screen_record::source_preview_stop_handler(args).into()
}

pub(super) async fn stop(state: &AppState, args: Value, actor: Actor) -> VerbResult {
    screen_record_stop(state, args, actor).await.into()
}

pub(super) async fn studio_event(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record_studio::screen_record_studio_event(state, args)
        .await
        .into()
}

pub(super) async fn autoedit(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record::screen_record_autoedit(state, args)
        .await
        .into()
}

pub(super) async fn polish(state: &AppState, args: Value, actor: Actor) -> VerbResult {
    screen_record_polish(state, args, actor).await.into()
}

pub(super) async fn export(state: &AppState, args: Value) -> VerbResult {
    crate::screen_record::screen_record_export(state, args)
        .await
        .into()
}
