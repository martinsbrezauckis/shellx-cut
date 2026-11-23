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
    )
    .with_suggested_action(
        "Stop the recording to finish it, or start a new take. Pause and resume is available on macOS only.",
    ))
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pause_and_resume_refusals_keep_cause_and_offer_a_next_action() {
        for result in [
            pause(serde_json::json!({ "capture_id": "recording-test" })).await,
            resume(serde_json::json!({ "capture_id": "recording-test" })).await,
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.code, error_codes::NOT_FOUND);
            assert_eq!(
                error.message,
                "Pause and resume are available only on macOS"
            );
            assert_eq!(
                error.cause,
                "this build has no public pause-safe capture owner on the current platform"
            );
            assert_eq!(error.suggested_action.as_deref(), Some(
                "Stop the recording to finish it, or start a new take. Pause and resume is available on macOS only."
            ));
        }
    }
}
