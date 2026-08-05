//! matte.rs — server-side matte BAKE.
//!
//! Role: turn a clip's SOURCE into a baked straight-alpha matte by calling the
//! matting sidecar (RVM `POST /video/matte`), and cache the result
//! content-addressed under the project's `cache/matte/` dir. This is the ONLY
//! place that talks to the sidecar (network) — `edit.matte` triggers it (so the
//! agent gets the quality receipt immediately and the cache is warm), while
//! op-replay and the renderer stay fully OFFLINE (the renderer just reads the
//! cached `.mkv`, see render.rs `collect_graph_inputs`).
//!
//! Cache key: the alpha depends only on the source pixels + model + quality
//! (see `ClipMatte::cache_filename`), so a re-apply / re-render reuses it. The
//! whole ASSET is baked (not just the clip's range) so the render can trim the
//! alpha in lock-step with the foreground (frame-aligned alphamerge).
//!
//! Endpoint: `CUT_MATTE_ENDPOINT` (default `http://127.0.0.1:8745`) — a loopback
//! sidecar; set it before launch when the service uses a custom address.
//!
//! Dependencies: cut_core (ClipMatte, CutError), ureq (the server's blocking
//! HTTP client). Primary caller: dispatch.rs `edit_matte`.

use std::path::{Path, PathBuf};

use cut_core::{error_codes, ClipMatte, CutError, MatteModel, MatteQuality, MatteSeed};
use serde::{Deserialize, Serialize};

/// The matte quality receipt (computed by the sidecar from the alpha
/// alone): subject COVERAGE (fraction of frame matted; a sudden drop = the matte
/// lost the subject) and TEMPORAL FLICKER (mean frame-to-frame alpha delta; high
/// = a jittery edge). Cheap, model-agnostic, ships first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatteStats {
    pub frames: u64,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    /// RVM downsample ratio (RVM only; the MatAnyone2 runner omits it).
    #[serde(default)]
    pub downsample_ratio: f64,
    pub coverage_mean: f64,
    pub cov_min: f64,
    pub cov_max: f64,
    pub temporal_flicker: f64,
    /// Fraction of pixels in the soft alpha transition band (0.05..0.95) — a
    /// feathered matte (hair, fine edges) carries more than a hard cut. Emitted by
    /// the MatAnyone2 runner as an honest quality descriptor (the learned MQE is
    /// unreleased upstream → not faked). `None` for RVM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_softness: Option<f64>,
    /// Matting model that produced the alpha ("rvm" | "matanyone2"). MatAnyone2 only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Compute device the bake ran on ("cuda" | "cpu"). MatAnyone2 only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// True when this came from the content-addressed cache (no re-bake).
    #[serde(default)]
    pub cached: bool,
}

/// The matting sidecar endpoint (loopback). Override with `CUT_MATTE_ENDPOINT`.
fn endpoint() -> String {
    std::env::var("CUT_MATTE_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:8745".to_string())
}

fn io_err(ctx: &str, e: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("matte bake: {ctx}: {e}"),
        "failed writing/reading the matte cache",
    )
}

