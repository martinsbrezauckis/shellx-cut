//! track.rs — server-side MOTION TRACKING (the `edit.track` measurement).
//!
//! Role: track a single region (a face, a bottle, a label, a plate…) across a
//! clip's frames and turn its trajectory into ready-to-apply keyframe arrays —
//! position (pos_x/pos_y) and optional scale — that the EXISTING keyframe machinery
//! (`edit.keyframe`) and region masks (`edit.add_mask`) already consume. It
//! is a MEASUREMENT (like `verify.scopes`): it runs the tracker ONCE in the dispatch
//! handler and returns a receipt; it never mutates the op log, so replay stays fully
//! deterministic and offline (no cv2 at replay time). The agent binds the returned
//! arrays to a target in one further call — `edit.keyframe {clip, param:"pos_x",
//! points:…}` makes a title/PiP/overlay FOLLOW the tracked subject.
//!
//! Engine: the perception sidecar's python + cv2 (already present via scenedetect's
//! opencv dep) running `track_runner.py` — CSRT (the accurate DCF single-object
//! tracker) with a dependency-free `matchTemplate` fallback. The one-shot CLI is the
//! SAME transport as the matte runner / perception sidecar (no port, no 2nd window).
//!
//! Dependencies: cut_core (KfPoint, CutError), cut_perception (sidecar paths),
//! serde_json. Primary caller: dispatch.rs `edit_track`.

use cut_core::{error_codes, CutError, KfPoint};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The resolved tracker runtime: the perception python + the one-shot script. Both
/// are overridable for dev (the bundled appdata venv may lack cv2; the repo dev venv
/// at `app/perception/py/.venv` carries the full stack).
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
}

/// The one-shot tracker script (ships beside `instruments.py` in the sidecar payload).
pub fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("track_runner.py"))
        .unwrap_or_else(|| PathBuf::from("track_runner.py"))
}

/// `Some` when the perception python + the tracker script exist — i.e. the box can
/// run cv2 tracking. `TRACK_RUNNER_PY` / `TRACK_RUNNER_SCRIPT` override the python /
/// script (dev points them at the repo venv). `None` → `edit.track` returns a setup
/// hint (the runner surfaces a clear error if the venv lacks cv2).
pub fn runtime() -> Option<Runtime> {
    let python = std::env::var_os("TRACK_RUNNER_PY")
        .map(PathBuf::from)
        .unwrap_or_else(|| cut_perception::sidecar_paths().0);
    let script = std::env::var_os("TRACK_RUNNER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(runner_script);
    (python.exists() && script.exists()).then_some(Runtime { python, script })
}

/// One sampled box: centre (cx,cy) + size (w,h) as FRACTIONS of the frame, at SOURCE
/// time `t_ms`. `ok=false` = the tracker lost lock on that frame (box is last-good).
#[derive(Debug, Clone, Deserialize)]
pub struct TrackPoint {
    pub t_ms: u64,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub ok: bool,
}

/// The runner's JSON output: the engine actually used, the source fps/size, the
/// fraction of in-lock samples, and the trajectory.
#[derive(Debug, Clone, Deserialize)]
pub struct TrackResult {
    pub engine: String,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub coverage: f64,
    pub points: Vec<TrackPoint>,
}

/// The seed region: an explicit box or a point (the runner grows a small box).
pub enum Seed {
    /// [x, y, w, h] as fractions of the frame (top-left origin).
    Bbox([f64; 4]),
    /// [x, y] as fractions; the runner builds a default-size box around it.
    Point([f64; 2]),
}

/// Run the tracker over the SOURCE video across `[src_start_ms, src_end_ms]`, seeded
/// by `seed`, sampling at most every `every_ms`. Spawns the one-shot runner and
/// parses its single JSON line (same transport as the matte runner).
pub fn run_tracker(
    rt: &Runtime,
    video: &Path,
    seed: &Seed,
    src_start_ms: u64,
    src_end_ms: u64,
    every_ms: u64,
    engine: &str,
) -> Result<TrackResult, CutError> {
    let mut cmd = std::process::Command::new(&rt.python);
    cmd.arg(&rt.script).arg(video);
    match seed {
        Seed::Bbox(b) => {
            cmd.arg("--bbox")
                .arg(format!("{},{},{},{}", b[0], b[1], b[2], b[3]));
        }
        Seed::Point(p) => {
            cmd.arg("--point").arg(format!("{},{}", p[0], p[1]));
        }
    }
    cmd.arg("--start-ms")
        .arg(src_start_ms.to_string())
        .arg("--end-ms")
        .arg(src_end_ms.to_string())
        .arg("--every-ms")
        .arg(every_ms.to_string())
        .arg("--engine")
        .arg(engine);
    let out = cmd.output().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("track runner spawn failed: {e}"),
            "the local motion-tracking runtime could not be started",
        )
        .with_suggested_action(
            "install the perception sidecar (system.doctor / system.setup_*) so its python+cv2 is available",
        )
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("track runner failed: {}", err.trim()),
            "the local tracking runtime errored (is cv2 installed in the perception venv?)",
        ));
    }
    // The runner prints exactly one JSON line; tolerate leading noise.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or("")
        .trim();
    serde_json::from_str(line).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("track runner output not JSON ({e}); got: {line}"),
            "the runner must print one JSON track line on stdout",
        )
    })
}

