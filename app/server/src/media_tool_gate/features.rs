//! features.rs — refuse a step up front when no installed ffmpeg has a filter
//! the step needs: vid.stab for Stabilize, libass for captions burned into an
//! export, zimg for a color-managed export. Without this, the edit is accepted
//! and the render later fails with ffmpeg's raw "No such filter".
//!
//! The filters are resolved exactly as a render resolves them
//! (`toolpath::ffmpeg_for`), so an installed build that has the filter counts
//! even when the default or the chosen build lacks it.

use crate::state::AppState;
use cut_core::{Clip, ColorConfig, ColorSpace, Project};
use cut_media::render::needs_color_conversion;
use cut_media::toolpath::FfmpegFeature;
use serde_json::Value;

/// Verbs that render the whole timeline: burned captions, stabilized clips and
/// color management all apply there.
const TIMELINE_RENDERS: &[&str] = &[
    "render.final",
    "render.queue",
    "render.bundle",
    "export.publish",
    "export.range",
    "autopilot.run",
];

/// Verbs that build the composed timeline picture without the full feature
/// set of an export (previews, composed frames, scopes). They need zimg only
/// for an HDR clip's tone map. `render.frame` and `render.storyboard` compose
/// only with `compose:true` (otherwise they read the converted proxy).
const PICTURE_RENDERS: &[&str] = &[
    "render.preview",
    "render.direct",
    "render.reframe",
    "export.gif",
    "export.frame",
    "verify.scopes",
];

fn composes(verb: &str, args: &Value) -> bool {
    PICTURE_RENDERS.contains(&verb)
        || (matches!(verb, "render.frame" | "render.storyboard")
            && args.get("compose").and_then(Value::as_bool) == Some(true))
}

/// File names of the HDR clips the open project's timeline tone-maps.
pub(super) async fn hdr_files(state: &AppState) -> Vec<String> {
    let guard = state.project.read().await;
    guard
        .as_ref()
        .map(|store| {
            let edl = cut_core::edl_from_project(&store.project);
            cut_media::render::tone_map_support::tone_map_files(&store.project, &edl)
        })
        .unwrap_or_default()
}

/// The filters `verb` with `args` will need, given the open project.
pub(super) async fn needed(state: &AppState, verb: &str, args: &Value) -> Vec<FfmpegFeature> {
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    if composes(verb, args) {
        return if hdr_files(state).await.is_empty() {
            Vec::new()
        } else {
            vec![FfmpegFeature::Zscale]
        };
    }
    match verb {
        "edit.stabilize" => {
            if args.get("enabled").and_then(Value::as_bool) == Some(false) {
                Vec::new()
            } else {
                vec![FfmpegFeature::Vidstab]
            }
        }
        "project.color" | "edit.color_space" => {
            let guard = state.project.read().await;
            let Some(store) = guard.as_ref() else {
                return Vec::new();
            };
            let mut color = store.project.settings.color;
            let mut tags = clip_tags(&store.project);
            if verb == "project.color" {
                if let Some(space) = text("working").and_then(ColorSpace::parse) {
                    color.working = space;
                }
                if let Some(space) = text("output").and_then(ColorSpace::parse) {
                    color.output = space;
                }
            } else if let Some(space) = text("input").and_then(ColorSpace::parse) {
                tags.push(space);
            }
            if color_needs_zimg(&color, &tags) {
                vec![FfmpegFeature::Zscale]
            } else {
                Vec::new()
            }
        }
        v if TIMELINE_RENDERS.contains(&v) => {
            let guard = state.project.read().await;
            guard
                .as_ref()
                .map(|store| project_needs(&store.project))
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn clip_tags(project: &Project) -> Vec<ColorSpace> {
    project
        .all_sequence_tracks()
        .flat_map(|t| &t.clips)
        .filter_map(|c| match c {
            Clip::Media(m) => m.input_color_space,
            _ => None,
        })
        .collect()
}

fn color_needs_zimg(color: &ColorConfig, tags: &[ColorSpace]) -> bool {
    needs_color_conversion(None, color)
        || tags.iter().any(|t| needs_color_conversion(Some(t), color))
}

/// The filters a full render of `project` needs.
pub(crate) fn project_needs(project: &Project) -> Vec<FfmpegFeature> {
    let clips: Vec<&Clip> = project
        .all_sequence_tracks()
        .flat_map(|t| &t.clips)
        .collect();
    let mut needs = Vec::new();
    if clips
        .iter()
        .any(|c| matches!(c, Clip::Media(m) if m.stabilize.is_some()))
    {
        needs.push(FfmpegFeature::Vidstab);
    }
    if clips.iter().any(|c| matches!(c, Clip::Caption(_))) {
        needs.push(FfmpegFeature::Libass);
    }
    let edl = cut_core::edl_from_project(project);
    let hdr = !cut_media::render::tone_map_support::tone_map_files(project, &edl).is_empty();
    if hdr || color_needs_zimg(&project.settings.color, &clip_tags(project)) {
        needs.push(FfmpegFeature::Zscale);
    }
    needs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_project_needs_no_optional_filter() {
        let project: Project = serde_json::from_value(serde_json::json!({
            "name": "p",
            "schema": "shellx-cut/1",
            "settings": {"width": 1920, "height": 1080, "fps": 30.0, "audio_rate": 48000},
            "assets": {},
            "tracks": [{"id": "v1", "kind": "video", "clips": []}]
        }))
        .expect("minimal project");
        assert!(project_needs(&project).is_empty());

        let mut managed = project.clone();
        managed.settings.color.output = ColorSpace::Rec2020;
        assert_eq!(project_needs(&managed), vec![FfmpegFeature::Zscale]);
        let mut wide = project;
        wide.settings.color.working = ColorSpace::Rec2020;
        wide.settings.color.output = ColorSpace::Rec2020;
        assert!(
            project_needs(&wide).is_empty(),
            "no conversion: rec2020 in and out"
        );
    }
}