/// Ensure the baked alpha for `(asset, model, quality)` exists under
/// `project_dir/cache/matte/`, baking it via the sidecar on a cache miss.
/// Idempotent + content-addressed: a warm cache returns instantly (`cached`).
/// Returns the matte quality receipt.
pub fn ensure_baked(
    project_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
    m: &ClipMatte,
) -> Result<MatteStats, CutError> {
    let dir = project_dir.join("cache").join("matte");
    let alpha = dir.join(m.cache_filename(asset_hash));
    let stats_path = alpha.with_extension("json");

    // Cache hit: alpha + its receipt both present → no network.
    if alpha.exists() && stats_path.exists() {
        if let Ok(txt) = std::fs::read_to_string(&stats_path) {
            if let Ok(mut s) = serde_json::from_str::<MatteStats>(&txt) {
                s.cached = true;
                return Ok(s);
            }
        }
    }

    std::fs::create_dir_all(&dir).map_err(|e| io_err("create cache dir", e))?;

    // Transport, by model:
    //  - `matanyone` (PREMIUM, opt-in): the local torch runtime; the bake first
    //    seeds a first-frame subject mask via RVM (zero-click) then propagates it
    //    with MatAnyone2. Requires both the premium runtime AND the RVM runtime.
    //  - `rvm` (DEFAULT): prefer the LOCAL onnxruntime runner (shippable, cutd-
    //    managed, invisible) when installed; else the HTTP sidecar
    //    (CUT_MATTE_ENDPOINT — dev / a remote GPU box). The doctor detects + reports
    //    each runtime (the ffmpeg pattern: autodetect → setup_matte fetch / browse).
    let mut stats = match m.model {
        MatteModel::Matanyone => bake_matanyone(&dir, asset_path, asset_hash, &alpha, m)?,
        MatteModel::Rvm => match runtime() {
            Some(rt) => bake_local(&rt, asset_path, &alpha, m)?,
            None => bake_http(asset_path, &alpha, m)?,
        },
    };

    // Persist the receipt next to the alpha so a cache hit can return it.
    let _ = std::fs::write(
        &stats_path,
        serde_json::to_string(&stats).unwrap_or_default(),
    );
    stats.cached = false;
    Ok(stats)
}

/// Bake via the LOCAL one-shot CLI (`matte_runner.py`): cutd spawns it per bake
/// exactly like the perception sidecar; the runner writes the FFV1 alpha to
/// `alpha_out` and prints ONE JSON stats line on stdout. No network, no second
/// window — one app.
fn bake_local(
    rt: &MatteRuntime,
    in_path: &Path,
    alpha_out: &Path,
    m: &ClipMatte,
) -> Result<MatteStats, CutError> {
    let mut cmd = std::process::Command::new(&rt.python);
    cmd.arg(&rt.script)
        .arg(in_path)
        .arg(alpha_out)
        .arg("--model")
        .arg(&rt.model);
    if matches!(m.quality, MatteQuality::Fast) {
        cmd.arg("--downsample").arg("0.25");
    }
    let out = cmd.output().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("matte runner spawn failed: {e}"),
            "the local Background Removal runtime could not be started",
        )
        .with_suggested_action(
            "install or re-point the Background Removal tool (system.doctor / system.fetch_tool)",
        )
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("matte runner failed: {}", err.trim()),
            "the local matte runtime errored",
        ));
    }
    // The runner prints exactly one JSON stats line; tolerate any leading noise.
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
            format!("matte runner stats not JSON ({e}); got: {line}"),
            "the runner must print one JSON stats line on stdout",
        )
    })
}

/// Bake via the HTTP sidecar (`matte_service.py` at `CUT_MATTE_ENDPOINT`) — the
/// dev / remote-GPU path. POSTs the whole asset, writes the returned FFV1 alpha
/// to `alpha`, parses the `X-Matte-Stats` header.
fn bake_http(asset_path: &Path, alpha: &Path, m: &ClipMatte) -> Result<MatteStats, CutError> {
    let bytes = std::fs::read(asset_path).map_err(|e| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!(
                "matte bake: cannot read source {}: {e}",
                asset_path.display()
            ),
            "the clip's source file must exist to bake its matte",
        )
    })?;
    let mut url = format!("{}/video/matte", endpoint());
    if matches!(m.quality, MatteQuality::Fast) {
        url.push_str("?downsample_ratio=0.25");
    }
    let resp = ureq::post(&url)
        .header("content-type", "application/octet-stream")
        .send(&bytes[..])
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("matte sidecar POST failed ({url}): {e}"),
                "no local Background Removal runtime is installed and the HTTP sidecar was unreachable",
            )
            .with_suggested_action(
                "install the Background Removal tool (system.fetch_tool), or set CUT_MATTE_ENDPOINT to a running sidecar",
            )
        })?;
    let stats_hdr = resp
        .headers()
        .get("x-matte-stats")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| {
            CutError::new(
                error_codes::IO,
                "matte sidecar response missing X-Matte-Stats header",
                "the sidecar must return the quality stats header",
            )
        })?;
    // Stream the alpha (ureq's read_to_vec caps at 10 MB; a long clip's alpha
    // can exceed that).
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_body().into_reader(), &mut body)
        .map_err(|e| io_err("read alpha body", e))?;
    std::fs::write(alpha, &body).map_err(|e| io_err("write alpha", e))?;
    serde_json::from_str(&stats_hdr).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("matte stats header is not valid JSON: {e}"),
            "the sidecar returned a malformed X-Matte-Stats header",
        )
    })
}

