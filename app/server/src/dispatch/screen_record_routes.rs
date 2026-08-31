//! Screen-record dispatch adapters.
//!
//! Keeps the recorder-specific routes out of the top-level schema dispatcher.
//! The stop and polish adapters deliberately call the parent-private handlers:
//! those handlers coordinate existing dispatch sub-verbs, while low-level
//! recorder operations remain in `crate::screen_record`.

use super::{screen_record_polish, screen_record_stop};
use crate::state::AppState;
use cut_core::{Actor, VerbResult};
use serde_json::Value;

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
