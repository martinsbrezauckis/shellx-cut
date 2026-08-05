//! faces.rs — the local FACE-DETECT runtime for `edit.redact{faces}` (auto-blur
//! people's faces). Spawns the bundled YuNet `face_runner.py` on one frame and parses
//! its face boxes; the dispatch handler turns them into a multi-region redact.
//!
//! Mirror of `ocr.rs` — same one-shot transport (no port, no 2nd window), same env
//! overrides (`FACE_RUNNER_PY` / `FACE_RUNNER_SCRIPT` point at the dev venv + repo
//! script). The YuNet model ships beside `face_runner.py`, so a cold install needs no
//! download. Detection runs ONCE here; the committed op stores the resolved rects, so
//! replay is face-detector-free + deterministic.

use cut_core::{error_codes, CutError};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The resolved face-detect runtime: the perception python + the one-shot script.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python: PathBuf,
    pub script: PathBuf,
}

/// The one-shot face script (ships beside `instruments.py` in the sidecar payload).
pub fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("face_runner.py"))
        .unwrap_or_else(|| PathBuf::from("face_runner.py"))
}

/// `Some` when the perception python + the face script exist. `None` → `faces`
/// returns a setup hint. The runner surfaces a crisp error if opencv is missing.
pub fn runtime() -> Option<Runtime> {
    let python = std::env::var_os("FACE_RUNNER_PY")
        .map(PathBuf::from)
        .unwrap_or_else(|| cut_perception::sidecar_paths().0);
    let script = std::env::var_os("FACE_RUNNER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(runner_script);
    (python.exists() && script.exists()).then_some(Runtime { python, script })
}

/// One point of a face's motion track: the face centre over clip-local time
/// (fractions). The size stays the seed box's (MaskTrackPoint is centre-only).
#[derive(Debug, Clone, Deserialize)]
pub struct FaceTrackPt {
    pub t_ms: u64,
    pub cx: f64,
    pub cy: f64,
}

/// One detected face: centre/size as FRACTIONS of the frame (already margin-expanded
/// by the runner), the YuNet confidence, and an optional CSRT motion track.
#[derive(Debug, Clone, Deserialize)]
pub struct FaceBox {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub conf: f64,
    /// Present when detected with `--track`: the face centre over time so a
    /// MOVING face stays covered. Mapped to the region's MaskTrackPoint track.
    #[serde(default)]
    pub track: Option<Vec<FaceTrackPt>>,
}

/// The runner's JSON output (`width`/`height` informational — boxes are fractions).
#[derive(Debug, Clone, Deserialize)]
pub struct FaceResult {
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    pub boxes: Vec<FaceBox>,
}

/// Detect faces in the frame at `at_ms` of `video` via the one-shot runner (same
/// transport as the OCR / matte / track runners). Parses the single JSON line.
pub fn run_faces(
    rt: &Runtime,
    video: &Path,
    at_ms: u64,
    track: bool,
) -> Result<FaceResult, CutError> {
    let mut cmd = std::process::Command::new(&rt.python);
    cmd.arg(&rt.script)
        .arg(video)
        .arg("--at-ms")
        .arg(at_ms.to_string());
    if track {
        cmd.arg("--track"); // CSRT-track each face so a moving face stays covered.
    }
    let out = cmd.output().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("face runner spawn failed: {e}"),
            "the local face-detect runtime could not be started",
        )
        .with_suggested_action("install the perception sidecar (opencv) in its venv")
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("face runner failed: {}", err.trim()),
            "the local face-detect runtime errored (is opencv installed in the perception venv?)",
        ));
    }
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
            format!("face runner output not JSON ({e}); got: {line}"),
            "the runner must print one JSON line on stdout",
        )
    })
}

/// A detected face box → an axis-aligned rect as `[[x0,y0],[x1,y1]]` (frame fractions,
/// clamped to 0..1). Used to build the multi-region redact.
pub fn box_to_rect(b: &FaceBox) -> [[f64; 2]; 2] {
    let x0 = (b.cx - b.w / 2.0).clamp(0.0, 1.0);
    let y0 = (b.cy - b.h / 2.0).clamp(0.0, 1.0);
    let x1 = (b.cx + b.w / 2.0).clamp(0.0, 1.0);
    let y1 = (b.cy + b.h / 2.0).clamp(0.0, 1.0);
    [[x0, y0], [x1, y1]]
}