// ---------------------------------------------------------------------------
// The LOCAL runtime: resolve / install / persist (the ffmpeg-pattern install)
// ---------------------------------------------------------------------------

/// The pinned RVM model (PeterL1n/RobustVideoMatting v1.0.0, mobilenetv3 fp32,
/// 14 MB, opset 12). We ship integration; the USER fetches the model on consent.
const RVM_URL: &str =
    "https://github.com/PeterL1n/RobustVideoMatting/releases/download/v1.0.0/rvm_mobilenetv3_fp32.onnx";
const RVM_SHA256: &str = "88d4531297118f595bf2fd60f6f566aec2e559393802d1f436c380f0cbbd2828";

/// The resolved local matte runtime (the one-shot CLI): a python carrying
/// onnxruntime, the runner script (ships beside the perception payload), and the
/// RVM model.
pub struct MatteRuntime {
    pub python: PathBuf,
    pub script: PathBuf,
    pub model: PathBuf,
}

/// Managed dir for the matte model + settings — a sibling of the perception
/// appdata dir (e.g. `…/ShellX Cut/matte`).
pub fn appdata_matte_dir() -> Option<PathBuf> {
    cut_perception::appdata_sidecar_dir().and_then(|p| p.parent().map(|d| d.join("matte")))
}

/// The runner script ships beside `instruments.py` in the python payload dir.
fn runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("matte_runner.py"))
        .unwrap_or_else(|| PathBuf::from("matte_runner.py"))
}

/// The default model location (the one-click fetch target).
fn default_model_path() -> Option<PathBuf> {
    appdata_matte_dir().map(|d| d.join("rvm.onnx"))
}

fn settings_path() -> Option<PathBuf> {
    appdata_matte_dir().map(|d| d.join("settings.json"))
}

