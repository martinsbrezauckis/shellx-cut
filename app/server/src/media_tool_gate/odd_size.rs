//! odd_size.rs — refuse a video export of an odd-sized project up front.
//!
//! An older project can carry an odd frame size (641x361). A video at that
//! size cannot be encoded (4:2:0 needs even sides), and the owner's rule is to
//! tell the person BEFORE anything changes rather than round during the
//! export. So every verb that would encode video at the project size refuses
//! here, before its handler runs, with `odd_project_size`, one plain sentence
//! and the exact `project.format` call. The media layer repeats the check
//! (`cut_media::render::odd_size`) for paths that do not come through here.
//!
//! Not refused, and why:
//! - exports with their own even size: `export.publish` (platform size),
//!   `render.bundle` (platform sizes), `render.final` with `aspect`, `width`/
//!   `height` or `resolution:"match_source"`;
//! - stills and previews of frames (`export.frame`, `render.frame`,
//!   `render.storyboard`, `verify.scopes`): they are composed at an even
//!   preview size already; `export.frame` says when its still is 1 px smaller;
//! - `export.audio` and everything without video.

use crate::state::AppState;
use cut_core::CutError;
use serde_json::Value;

/// Verbs that always encode video at the project size.
const PROJECT_SIZE_VIDEO: &[&str] = &[
    "render.direct",
    "render.reframe",
    "render.preview",
    "export.range",
    "export.gif",
    "autopilot.run",
];

/// A `render.final`-shaped request (a render.final call or one render.queue
/// entry) that renders at the project size: no explicit geometry and no
/// `match_source`.
fn renders_at_project_size(args: &Value) -> bool {
    let has = |key: &str| args.get(key).is_some_and(|v| !v.is_null());
    let resolution = args.get("resolution").and_then(Value::as_str);
    !has("aspect")
        && !has("width")
        && !has("height")
        && matches!(resolution, None | Some("project"))
}

/// True when `verb` with `args` would encode video at the project size.
pub(super) fn needs_even_project(verb: &str, args: &Value) -> bool {
    match verb {
        "render.final" => renders_at_project_size(args),
        "render.queue" => args
            .get("jobs")
            .and_then(Value::as_array)
            .is_some_and(|jobs| jobs.iter().any(renders_at_project_size)),
        v => PROJECT_SIZE_VIDEO.contains(&v),
    }
}

/// Refuse when the open project is odd-sized and `verb` would encode video at
/// that size.
pub(super) async fn check(state: &AppState, verb: &str, args: &Value) -> Result<(), CutError> {
    if !needs_even_project(verb, args) {
        return Ok(());
    }
    let size = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .map(|store| (store.project.settings.width, store.project.settings.height))
    };
    match size {
        Some((w, h)) => cut_core::frame_size::require_even_project(w, h),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_video_at_the_project_size_needs_an_even_project() {
        for (verb, args) in [
            ("render.final", json!({})),
            (
                "render.final",
                json!({"dry_run": true, "resolution": "project"}),
            ),
            (
                "render.queue",
                json!({"jobs": [{"aspect": "9:16"}, {"preset": "draft"}]}),
            ),
            ("export.range", json!({"range_ms": [0, 1000]})),
            ("export.gif", json!({})),
            ("render.preview", json!({})),
            ("render.direct", json!({})),
            ("render.reframe", json!({"aspect": "9:16"})),
            ("autopilot.run", json!({})),
        ] {
            assert!(needs_even_project(verb, &args), "{verb} {args}");
        }
        for (verb, args) in [
            ("render.final", json!({"aspect": "16:9"})),
            ("render.final", json!({"width": 1280, "height": 720})),
            ("render.final", json!({"resolution": "match_source"})),
            ("render.queue", json!({"jobs": [{"aspect": "9:16"}]})),
            ("export.publish", json!({"platform": "youtube"})),
            ("render.bundle", json!({})),
            ("export.frame", json!({"at_ms": 0})),
            ("render.frame", json!({"at_ms": 0, "compose": true})),
            ("render.storyboard", json!({})),
            ("verify.scopes", json!({})),
            ("export.audio", json!({})),
        ] {
            assert!(!needs_even_project(verb, &args), "{verb} {args}");
        }
    }
}
