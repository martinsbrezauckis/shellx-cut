//! Render sources are planned from facts measured on their source files.
//!
//! `reverse` holds every decoded frame of a reversed part, so the render plan
//! (window splits, the single-pass limit, the memory estimate) multiplies the
//! source's width, height, pixel format and frame rate. Those came from the
//! project's cached probe, which a received project can state as anything: a
//! 4K 30 fps source described as small and slow got a window far above the
//! memory bound. Before planning, every source of a reversed clip is measured
//! by this engine (ffprobe on the file the render reads, cached per file
//! identity), and those facts replace the cached ones in the copy of the
//! project the render plans and builds with. A source that cannot be measured
//! or reports impossible facts is planned at the conservative bound below
//! (the smallest windows). Complete probes also replace kind and colour facts
//! for ordinary clips, in a runtime copy only.

use cut_core::{Asset, Project, TrackKind};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// The facts a reversed part's memory depends on.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Facts {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pix_fmt: String,
    pub(super) fps: f64,
}

/// The conservative bound: an 8K, 16-bit 4:4:4 with alpha, 240 fps frame
/// stream. Planned with these, a reversed clip is split into the shortest
/// parts the planner makes (one second).
pub(super) fn conservative() -> Facts {
    Facts {
        width: 7680,
        height: 4320,
        pix_fmt: "yuva444p16le".into(),
        fps: 240.0,
    }
}

/// Largest side and rate taken as measured; anything beyond is not a real
/// source and gets the conservative bound.
const MAX_SIDE: u32 = 16_384;
const MAX_FPS: f64 = 1_000.0;

/// The facts of a fresh probe, or `None` when they are missing or impossible.
pub(super) fn from_probe(probe: &crate::probe::MediaProbe) -> Option<Facts> {
    let (width, height, fps) = (probe.width?, probe.height?, probe.fps?);
    if !(1..=MAX_SIDE).contains(&width)
        || !(1..=MAX_SIDE).contains(&height)
        || !(fps.is_finite() && fps > 0.0 && fps <= MAX_FPS)
    {
        return None;
    }
    let pix_fmt = probe.raw["streams"]
        .as_array()?
        .iter()
        .find(|s| s["codec_type"].as_str() == Some("video"))?["pix_fmt"]
        .as_str()?
        .to_string();
    Some(Facts {
        width,
        height,
        pix_fmt,
        fps,
    })
}

type Key = (PathBuf, u64, Option<SystemTime>);
#[derive(Clone)]
struct Measurement {
    facts: Facts,
    measured: bool,
    probe: Option<crate::probe::MediaProbe>,
}

static MEASURED: Mutex<Option<HashMap<Key, Measurement>>> = Mutex::new(None);

/// The measured facts of the file at `path` (cached per path, size and
/// modification time); the conservative bound when it cannot be measured.
fn measure(path: &Path) -> Measurement {
    let Ok(meta) = std::fs::metadata(path) else {
        return Measurement {
            facts: conservative(),
            measured: false,
            probe: None,
        };
    };
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(f) = MEASURED
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(&key).cloned()))
    {
        return f;
    }
    let probe = crate::probe::probe(path).ok();
    let observed = probe.as_ref().and_then(from_probe);
    let measurement = Measurement {
        measured: observed.is_some(),
        facts: observed.unwrap_or_else(conservative),
        probe,
    };
    if measurement.probe.is_some() {
        if let Ok(mut m) = MEASURED.lock() {
            let m = m.get_or_insert_with(HashMap::new);
            if m.len() >= 4096 {
                m.clear(); // bounded: a long session re-measures rather than grows
            }
            m.insert(key, measurement.clone());
        }
    }
    measurement
}

/// Complete engine-origin probe for the actual selected decoder input.
pub(super) fn input_probe(path: &Path) -> Result<crate::probe::MediaProbe, cut_core::CutError> {
    if let Some(probe) = measure(path).probe {
        Ok(probe)
    } else {
        crate::probe::probe(path)
    }
}

/// Write `facts` into the probe fields the planner and the graph read.
fn apply(asset: &mut Asset, facts: &Facts) {
    let mut probe = asset.probe.take().unwrap_or_else(|| json!({}));
    if !probe.is_object() {
        probe = json!({});
    }
    probe["width"] = json!(facts.width);
    probe["height"] = json!(facts.height);
    probe["fps"] = json!(facts.fps);
    let streams = probe["raw"]["streams"].as_array_mut();
    match streams.and_then(|s| s.iter_mut().find(|s| s["codec_type"] == "video")) {
        Some(video) => video["pix_fmt"] = Value::from(facts.pix_fmt.clone()),
        None => {
            probe["raw"] = json!({"streams": [{"codec_type": "video", "pix_fmt": facts.pix_fmt}]});
        }
    }
    asset.probe = Some(probe);
}

