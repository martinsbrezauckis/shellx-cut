use super::no_project;
use crate::state::AppState;
use cut_core::{error_codes, CutError};

/// Load the word transcript for an asset from its sidecar file (real I/O —
/// live as soon as a transcribe run has produced the file).
pub(crate) async fn load_transcript(
    state: &AppState,
    asset_id: &str,
) -> Result<cut_perception::Transcript, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let asset = store.project.assets.get(asset_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{asset_id}'"),
            "unknown asset id",
        )
    })?;
    // No transcript: say why (captions not installed, or never transcribed).
    let rel = asset
        .transcript
        .as_ref()
        .ok_or_else(|| no_transcript_reason(Some(asset_id)))?;
    let bytes = crate::transcript_receipt::read(&store.dir, asset_id, rel)?;
    let t: cut_perception::Transcript = serde_json::from_slice(&bytes)?;
    Ok(t)
}

/// The truthful reason an asset has no transcript to work from.
///
/// - Captions are not installed (no captions Python): the one shared captions
///   refusal, with the Install captions / `system.setup_perception` action.
///   Nothing can be transcribed until the person installs them.
/// - Captions are installed but this asset was never transcribed: a plain
///   not-found that names the verb and the button that make one.
pub(crate) fn no_transcript_reason(asset_id: Option<&str>) -> CutError {
    if let Err(error) = cut_perception::sidecar_runtime() {
        if cut_perception::is_captions_not_installed(&error) {
            return error;
        }
    }
    let what = match asset_id {
        Some(id) => format!("asset '{id}' has no transcript yet"),
        None => "no asset in this project has a transcript yet".to_string(),
    };
    CutError::new(
        error_codes::NOT_FOUND,
        what,
        "this verb works from the spoken words, and nothing has been transcribed for it",
    )
    .with_suggested_action(
        "call media.transcribe{asset} and wait for the job to finish, then retry (in the app a transcript is made when a clip is imported, or by a recipe that starts with one)",
    )
    .with_requirement("transcript")
}

/// Which of `asset_ids` have a transcript receipt registered.
pub(crate) async fn assets_with_transcript(
    state: &AppState,
    asset_ids: &[String],
) -> Result<(Vec<String>, Vec<String>), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    if let Some(unknown) = asset_ids
        .iter()
        .find(|id| !store.project.assets.contains_key(*id))
    {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{unknown}'"),
            "unknown asset id",
        ));
    }
    let (with, without) = asset_ids.iter().cloned().partition(|id| {
        store
            .project
            .assets
            .get(id)
            .is_some_and(|asset| asset.transcript.is_some())
    });
    Ok((with, without))
}
