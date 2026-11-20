//! Reversed clips are planned from facts measured on their source files.
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
//! (the smallest windows). Projects without a reversed clip are not touched.

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
}

static MEASURED: Mutex<Option<HashMap<Key, Measurement>>> = Mutex::new(None);

/// The measured facts of the file at `path` (cached per path, size and
/// modification time); the conservative bound when it cannot be measured.
fn measure(path: &Path) -> Measurement {
    let Ok(meta) = std::fs::metadata(path) else {
        return Measurement {
            facts: conservative(),
            measured: false,
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
    let observed = crate::probe::probe(path).ok().as_ref().and_then(from_probe);
    let measurement = Measurement {
        measured: observed.is_some(),
        facts: observed.unwrap_or_else(conservative),
    };
    if let Ok(mut m) = MEASURED.lock() {
        let m = m.get_or_insert_with(HashMap::new);
        if m.len() >= 4096 {
            m.clear(); // bounded: a long session re-measures rather than grows
        }
        m.insert(key, measurement.clone());
    }
    measurement
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

/// The ONE place that says what is known for sure about an asset's source:
/// today this engine's own probe of the file the render reads (cached per file
/// identity), else the conservative bound. A verified-probe record kept by the
/// project store can be connected here without touching the planner.
pub(super) fn known_facts(asset: &Asset, project_dir: &Path) -> Facts {
    measure(&source_path(asset, project_dir)).facts
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

/// `project` with the measured facts of every source of a reversed video clip
/// (`None` when it has no reversed clip: nothing to plan, nothing changes).
pub fn measured_for_reverse(project: &Project, project_dir: &Path) -> Option<Project> {
    let reversed: Vec<String> = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match c {
            cut_core::Clip::Media(m) if m.reverse && m.freeze.is_none() => Some(m.asset.clone()),
            _ => None,
        })
        .collect();
    if reversed.is_empty() {
        return None;
    }
    let mut out = project.clone();
    for id in reversed {
        if let Some(asset) = out.assets.get_mut(&id) {
            let facts = known_facts(asset, project_dir);
            apply(asset, &facts);
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