/// Size one reverse part from the same source measurement used by rendering.
/// The received asset probe is discarded; unknown sources use the render's
/// conservative bound and are reported as such to the edit caller.
pub fn measured_reverse_part_bytes(
    asset: &Asset,
    project_dir: &Path,
    project_fps: f64,
    speed: f64,
    clip_ms: u64,
) -> (u64, bool) {
    let measurement = measure(&source_path(asset, project_dir));
    let mut measured_asset = asset.clone();
    measured_asset.probe = None;
    apply(&mut measured_asset, &measurement.facts);
    (
        super::reverse_part_bytes(&measured_asset, project_fps, speed, clip_ms),
        measurement.measured,
    )
}

/// The source file an asset's clips are decoded from.
fn source_path(asset: &Asset, project_dir: &Path) -> PathBuf {
    let mut path = PathBuf::from(&asset.path);
    if path.is_relative() {
        path = project_dir.join(path);
    }
    super::input_paths::strip_verbatim_prefix(&path)
}

/// Runtime source snapshot. The historical name is retained for callers;
/// ordinary clips also need measured kind, dimensions and colour on reopen.
/// Unmeasurable reversed sources retain conservative memory planning.
/// An unavailable ordinary source keeps its last-known metadata so existing
/// proxy eligibility survives. `input_probe` still measures the selected input.
pub fn measured_for_reverse(project: &Project, project_dir: &Path) -> Option<Project> {
    let referenced: std::collections::BTreeSet<String> = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match c {
            cut_core::Clip::Media(m) => Some(m.asset.clone()),
            _ => None,
        })
        .collect();
    if referenced.is_empty() {
        return None;
    }
    let mut out = project.clone();
    for id in referenced {
        let reversed = project.tracks.iter().flat_map(|t| &t.clips).any(|c| matches!(c, cut_core::Clip::Media(m) if m.asset == id && m.reverse && m.freeze.is_none()));
        if let Some(asset) = out.assets.get_mut(&id) {
            let measurement = measure(&source_path(asset, project_dir));
            if let Some(probe) = measurement.probe {
                asset.probe = serde_json::to_value(probe).ok();
                if reversed && !measurement.measured {
                    apply(asset, &measurement.facts);
                }
            } else if reversed {
                // Unknown reverse memory stays bounded. A missing ordinary
                // source must not lose the geometry needed to use its proxy;
                // the selected decoder input is always measured separately.
                asset.probe = None;
                apply(asset, &measurement.facts);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(w: Option<u32>, h: Option<u32>, fps: Option<f64>) -> crate::probe::MediaProbe {
        serde_json::from_value(json!({
            "kind": "video", "duration_ms": 1000, "width": w, "height": h, "fps": fps,
            "variable_frame_rate": false, "has_audio": false, "format": "mp4",
            "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p10le"}]}
        }))
        .expect("probe")
    }

    #[test]
    fn render_kind_comes_from_the_file_even_after_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.mp4");
        let mut project = Project::new("forged", cut_core::ProjectSettings::default());
        project.settings.width = 64;
        project.settings.height = 36;
        project.settings.fps = 10.0;
        project.assets.insert(
            "a1".into(),
            serde_json::from_value(json!({
                "path": path, "hash": "forged", "probe": {"kind": "image"}
            }))
            .unwrap(),
        );
        project.tracks[0].clips.push(
            serde_json::from_value(json!({
                "id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000
            }))
            .unwrap(),
        );
        let made = std::process::Command::new(crate::ffmpeg::ffmpeg_bin())
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=64x36:r=10:d=1",
                "-c:v",
                "libx264",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let measured =
            measured_for_reverse(&project, dir.path()).expect("ordinary sources must be measured");
        assert_eq!(
            measured.assets["a1"].probe.as_ref().unwrap()["kind"],
            "video"
        );
        render_matches_measured(&project, &measured, dir.path());
        let proxy = dir.path().join("proxy.mp4");
        std::fs::copy(&path, &proxy).unwrap();
        // Reopened project still claims video; the same path now holds a picture.
        project.assets.get_mut("a1").unwrap().probe = Some(json!({"kind": "video"}));
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/valid-still.png"),
            &path,
        )
        .unwrap();
        let measured = measured_for_reverse(&project, dir.path()).unwrap();
        assert_eq!(
            measured.assets["a1"].probe.as_ref().unwrap()["kind"],
            "image"
        );
        render_matches_measured(&project, &measured, dir.path());
        assert_eq!(
            project.assets["a1"].probe.as_ref().unwrap()["kind"],
            "video"
        );
        let mut with_proxy = measured;
        with_proxy.assets.get_mut("a1").unwrap().proxy = Some(proxy.display().to_string());
        let edl = cut_core::edl_from_project(&with_proxy);
        let inputs = super::super::graph_inputs::collect_graph_inputs(
            &with_proxy,
            &edl,
            dir.path(),
            true,
            true,
            64,
            36,
        )
        .unwrap()
        .0;
        assert!(inputs[0].proxy);
        assert!(
            !inputs[0].image,
            "selected proxy is a video, despite image source"
        );
        std::fs::remove_file(proxy).unwrap();
        let inputs = super::super::graph_inputs::collect_graph_inputs(
            &with_proxy,
            &edl,
            dir.path(),
            true,
            true,
            64,
            36,
        )
        .unwrap()
        .0;
        assert!(!inputs[0].proxy);
        assert!(
            inputs[0].image,
            "vanished proxy falls back to measured still"
        );
    }

    #[test]
    fn an_offline_source_keeps_a_measurable_proxy_available() {
        let dir = tempfile::tempdir().unwrap();
        let proxy = dir.path().join("proxy.mp4");
        super::super::tests::graph_fixture(&proxy);
        let mut project = Project::new("offline", Default::default());
        project.assets.insert(
            "a1".into(),
            serde_json::from_value(json!({
                "path": dir.path().join("offline.mp4"), "hash": "old-source",
                "proxy": proxy, "probe": {"kind": "video", "width": 64, "height": 36, "fps": 10.0}
            }))
            .unwrap(),
        );
        project.tracks[0].clips.push(
            serde_json::from_value(json!({
                "id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000
            }))
            .unwrap(),
        );
        let measured = measured_for_reverse(&project, dir.path()).unwrap();
        let edl = cut_core::edl_from_project(&measured);
        let inputs = super::super::graph_inputs::collect_graph_inputs(
            &measured,
            &edl,
            dir.path(),
            true,
            true,
            64,
            36,
        )
        .expect("an offline original must not disable its usable proxy")
        .0;
        assert!(inputs[0].proxy);
        assert!(!inputs[0].image);
    }

    fn render_matches_measured(forged: &Project, measured: &Project, dir: &Path) {
        let edl = cut_core::edl_from_project(forged);
        let fence = crate::PathFence::new(dir).unwrap();
        let preset = super::super::RenderPreset::named("draft").unwrap();
        let a = super::super::render_final(
            forged,
            &edl,
            &fence,
            &dir.join("forged.mp4"),
            &preset,
            Default::default(),
            None,
        )
        .unwrap();
        let b = super::super::render_final(
            measured,
            &edl,
            &fence,
            &dir.join("correct.mp4"),
            &preset,
            Default::default(),
            None,
        )
        .unwrap();
        assert_eq!(
            a.hash, b.hash,
            "forged kind must not change rendered pictures"
        );
    }

    #[test]
    fn impossible_or_missing_facts_take_the_conservative_bound() {
        let ok = from_probe(&probe(Some(3840), Some(2160), Some(30.0))).unwrap();
        assert_eq!(
            (ok.width, ok.height, ok.pix_fmt.as_str()),
            (3840, 2160, "yuv420p10le")
        );
        assert_eq!(
            from_probe(&probe(Some(u32::MAX), Some(2160), Some(30.0))),
            None
        );
        assert_eq!(from_probe(&probe(Some(3840), None, Some(30.0))), None);
        assert_eq!(
            from_probe(&probe(Some(3840), Some(2160), Some(f64::NAN))),
            None
        );
        assert_eq!(from_probe(&probe(Some(3840), Some(2160), Some(1e9))), None);
    }

    #[test]
    fn a_source_that_cannot_be_measured_is_planned_at_the_bound() {
        let fallback = measure(Path::new("/nonexistent/clip.mp4"));
        assert!(!fallback.measured);
        assert_eq!(fallback.facts, conservative());
    }
}
