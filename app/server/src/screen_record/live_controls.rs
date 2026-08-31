//! Public live-recording controls kept outside the capture orchestration root.

#[cfg(target_os = "macos")]
use super::macos_pause_control;
use super::recording_scenes;
#[cfg(not(target_os = "macos"))]
use crate::dispatch::parse_args;
use crate::state::AppState;
#[cfg(not(target_os = "macos"))]
use cut_core::error_codes;
use cut_core::{CutError, VerbResult};
use serde_json::Value;

pub(crate) async fn scene_activate(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    recording_scenes::screen_record_scene_activate(state, args).await
}

pub(crate) async fn scene_timer(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    recording_scenes::screen_record_scene_timer(state, args).await
}

#[cfg(target_os = "macos")]
pub(crate) async fn pause(args: Value) -> Result<VerbResult, CutError> {
    macos_pause_control::pause_handler(args).await
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn pause(args: Value) -> Result<VerbResult, CutError> {
    unavailable_pause(args)
}

#[cfg(target_os = "macos")]
pub(crate) async fn resume(args: Value) -> Result<VerbResult, CutError> {
    macos_pause_control::resume_handler(args).await
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn resume(args: Value) -> Result<VerbResult, CutError> {
    unavailable_pause(args)
}

#[cfg(not(target_os = "macos"))]
fn unavailable_pause(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        capture_id: String,
    }

    let args: Args = parse_args(args)?;
    super::recovery::validate_capture_id(&args.capture_id)?;
    Err(CutError::new(
        error_codes::NOT_FOUND,
        "Pause and resume are available only on macOS",
        "this build has no public pause-safe capture owner on the current platform",
    ))
}
