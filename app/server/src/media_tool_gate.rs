//! media_tool_gate.rs — refuse media verbs up front when ffmpeg is missing.
//!
//! Every verb that reads, previews, converts or renders media runs ffmpeg or
//! ffprobe, often inside a job started after the verb already answered ok or
//! changed the project. When the tools are not installed, that produced an
//! ok:true import whose job then failed with "No such file or directory" and
//! left a half-registered asset. This gate runs in `dispatch` after argument
//! validation and before any handler, so such a verb is refused with
//! `tool_missing`, one plain reason, and the platform's way to get the tools
//! (cut_media::tool_setup) — and nothing has changed.
//!
//! The presence check starts no process: it resolves the tools the same way
//! the media engine does and looks for the files. The spawn-level mapping in
//! cut_media remains the catch-all for anything not listed here.
//!
//! Keep [`MEDIA_TOOL_VERBS`] complete when a verb starts using ffmpeg: a
//! server test checks every entry names a real verb.
//!
//! A second check (`features.rs`) refuses a step whose filters no installed
//! build has — Stabilize without vid.stab, burned captions without libass,
//! color-managed export without zimg — with what is missing and how to get it.

use crate::state::AppState;
use cut_core::CutError;
use serde_json::Value;

mod features;
mod odd_size;

/// Everything `dispatch` admits before a handler runs: valid arguments, then
/// ffmpeg/ffprobe present, then a project size a video export can use, then
/// the filters this step needs.
pub(crate) async fn admit(
    state: &AppState,
    verb: &str,
    args: &Value,
    validated: Result<(), CutError>,
) -> Result<(), CutError> {
    validated?;
    check(verb, args)?;
    odd_size::check(state, verb, args).await?;
    check_features(state, verb, args).await
}

async fn check_features(state: &AppState, verb: &str, args: &Value) -> Result<(), CutError> {
    let needed = features::needed(state, verb, args).await;
    if needed.is_empty() {
        return Ok(());
    }
    if !present("ffmpeg") {
        return Err(cut_media::tool_setup::missing_tool_error("ffmpeg"));
    }
    #[cfg(test)]
    {
        if let Some(stubbed) = test_presence::missing_features(&needed) {
            return refusal(state, verb, args, stubbed.first().copied()).await;
        }
    }
    // Probing filters starts ffmpeg (cached per build per tools generation).
    let missing =
        tokio::task::spawn_blocking(move || cut_media::toolpath::missing_features(&needed))
            .await
            .unwrap_or_default();
    refusal(state, verb, args, missing.first().copied()).await
}

fn requests_keep_hdr(verb: &str, args: &Value) -> bool {
    if args.get("color").and_then(Value::as_str) == Some("keep_hdr") {
        return true;
    }
    verb == "render.queue"
        && args
            .get("jobs")
            .and_then(Value::as_array)
            .is_some_and(|jobs| {
                jobs.iter()
                    .any(|j| j.get("color").and_then(Value::as_str) == Some("keep_hdr"))
            })
}

/// The refusal for the first missing filter. A missing zimg on a timeline
/// with HDR clips says which clips need the tone map (or Keep HDR) and how to
/// proceed, not only "color-managed export".
async fn refusal(
    state: &AppState,
    verb: &str,
    args: &Value,
    missing: Option<cut_media::toolpath::FfmpegFeature>,
) -> Result<(), CutError> {
    let Some(feature) = missing else {
        return Ok(());
    };
    if feature == cut_media::toolpath::FfmpegFeature::Zscale {
        let hdr = features::hdr_files(state).await;
        if !hdr.is_empty() {
            let mut err = cut_media::render::tone_map_support::missing_tone_map_error(&hdr);
            if requests_keep_hdr(verb, args) {
                err.message = format!(
                    "Keep HDR export ({}) needs FFmpeg with zimg (the zscale filter), and the installed FFmpeg has none",
                    hdr.join(", ")
                );
            }
            return Err(err);
        }
    }
    Err(cut_media::tool_setup::missing_feature_error(feature))
}