/// Persisted model override (browse-to-existing) from `settings.json`.
pub fn read_model_setting() -> Option<PathBuf> {
    let txt = std::fs::read_to_string(settings_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
    v.get("model")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Persist (or clear, with `None`) the browse-to-existing model path.
pub fn write_model_setting(path: Option<&str>) -> Result<(), CutError> {
    let dir = appdata_matte_dir().ok_or_else(|| io_err("matte dir", "HOME/LOCALAPPDATA unset"))?;
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create matte dir", e))?;
    let body = match path {
        Some(p) => serde_json::json!({ "model": p }),
        None => serde_json::json!({}),
    };
    std::fs::write(
        dir.join("settings.json"),
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
    .map_err(|e| io_err("write matte settings", e))
}

/// Resolve the local runtime. Override order: `MATTE_*` env (dev) → the persisted
/// browse setting → the default fetched location. `Some` iff python + runner +
/// model all exist on disk.
pub fn runtime() -> Option<MatteRuntime> {
    let (perc_py, _instruments) = cut_perception::sidecar_paths();
    let python = std::env::var_os("MATTE_RUNNER_PY")
        .map(PathBuf::from)
        .unwrap_or(perc_py);
    let script = std::env::var_os("MATTE_RUNNER_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(runner_script);
    let model = std::env::var_os("MATTE_MODEL")
        .map(PathBuf::from)
        .or_else(read_model_setting)
        .or_else(default_model_path)
        .unwrap_or_default();
    (python.exists() && script.exists() && model.exists()).then_some(MatteRuntime {
        python,
        script,
        model,
    })
}

/// Download the RVM model (sha-pinned) into the matte appdata dir — the SEAMLESS
/// one-click install. No user pip: onnxruntime rides the perception venv, so the
/// whole install is "fetch one 14 MB file". Idempotent (verifies a present file).
pub fn install_model(progress: &dyn Fn(f32, &str)) -> Result<PathBuf, CutError> {
    let dir = appdata_matte_dir().ok_or_else(|| io_err("matte dir", "HOME/LOCALAPPDATA unset"))?;
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create matte dir", e))?;
    let dest = dir.join("rvm.onnx");
    if dest.exists() {
        if let Ok(bytes) = std::fs::read(&dest) {
            if hex_sha256(&bytes) == RVM_SHA256 {
                progress(1.0, "model already installed");
                return Ok(dest);
            }
        }
    }
    progress(0.05, "downloading the background-removal model (14 MB)");
    let resp = ureq::get(RVM_URL).call().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("model download failed: {e}"),
            "could not reach the RVM model release (needs internet for the one-time download)",
        )
    })?;
    // Stream the body (ureq's read_to_vec caps at 10 MB; the model is 14 MB).
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_body().into_reader(), &mut bytes)
        .map_err(|e| io_err("read model", e))?;
    let got = hex_sha256(&bytes);
    if got != RVM_SHA256 {
        return Err(CutError::new(
            error_codes::IO,
            format!("model checksum mismatch (got {got})"),
            "the downloaded model did not match the pinned hash; try the install again",
        ));
    }
    let tmp = dir.join("rvm.onnx.part");
    std::fs::write(&tmp, &bytes).map_err(|e| io_err("write model", e))?;
    std::fs::rename(&tmp, &dest).map_err(|e| io_err("place model", e))?;
    progress(1.0, "background-removal model installed");
    Ok(dest)
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

// ===========================================================================
// The optional MatAnyone2 premium runtime.
// ===========================================================================
//
// Target-assigned video matting: the first-frame mask picks WHICH subject (RVM
// mattes "the human" automatically, no choice), and a memory-propagation
// transformer gives much cleaner edges + temporal stability than RVM on hard
// real-world footage. PyTorch (NOT onnxruntime) → its OWN isolated torch venv,
// kept SEPARATE from the perception venv so the premium tier can never perturb
// transcription/captions/reframe. NVIDIA realistically (CPU = unusably slow) +
// NON-COMMERCIAL (NTU S-Lab License 1.0) → opt-in, fetch-on-consent with a
// notice. Pluggable through the SAME ClipMatte infra (`model: matanyone`): the
// cache key already includes the model, the renderer is model-blind.

/// The pinned MatAnyone2 checkpoint (pq-yang/MatAnyone2 release v1.0.0, ~135 MB).
/// We ship integration; the USER fetches the weights on consent (non-commercial).
const MATANYONE_URL: &str =
    "https://github.com/pq-yang/MatAnyone2/releases/download/v1.0.0/matanyone2.pth";
const MATANYONE_SHA256: &str = "5e9821e4087231427376b437c85bb6e072b41e582314f06fd524f75bc4af5914";

/// The resolved premium runtime: a torch python (its own isolated venv), the
/// runner script (ships beside `instruments.py`), and the MatAnyone2 checkpoint.
pub struct MatanyoneRuntime {
    pub python: PathBuf,
    pub runner: PathBuf,
    pub model: PathBuf,
}

/// venv interpreter path, cross-platform.
fn venv_python(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

/// Managed dir for the premium runtime (venv + checkpoint + settings): a sibling
/// `matanyone` dir under the matte appdata dir (e.g. `…/ShellX Cut/matte/matanyone`).
pub fn matanyone_dir() -> Option<PathBuf> {
    appdata_matte_dir().map(|d| d.join("matanyone"))
}

/// The premium venv (built by `system.setup_matte{model:matanyone}`).
pub fn matanyone_venv_dir() -> Option<PathBuf> {
    matanyone_dir().map(|d| d.join("venv"))
}

fn matanyone_venv_python() -> Option<PathBuf> {
    matanyone_venv_dir().map(|d| venv_python(&d))
}

/// The default checkpoint location (the one-click fetch target).
pub fn matanyone_default_model() -> Option<PathBuf> {
    matanyone_dir().map(|d| d.join("matanyone2.pth"))
}

/// The MatAnyone2 runner ships beside `instruments.py` in the python payload dir.
fn matanyone_runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("matanyone_runner.py"))
        .unwrap_or_else(|| PathBuf::from("matanyone_runner.py"))
}

