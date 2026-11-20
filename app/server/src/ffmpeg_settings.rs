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
            )
            .with_suggested_action(cut_media::tool_setup::CUSTOM_BUILD_NEEDS));
        }
        ffprobe_for_choice(
            std::path::Path::new(p),
            cut_media::toolpath::locate_media_tool("ffprobe"),
        )?;
    }

    cut_media::toolpath::write_override_setting(chosen).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("could not save the ffmpeg choice: {e}"),
            "check the app-data directory is writable",
        )
    })?;

    // Rescan: the scan notices the new choice, re-probes its hardware encoders
    // and fires doctor_updated, so the choice applies now.
    let report = state.doctor_rescan().await;
    Ok(VerbResult::ok(json!({
        "override": cut_media::toolpath::read_override_setting(),
        // Kept for older clients: the choice applies at the re-scan above.
        "restart_required": false,
        "doctor": serde_json::to_value(report)?,
    })))
}

/// A chosen ffmpeg is only useful with an ffprobe: Cut reads every clip with
/// it. Accept the ffprobe beside the choice (used first) or one that already
/// resolves elsewhere (`fallback`); otherwise refuse before saving anything.
fn ffprobe_for_choice(
    ffmpeg: &std::path::Path,
    fallback: Option<std::path::PathBuf>,
) -> Result<(), CutError> {
    if cut_media::toolpath::sibling_tool(ffmpeg, "ffprobe").is_some() || fallback.is_some() {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        format!("no ffprobe next to '{}'", ffmpeg.display()),
        "ShellX Cut reads every clip with ffprobe from the same build, and none was found \
         beside this ffmpeg or anywhere else",
    )
    .with_suggested_action(
        "Put the matching ffprobe in the same folder as this ffmpeg (some builds offer it as a \
         separate download), then choose the ffmpeg again.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_needs_an_ffprobe_beside_it_or_elsewhere() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ffmpeg = dir.path().join("ffmpeg");
        std::fs::write(&ffmpeg, b"").expect("stub");
        let refused = ffprobe_for_choice(&ffmpeg, None).expect_err("no ffprobe anywhere");
        assert_eq!(refused.code, error_codes::INVALID_ARGS);
        assert!(refused.message.contains("no ffprobe"));
        assert!(refused.suggested_action.is_some());

        assert!(ffprobe_for_choice(&ffmpeg, Some("/usr/bin/ffprobe".into())).is_ok());
        let probe_name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        std::fs::write(dir.path().join(probe_name), b"").expect("stub");
        assert!(ffprobe_for_choice(&ffmpeg, None).is_ok());
    }
}