/// Verbs that cannot do their work without ffmpeg and ffprobe, with the
/// argument shapes that do not touch them (checked by [`exempt`]). Verbs that
/// spawn ffmpeg only in rare states (nested-sequence bakes during interchange
/// export, capture recovery on project.open, jobs.retry of an export job) are
/// left to the spawn-level `tool_missing` mapping; orchestrators (recipe.run,
/// comment.apply, plugins.call) are covered when they dispatch the inner verb.
pub(crate) const MEDIA_TOOL_VERBS: &[&str] = &[
    // import, inspection and relinking: the asset is registered before the
    // probe job runs, so these must refuse up front
    "media.import",
    "media.relink",
    "media.probe",
    "media.relink_preview",
    "media.relink_apply",
    "library.add",
    "library.relink",
    "library.add_to_project",
    "assets.fetch",
    "assets.generate",
    "assemble.broll",
    "import.otio",
    "motion.apply_import",
    "motion.link.refresh",
    "motion.template_to_cut",
    "motion.script_to_cut",
    // media analysis
    "media.waveform",
    "media.filmstrip",
    "media.transcribe",
    "media.perception",
    "media.diarize",
    "media.index",
    "project.cache_rebuild",
    // editing steps that analyse or bake media
    "audio.dub",
    "edit.color_match",
    "edit.auto_balance",
    "edit.multicam_sync",
    "edit.matte",
    "title.add",
    "title.update",
    "edit.add_shape",
    "shape.update",
    "captions.kinetic",
    "generate.insert",
    "generate.from_prompt",
    "generate.storyboard",
    // preview, verification and render
    "render.frame",
    "render.preview",
    "render.compare",
    "render.storyboard",
    "render.final",
    "render.queue",
    "render.direct",
    "render.reframe",
    "render.qc",
    "render.bundle",
    "autopilot.run",
    "verify.loudness",
    "verify.scopes",
    "verify.rerun",
    "verify.judge",
    // export
    "export.publish",
    "export.frame",
    "export.range",
    "export.gif",
    "export.audio",
    "screen_record.polish",
    "screen_record.export",
];

/// Argument shapes of a listed verb that never run the tools. A render dry
/// run is NOT exempt: its plan cannot be carried out here, and saying so now
/// is the point of the gate.
fn exempt(verb: &str, args: &Value) -> bool {
    let text = |name: &str| args.get(name).and_then(Value::as_str);
    let flag = |name: &str| args.get(name).and_then(Value::as_bool);
    match verb {
        // `enabled:false` only clears an existing matte.
        "edit.matte" => flag("enabled") == Some(false),
        // A preview reads the OTIO file without probing media.
        "import.otio" => text("mode") == Some("preview"),
        "motion.apply_import" => flag("dryRun") == Some(true),
        // Motion renders and imports only with the default `insert` policy.
        "motion.template_to_cut" | "motion.script_to_cut" => text("policy") == Some("preview"),
        // Generate plans by default; only `insert` materializes media.
        "generate.from_prompt" | "generate.storyboard" => text("policy") != Some("insert"),
        _ => false,
    }
}

/// Refuse `verb` with `tool_missing` when it needs ffmpeg/ffprobe and either
/// is not installed where Cut looks. Ok for every other verb.
pub(crate) fn check(verb: &str, args: &Value) -> Result<(), CutError> {
    if !MEDIA_TOOL_VERBS.contains(&verb) || exempt(verb, args) {
        return Ok(());
    }
    for stem in ["ffmpeg", "ffprobe"] {
        if !present(stem) {
            return Err(cut_media::tool_setup::missing_tool_error(stem));
        }
    }
    Ok(())
}

fn present(stem: &str) -> bool {
    #[cfg(test)]
    {
        if let Some(stubbed) = test_presence::get(stem) {
            return stubbed;
        }
    }
    cut_media::toolpath::locate_media_tool(stem).is_some()
}