/// Ready-to-apply keyframe arrays derived from a track. `pos_x`/`pos_y` are the
/// tracked box TOP-LEFT (centre − size/2) as frame fractions — pipe them straight
/// into `edit.keyframe {param:"pos_x"|"pos_y"}` so an overlay/title FOLLOWS the
/// subject. `scale` (only when requested) is the box size relative to the FIRST
/// sample (1.0 = unchanged), for a zoom that tracks the subject's apparent size.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackKeyframes {
    pub pos_x: Vec<KfPoint>,
    pub pos_y: Vec<KfPoint>,
    pub scale: Option<Vec<KfPoint>>,
}

/// Convert a SOURCE-time track into CLIP-LOCAL keyframe arrays. Source time maps to
/// clip-local time by subtracting the clip's `src_in_ms` and dividing by `speed`
/// (matching the renderer's source↔timeline mapping). `track_scale` adds the scale
/// channel (box size relative to the first sample). Points before `src_in_ms` clamp
/// to t=0. Pure (no I/O) → unit-tested.
pub fn to_keyframes(
    res: &TrackResult,
    src_in_ms: u64,
    speed: f64,
    track_scale: bool,
) -> TrackKeyframes {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    let local_t = |t_ms: u64| -> u64 {
        let src_off = t_ms.saturating_sub(src_in_ms) as f64;
        (src_off / speed).round() as u64
    };
    let mut pos_x = Vec::new();
    let mut pos_y = Vec::new();
    for p in &res.points {
        let t = local_t(p.t_ms);
        // Box TOP-LEFT = centre − half-size (aligns a same-size overlay to the region).
        pos_x.push(KfPoint {
            t_ms: t,
            value: p.cx - p.w / 2.0,
        });
        pos_y.push(KfPoint {
            t_ms: t,
            value: p.cy - p.h / 2.0,
        });
    }
    let scale = if track_scale {
        let w0 = res.points.first().map(|p| p.w).filter(|w| *w > 0.0);
        w0.map(|w0| {
            res.points
                .iter()
                .map(|p| KfPoint {
                    t_ms: local_t(p.t_ms),
                    value: p.w / w0,
                })
                .collect()
        })
    } else {
        None
    };
    TrackKeyframes {
        pos_x,
        pos_y,
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(points: Vec<(u64, f64, f64, f64, f64, bool)>) -> TrackResult {
        TrackResult {
            engine: "csrt".into(),
            fps: 30.0,
            width: 640,
            height: 360,
            coverage: 1.0,
            points: points
                .into_iter()
                .map(|(t_ms, cx, cy, w, h, ok)| TrackPoint {
                    t_ms,
                    cx,
                    cy,
                    w,
                    h,
                    ok,
                })
                .collect(),
        }
    }

    /// Centre→top-left, clip-local time, and the optional scale channel.
    #[test]
    fn to_keyframes_maps_centre_time_and_scale() {
        let r = res(vec![
            (0, 0.20, 0.50, 0.10, 0.10, true),
            (1000, 0.60, 0.50, 0.20, 0.10, true),
        ]);
        // src_in 0, speed 1, no scale.
        let kf = to_keyframes(&r, 0, 1.0, false);
        assert_eq!(kf.pos_x.len(), 2);
        // top-left x = cx - w/2 = 0.20-0.05 = 0.15 ; 0.60-0.10 = 0.50.
        assert!((kf.pos_x[0].value - 0.15).abs() < 1e-9);
        assert!((kf.pos_x[1].value - 0.50).abs() < 1e-9);
        assert!((kf.pos_y[0].value - 0.45).abs() < 1e-9);
        assert_eq!(kf.pos_x[1].t_ms, 1000);
        assert!(kf.scale.is_none(), "scale only when requested");

        // With scale: w 0.10→0.20 ⇒ 1.0→2.0 relative.
        let kf2 = to_keyframes(&r, 0, 1.0, true);
        let s = kf2.scale.unwrap();
        assert!((s[0].value - 1.0).abs() < 1e-9);
        assert!((s[1].value - 2.0).abs() < 1e-9);
    }

    /// Source time maps to clip-local by subtracting src_in and dividing by speed.
    #[test]
    fn to_keyframes_applies_src_in_and_speed() {
        let r = res(vec![
            (2000, 0.50, 0.50, 0.10, 0.10, true),
            (4000, 0.50, 0.50, 0.10, 0.10, true),
        ]);
        // src_in 2000 ⇒ first point is clip-local 0; speed 2.0 halves the offset.
        let kf = to_keyframes(&r, 2000, 2.0, false);
        assert_eq!(kf.pos_x[0].t_ms, 0);
        // (4000-2000)/2 = 1000.
        assert_eq!(kf.pos_x[1].t_ms, 1000);
        // A point BEFORE src_in clamps to t=0 (no underflow).
        let r2 = res(vec![(500, 0.5, 0.5, 0.1, 0.1, true)]);
        let kf2 = to_keyframes(&r2, 2000, 1.0, false);
        assert_eq!(kf2.pos_x[0].t_ms, 0);
    }
}
