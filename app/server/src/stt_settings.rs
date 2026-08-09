//! STT model setting verb handler.
//!
//! Keeps the transcription-model Environment/debug contract out of the main
//! verb dispatcher while preserving the existing `system.set_stt_model` result.

use crate::dispatch::parse_args;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};

/// system.set_stt_model — pick the TRANSCRIPTION (STT) model + optional language
/// Persists the choice (perception app-data) so every perception run
/// injects it (SHELLX_CUT_STT_MODEL / SHELLX_CUT_STT_LANG) into the python
/// sidecar. Recommended models: `nemo-parakeet-tdt-0.6b-v3`,
/// `nemo-canary-1b-v2`, and `whisperx-large-v3`.
pub(crate) async fn system_set_stt_model(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    #[allow(dead_code)]
    struct Args {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        language: Option<String>,
        /// Reset to the built-in default model (ignores model/language).
        #[serde(default)]
        clear: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let (model, language) = if a.clear.unwrap_or(false) {
        (None, None)
    } else {
        (
            a.model.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            a.language
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        )
    };
    cut_perception::write_stt_setting(model, language).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not save the STT model choice: {e}"),
            "check the perception app-data directory is writable",
        )
    })?;
    // Rescan so the perception doctor card reflects the new active model.
    let report = state.doctor_rescan().await;
    let (active_model, active_language) = cut_perception::read_stt_setting();
    Ok(VerbResult::ok(json!({
        "model": active_model,
        "language": active_language,
        "default_model": "nemo-parakeet-tdt-0.6b-v3",
        "applies": "next uncached perception run (re-run media.perception/transcribe to re-transcribe)",
        "doctor": serde_json::to_value(report)?,
    })))
}