/// Test seam: pretend a tool is missing (or present) on THIS thread only, so
/// gate tests never disturb parallel tests that use the real tools. A
/// `#[tokio::test]` runs `dispatch` on its own thread up to the gate.
#[cfg(test)]
pub(crate) mod test_presence {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static STUB: RefCell<HashMap<&'static str, bool>> = RefCell::new(HashMap::new());
        static NO_FEATURES: RefCell<Option<Vec<cut_media::toolpath::FfmpegFeature>>> =
            const { RefCell::new(None) };
    }

    /// The stubbed-missing subset of `needed`, when a feature stub is set.
    pub(crate) fn missing_features(
        needed: &[cut_media::toolpath::FfmpegFeature],
    ) -> Option<Vec<cut_media::toolpath::FfmpegFeature>> {
        NO_FEATURES.with(|s| {
            s.borrow().as_ref().map(|gone| {
                needed
                    .iter()
                    .copied()
                    .filter(|f| gone.contains(f))
                    .collect()
            })
        })
    }

    /// Stub both tools present but `features` missing from every build.
    pub(crate) fn features_missing(features: &[cut_media::toolpath::FfmpegFeature]) -> Guard {
        STUB.with(|s| {
            let mut s = s.borrow_mut();
            s.insert("ffmpeg", true);
            s.insert("ffprobe", true);
        });
        NO_FEATURES.with(|s| *s.borrow_mut() = Some(features.to_vec()));
        Guard
    }

    pub(crate) fn get(stem: &str) -> Option<bool> {
        STUB.with(|s| s.borrow().get(stem).copied())
    }

    /// Stub both tools missing until the guard drops.
    pub(crate) fn missing() -> Guard {
        STUB.with(|s| {
            let mut s = s.borrow_mut();
            s.insert("ffmpeg", false);
            s.insert("ffprobe", false);
        });
        Guard
    }

    /// Stub only `stem` missing until the guard drops.
    pub(crate) fn only_missing(stem: &'static str) -> Guard {
        STUB.with(|s| {
            let mut s = s.borrow_mut();
            s.insert("ffmpeg", true);
            s.insert("ffprobe", true);
            s.insert(stem, false);
        });
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            STUB.with(|s| s.borrow_mut().clear());
            NO_FEATURES.with(|s| *s.borrow_mut() = None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listed_verbs_refuse_with_the_missing_tool_and_a_way_forward() {
        let _missing = test_presence::missing();
        let error = check("media.import", &json!({"path": "/x.mp4"})).expect_err("refused");
        assert_eq!(error.code, cut_core::error_codes::TOOL_MISSING);
        assert_eq!(error.message, "ffmpeg is not installed");
        assert!(error.suggested_action.is_some());
    }

    #[test]
    fn a_missing_ffprobe_alone_is_named() {
        let _missing = test_presence::only_missing("ffprobe");
        let error = check("render.frame", &json!({})).expect_err("refused");
        assert_eq!(error.message, "ffprobe is not installed");
    }

    #[test]
    fn other_verbs_and_dry_runs_pass() {
        let _missing = test_presence::missing();
        assert!(check("project.create", &json!({"name": "x"})).is_ok());
        assert!(check("system.doctor", &json!({})).is_ok());
        assert!(check("edit.matte", &json!({"clip": "c1", "enabled": false})).is_ok());
        assert!(check(
            "import.otio",
            &json!({"path": "/x.otio", "mode": "preview"})
        )
        .is_ok());
        assert!(check("generate.from_prompt", &json!({"prompt": "x"})).is_ok());
        assert!(check(
            "generate.from_prompt",
            &json!({"prompt": "x", "policy": "insert"})
        )
        .is_err());
        assert!(check("motion.template_to_cut", &json!({"template": "t"})).is_err());
        assert!(check("render.final", &json!({"dry_run": true})).is_err());
    }

    #[test]
    fn present_tools_pass() {
        let _present = test_presence::only_missing("none");
        assert!(check("media.import", &json!({"path": "/x.mp4"})).is_ok());
    }
}
