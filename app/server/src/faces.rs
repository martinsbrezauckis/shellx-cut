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
    native_context: bool,
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
pub fn runtime() -> Result<Option<Runtime>, CutError> {
    let sidecar = cut_perception::sidecar_runtime()?;
    Ok(runtime_from_sidecar(
        sidecar,
        std::env::var_os("FACE_RUNNER_PY").map(PathBuf::from),
        std::env::var_os("FACE_RUNNER_SCRIPT").map(PathBuf::from),
    ))
}

fn runtime_from_sidecar(
    sidecar: cut_perception::SidecarRuntime,
    override_python: Option<PathBuf>,
    override_script: Option<PathBuf>,
) -> Option<Runtime> {
    let native_context = sidecar.native_context.is_some();
    let (python, script) = if native_context {
        // The Runner context pins the interpreter. The YuNet script/model stay
        // in the application inventory, so no FACE_* override may replace
        // either selection for an admitted native run.
        let script = sidecar
            .script
            .parent()
            .map(|dir| dir.join("face_runner.py"))
            .unwrap_or_else(|| PathBuf::from("face_runner.py"));
        (sidecar.python, script)
    } else {
        let python = override_python.unwrap_or(sidecar.python);
        let script = override_script.unwrap_or_else(runner_script);
        (python, script)
    };
    (python.exists() && script.exists()).then_some(Runtime {
        python,
        script,
        native_context,
    })
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
    #[serde(rename = "width")]
    _width: u32,
    #[serde(rename = "height")]
    _height: u32,
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
    cut_perception::apply_python_command_policy(&mut cmd, rt.native_context);
    cmd.arg(&rt.script)
        .arg(video)
        .arg("--at-ms")
        .arg(at_ms.to_string());
    if track {
        cmd.arg("--track"); // CSRT-track each face so a moving face stays covered.
    }
    let out =
        crate::dispatch::run_bounded_foreground_command(&mut cmd, "face runner").map_err(|e| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_context_keeps_bundled_face_script_and_pinned_python() {
        let temp = tempfile::tempdir().unwrap();
        let bundled = temp.path().join("bundled");
        let native = temp.path().join("native");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        let script = bundled.join("instruments.py");
        let face_script = bundled.join("face_runner.py");
        let python = native.join("python");
        let override_python = temp.path().join("override-python");
        let override_script = temp.path().join("override-face.py");
        for path in [
            &script,
            &face_script,
            &python,
            &override_python,
            &override_script,
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }
        let runtime = runtime_from_sidecar(
            cut_perception::SidecarRuntime {
                python: python.clone(),
                script,
                native_context: Some(native_context_fixture(&native, &python)),
            },
            Some(override_python),
            Some(override_script),
        )
        .unwrap();
        assert_eq!(runtime.python, python);
        assert_eq!(runtime.script, face_script);
    }

    #[test]
    fn normal_runtime_keeps_existing_face_override_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let default_python = temp.path().join("default-python");
        let bundled = temp.path().join("bundled");
        let script = bundled.join("instruments.py");
        let override_python = temp.path().join("override-python");
        let override_script = temp.path().join("override-face.py");
        std::fs::create_dir_all(&bundled).unwrap();
        for path in [&default_python, &script, &override_python, &override_script] {
            std::fs::write(path, b"fixture").unwrap();
        }
        let runtime = runtime_from_sidecar(
            cut_perception::SidecarRuntime {
                python: default_python,
                script,
                native_context: None,
            },
            Some(override_python.clone()),
            Some(override_script.clone()),
        )
        .unwrap();
        assert_eq!(runtime.python, override_python);
        assert_eq!(runtime.script, override_script);
    }

    fn native_context_fixture(
        root: &Path,
        python: &Path,
    ) -> cut_native_runtime_context::RuntimeContext {
        cut_native_runtime_context::RuntimeContext {
            schema: cut_native_runtime_context::CONTEXT_CONTRACT.into(),
            root: root.to_path_buf(),
            manifest_sha256: "a".repeat(64),
            receipt_sha256: "b".repeat(64),
            interpreter: cut_native_runtime_context::Interpreter {
                path: python.to_path_buf(),
                sha256: "c".repeat(64),
                version: "3.12.13".into(),
            },
            imports: vec![],
            models: vec![],
            files: 1,
            total_bytes: 7,
        }
    }
}
