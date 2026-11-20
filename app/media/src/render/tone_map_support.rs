//! Refuse, in plain words, a render that needs the HDR tone map when no
//! installed ffmpeg can run it.
//!
//! The tone map (`source_color::hdr_to_sdr_filter`) uses ffmpeg's `zscale`
//! filter, which exists only in builds with zimg. Without such a build every
//! path that builds the timeline picture (export, preview, a composed frame,
//! scopes) used to fail late with ffmpeg's "No such filter: 'zscale'". The
//! server's verb gate refuses those verbs up front with
//! [`missing_tone_map_error`]; [`require_tone_map_support`] is the same check
//! at the media layer, for any path (the HTTP frame route, a job) that reaches
//! the graph without that gate.

use super::source_color::needs_tone_map;
use cut_core::{CutError, Edl, Project, TrackKind};

/// File names of the HDR clips on `edl` that a render tone-maps (HDR and not
/// tagged by the person), in timeline order, each once.
pub fn tone_map_files(project: &Project, edl: &Edl) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for seg in edl
        .segments
        .iter()
        .filter(|s| s.track_kind == TrackKind::Video)
    {
        let Some(asset) = seg.asset.as_deref().and_then(|a| project.assets.get(a)) else {
            continue;
        };
        if !needs_tone_map(asset, seg.input_color_space.as_ref()) {
            continue;
        }
        let name = std::path::Path::new(&asset.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| asset.path.clone());
        if !files.contains(&name) {
            files.push(name);
        }
    }
    files
}

/// The refusal: which clips need converting, what is missing, how to get it
/// (the platform's way to a zimg build, from `tool_setup`), and the way to
/// export without converting.
pub fn missing_tone_map_error(files: &[String]) -> CutError {
    let mut error =
        crate::tool_setup::missing_feature_error(crate::toolpath::FfmpegFeature::Zscale);
    error.message = format!(
        "Converting the HDR video ({}) to SDR needs FFmpeg with zimg (the zscale filter), and \
         the installed FFmpeg has none",
        files.join(", ")
    );
    error.cause = "HDR (HLG/PQ) clips are tone-mapped to SDR BT.709 in every export, preview and \
                   composed frame. Without zimg they cannot be converted. To use them anyway, tag \
                   them rec709 with edit.color_space (they are then shown unconverted and their \
                   colours look flat), or remove them."
        .into();
    error
}

/// [`require_tone_map_support`] with the zscale answer given: pure, testable.
pub fn require_tone_map_support_with(
    project: &Project,
    edl: &Edl,
    zscale_available: impl FnOnce() -> bool,
) -> Result<(), CutError> {
    let files = tone_map_files(project, edl);
    if files.is_empty() || zscale_available() {
        return Ok(());
    }
    Err(missing_tone_map_error(&files))
}

/// Refuse before any ffmpeg runs when `edl` has an HDR clip to tone-map and
/// no installed ffmpeg has zscale. Probes ffmpeg only when there is such a clip.
pub fn require_tone_map_support(project: &Project, edl: &Edl) -> Result<(), CutError> {
    require_tone_map_support_with(project, edl, crate::hwencode::zscale_available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(stream: serde_json::Value, tag: Option<cut_core::ColorSpace>) -> Project {
        let mut p: Project = serde_json::from_value(json!({
            "name": "p",
            "schema": "shellx-cut/1",
            "settings": {"width": 320, "height": 180, "fps": 30.0, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/hlg.mp4", "hash": "sha256:00",
                "probe": {"kind": "video", "width": 320, "height": 180, "duration_ms": 1000,
                          "has_audio": false, "raw": {"streams": [stream]}}}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}
            ]}]
        }))
        .expect("project");
        if let Some(cut_core::Clip::Media(m)) = p.track_mut("v1").unwrap().clips.first_mut() {
            m.input_color_space = tag;
        }
        p
    }

    fn hlg() -> serde_json::Value {
        json!({"codec_type": "video", "pix_fmt": "yuv420p10le",
               "color_transfer": "arib-std-b67", "color_primaries": "bt2020"})
    }

    #[test]
    fn an_hdr_clip_without_zscale_is_refused_by_name_with_both_ways_out() {
        let p = project(hlg(), None);
        let edl = cut_core::edl_from_project(&p);
        let e = require_tone_map_support_with(&p, &edl, || false).unwrap_err();
        assert_eq!(e.code, cut_core::error_codes::TOOL_MISSING);
        assert!(
            e.message
                .starts_with("Converting the HDR video (hlg.mp4) to SDR"),
            "{}",
            e.message
        );
        assert!(e.cause.contains("edit.color_space"), "{}", e.cause);
        assert!(e.suggested_action.is_some());
        assert!(require_tone_map_support_with(&p, &edl, || true).is_ok());
    }

    #[test]
    fn sdr_and_person_tagged_clips_never_probe_or_refuse() {
        let sdr = project(json!({"codec_type": "video", "pix_fmt": "yuv420p"}), None);
        let edl = cut_core::edl_from_project(&sdr);
        assert!(require_tone_map_support_with(&sdr, &edl, || panic!("no probe for SDR")).is_ok());
        let tagged = project(hlg(), Some(cut_core::ColorSpace::Rec709));
        let edl = cut_core::edl_from_project(&tagged);
        assert!(require_tone_map_support_with(&tagged, &edl, || false).is_ok());
    }
}