fn matanyone_settings_path() -> Option<PathBuf> {
    matanyone_dir().map(|d| d.join("settings.json"))
}

/// Persisted browse-to-existing checkpoint override (`settings.json`).
pub fn read_matanyone_model_setting() -> Option<PathBuf> {
    let txt = std::fs::read_to_string(matanyone_settings_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&txt).ok()?;
    v.get("model")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Persist (or clear) the browse-to-existing checkpoint path.
pub fn write_matanyone_model_setting(path: Option<&str>) -> Result<(), CutError> {
    let dir = matanyone_dir().ok_or_else(|| io_err("matanyone dir", "HOME/LOCALAPPDATA unset"))?;
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create matanyone dir", e))?;
    let body = match path {
        Some(p) => serde_json::json!({ "model": p }),
        None => serde_json::json!({}),
    };
    std::fs::write(
        dir.join("settings.json"),
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
    .map_err(|e| io_err("write matanyone settings", e))
}

/// Resolve the premium runtime. Override order: `MATANYONE_*` env (dev) → the
/// persisted browse setting → the default fetched location. `Some` iff python +
/// runner + checkpoint all exist on disk.
pub fn runtime_matanyone() -> Option<MatanyoneRuntime> {
    let python = std::env::var_os("MATANYONE_PY")
        .map(PathBuf::from)
        .or_else(matanyone_venv_python)
        .unwrap_or_default();
    let runner = std::env::var_os("MATANYONE_RUNNER")
        .map(PathBuf::from)
        .unwrap_or_else(matanyone_runner_script);
    let model = std::env::var_os("MATANYONE_MODEL")
        .map(PathBuf::from)
        .or_else(read_matanyone_model_setting)
        .or_else(matanyone_default_model)
        .unwrap_or_default();
    (python.exists() && runner.exists() && model.exists()).then_some(MatanyoneRuntime {
        python,
        runner,
        model,
    })
}

/// Parse the ONE JSON stats line a runner prints on stdout (tolerate leading noise).
fn parse_stats_line(stdout: &str, ctx: &str) -> Result<MatteStats, CutError> {
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or("")
        .trim();
    serde_json::from_str(line).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("{ctx}: stats not JSON ({e}); got: {line}"),
            "the runner must print one JSON stats line on stdout",
        )
    })
}

/// The SAM2 runner ships beside `instruments.py` in the python payload dir.
fn sam2_runner_script() -> PathBuf {
    let (_py, instruments) = cut_perception::sidecar_paths();
    instruments
        .parent()
        .map(|d| d.join("sam2_runner.py"))
        .unwrap_or_else(|| PathBuf::from("sam2_runner.py"))
}

/// HF cache dir for the SAM2 weights (under the matanyone appdata dir, so the model
/// is fetched once by setup_matte and loaded offline thereafter).
fn matanyone_hf_dir() -> Option<PathBuf> {
    matanyone_dir().map(|d| d.join("hf"))
}

/// The pinned SAM2 HF revision fetched by `system.setup_matte{model:"matanyone"}`.
const SAM2_HF_REVISION: &str = "98efa66555fceff5f74ad281fb8003536dcfb6ff";

