//! FFmpeg override setting verb handler.
//!
//! Owns the Environment "Change ffmpeg" path separately from the main verb
//! dispatcher.

use crate::dispatch::parse_args;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};

/// `system.set_ffmpeg {path?}` — persist a manually chosen ffmpeg executable, or
/// clear back to automatic selection.
pub(crate) async fn system_set_ffmpeg(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        #[serde(default)]
        path: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let chosen = a.path.as_deref().map(str::trim).filter(|s| !s.is_empty());

    // Validate a non-empty pick actually runs ffmpeg before persisting it.
    if let Some(p) = chosen {
        let caps = cut_media::hwencode::probe_ffmpeg_caps(std::path::Path::new(p));
        if caps.version.is_none() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("'{p}' is not a runnable ffmpeg"),
                "pick the ffmpeg executable (it must answer `ffmpeg -version`), or clear to use automatic",
            ));
        }
    }

    cut_media::toolpath::write_override_setting(chosen).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not save the ffmpeg choice: {e}"),
            "check the app-data directory is writable",
        )
    })?;

    // Rescan so the doctor card reflects the new setting (fires doctor_updated).
    let report = state.doctor_rescan().await;
    Ok(VerbResult::ok(json!({
        "override": cut_media::toolpath::read_override_setting(),
        // The choice is cached at startup; applying it fully (incl. HW-encoder
        // selection) needs a restart of the engine/app.
        "restart_required": true,
        "doctor": serde_json::to_value(report)?,
    })))
}