/// Resolve MatAnyone2's required first-frame subject mask: a SAM2 click/box seed
/// when the clip carries one (`pick which subject`), else the RVM auto-seed
/// (zero-click, finds "the human"). The torch runtime `rt` runs both (SAM2 lives in
/// the matanyone venv).
fn resolve_seed_mask(
    cache_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
    m: &ClipMatte,
    rt: &MatanyoneRuntime,
) -> Result<PathBuf, CutError> {
    match &m.seed {
        Some(seed) => sam2_seed_mask(cache_dir, asset_path, asset_hash, seed, rt),
        None => rvm_seed_mask(cache_dir, asset_path, asset_hash),
    }
}

/// SAM2 click/box → a binary first-frame subject mask (the premium target-selection
/// path). Runs `sam2_runner.py` in the matanyone venv; cached per-seed as
/// `{asset_hash}.{seed_hash}.seed.png` (a different pick caches separately).
fn sam2_seed_mask(
    cache_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
    seed: &MatteSeed,
    rt: &MatanyoneRuntime,
) -> Result<PathBuf, CutError> {
    let out = cache_dir.join(format!("{asset_hash}.{}.seed.png", seed.short_hash()));
    if out.exists() {
        return Ok(out);
    }
    let script = sam2_runner_script();
    if !script.exists() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "the SAM2 click-to-pick-subject runner is not installed",
            "re-run system.setup_matte{model:\"matanyone\", accept_noncommercial:true} (it installs SAM2)",
        ));
    }
    let mut cmd = std::process::Command::new(&rt.python);
    cmd.arg(&script)
        .arg(asset_path)
        .arg(&out)
        .arg("--at-ms")
        .arg(seed.at_ms.to_string());
    if let Some(hf) = matanyone_hf_dir() {
        cmd.arg("--hf-home")
            .arg(hf)
            .arg("--hf-revision")
            .arg(SAM2_HF_REVISION);
    }
    match (seed.point, seed.bbox) {
        (Some(p), _) => {
            cmd.arg("--point").arg(format!("{},{}", p[0], p[1]));
        }
        (None, Some(b)) => {
            cmd.arg("--box")
                .arg(format!("{},{},{},{}", b[0], b[1], b[2], b[3]));
        }
        (None, None) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "matte seed needs a point [x,y] or a box [x,y,w,h]",
                "pass seed.point or seed.bbox to pick the subject",
            ));
        }
    }
    let o = cmd
        .output()
        .map_err(|e| io_err("sam2 seed: spawn runner", e))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("SAM2 seed generation failed: {}", err.trim()),
            "SAM2 could not produce the subject mask from the click/box",
        ));
    }
    Ok(out)
}

/// Seed MatAnyone2's first-frame subject mask by running RVM on frame 0 (the
/// zero-click bootstrap). Cached as `cache/matte/{asset_hash}.seed.png` (depends
/// only on the source). MatAnyone2 erodes/dilates the seed → a coarse mask is fine.
fn rvm_seed_mask(
    cache_dir: &Path,
    asset_path: &Path,
    asset_hash: &str,
) -> Result<PathBuf, CutError> {
    let seed = cache_dir.join(format!("{asset_hash}.seed.png"));
    if seed.exists() {
        return Ok(seed);
    }
    let rvm = runtime().ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "MatAnyone2 needs the RVM model to seed its first-frame subject mask, but RVM is not installed",
            "run system.setup_matte to install the default RVM model — the premium tier seeds its mask from it",
        )
    })?;
    let out = std::process::Command::new(&rvm.python)
        .arg(&rvm.script)
        .arg(asset_path)
        .arg("--first-frame-mask")
        .arg(&seed)
        .arg("--model")
        .arg(&rvm.model)
        .output()
        .map_err(|e| io_err("seed mask: spawn RVM runner", e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("seed mask generation failed: {}", err.trim()),
            "RVM could not produce the first-frame mask for the premium matte",
        ));
    }
    Ok(seed)
}

/// Bake via the premium MatAnyone2 runtime: seed the first-frame mask (RVM), then
/// propagate it with MatAnyone2 (`matanyone_runner.py`). Writes the FFV1 gray alpha
/// to `alpha_out` — the SAME cache contract as RVM, so the renderer stays model-blind.
fn bake_matanyone(
    cache_dir: &Path,
    in_path: &Path,
    asset_hash: &str,
    alpha_out: &Path,
    m: &ClipMatte,
) -> Result<MatteStats, CutError> {
    let rt = runtime_matanyone().ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the premium MatAnyone2 background-removal runtime is not installed",
            "run system.setup_matte{model:\"matanyone\"} to install it (opt-in, NVIDIA, non-commercial)",
        )
    })?;
    let seed = resolve_seed_mask(cache_dir, in_path, asset_hash, m, &rt)?;
    // VRAM/quality: Good = up to 1080-px min side, Fast = 720. The alpha is always
    // upscaled back to source res inside the runner (frame-aligned alphamerge).
    let max_size = if matches!(m.quality, MatteQuality::Fast) {
        "720"
    } else {
        "1080"
    };
    let out = std::process::Command::new(&rt.python)
        .arg(&rt.runner)
        .arg(in_path)
        .arg(alpha_out)
        .arg("--mask")
        .arg(&seed)
        .arg("--model")
        .arg(&rt.model)
        .arg("--max-size")
        .arg(max_size)
        .output()
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                format!("matanyone runner spawn failed: {e}"),
                "the premium MatAnyone2 runtime could not be started",
            )
            .with_suggested_action("re-run system.setup_matte{model:\"matanyone\"}")
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(CutError::new(
            error_codes::IO,
            format!("matanyone runner failed: {}", err.trim()),
            "the premium MatAnyone2 matte runtime errored (often VRAM — try quality:\"fast\")",
        ));
    }
    parse_stats_line(&String::from_utf8_lossy(&out.stdout), "matanyone runner")
}

/// Download the MatAnyone2 checkpoint (sha-pinned) into the matanyone appdata dir.
/// The weights are NON-COMMERCIAL (NTU S-Lab License 1.0) — the caller
/// (`system.setup_matte{model:matanyone}`) gates this behind EXPLICIT consent.
/// Idempotent (verifies a present file). Streaming read (ureq's read_to_vec caps
/// at 10 MB; the checkpoint is ~135 MB).
pub fn install_matanyone_model(progress: &dyn Fn(f32, &str)) -> Result<PathBuf, CutError> {
    let dir = matanyone_dir().ok_or_else(|| io_err("matanyone dir", "HOME/LOCALAPPDATA unset"))?;
    std::fs::create_dir_all(&dir).map_err(|e| io_err("create matanyone dir", e))?;
    let dest = dir.join("matanyone2.pth");
    if dest.exists() {
        if let Ok(bytes) = std::fs::read(&dest) {
            if hex_sha256(&bytes) == MATANYONE_SHA256 {
                progress(1.0, "checkpoint already installed");
                return Ok(dest);
            }
        }
    }
    progress(0.05, "downloading the MatAnyone2 checkpoint (135 MB)");
    let resp = ureq::get(MATANYONE_URL).call().map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!("checkpoint download failed: {e}"),
            "could not reach the MatAnyone2 release (needs internet for the one-time download)",
        )
    })?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_body().into_reader(), &mut bytes)
        .map_err(|e| io_err("read checkpoint", e))?;
    let got = hex_sha256(&bytes);
    if got != MATANYONE_SHA256 {
        return Err(CutError::new(
            error_codes::IO,
            format!("checkpoint checksum mismatch (got {got})"),
            "the downloaded checkpoint did not match the pinned hash; try the install again",
        ));
    }
    let tmp = dir.join("matanyone2.pth.part");
    std::fs::write(&tmp, &bytes).map_err(|e| io_err("write checkpoint", e))?;
    std::fs::rename(&tmp, &dest).map_err(|e| io_err("place checkpoint", e))?;
    progress(1.0, "MatAnyone2 checkpoint installed");
    Ok(dest)
}
