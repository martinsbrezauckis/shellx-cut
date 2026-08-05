//! perception_setup.rs — `system.setup_perception` consented runtime provisioner.
//!
//! ROLE
//!   The perception sidecar (transcription + silence/scenes/beats + auto-reframe)
//!   needs a Python venv. The installer bundles only the SCRIPT (instruments.py +
//!   requirements.txt); the heavy venv is provisioned here, on first run, with the
//!   user's consent — exactly like ffmpeg (fetch.rs), and for the same reasons
//!   (small installer, no multi-GB payload shipped, deps land on the USER's disk).
//!
//!   The hard part this solves: system Python on real desktops is too old
//!   (macOS ships 3.9; onnx-asr — the Parakeet STT engine — needs >=3.10) and
//!   unpinned. So we DON'T assume a system python. We fetch `uv` (Astral's single
//!   static binary, sha256-verified via the fetch.rs allow-list), have it install
//!   a standalone CPython 3.12, create the venv with it, and `uv pip install -r`
//!   the bundled, pinned requirements. The sidecar resolver then finds this venv
//!   at the app-data perception dir (cut_perception::appdata_sidecar_dir).
//!
//! SECURITY
//!   - uv is downloaded ONLY through fetch::install_tool — the same pinned-host,
//!     sha256-verified, staged-then-atomic path as ffmpeg. No caller-supplied URL.
//!   - The requirements file is the one bundled BESIDE instruments.py (resolved by
//!     cut_perception::sidecar_paths), never a caller path.
//!   - We invoke uv with explicit args (no shell), and only ever target the
//!     app-data perception venv dir.
//!
//! Dependencies: fetch.rs (uv download), cut-media toolpath (uv install dir),
//! cut-perception (venv dir + requirements path), jobs.rs (progress via the
//! shared ProgressFn). Primary caller: dispatch.rs (system.setup_perception).

use crate::fetch::{self, ProgressFn};
use cut_core::{error_codes, CutError};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Env override: use an EXISTING uv binary instead of downloading one. Test/dev
/// seam (the build box already has uv) — never a verb arg.
pub const ENV_UV: &str = "SHELLX_CUT_UV";

/// Exact Python version the sidecar venv is built on. CPython 3.12 has wheels
/// for every dep (onnx-asr/onnxruntime, torch, mediapipe) on win/linux/mac and
/// satisfies the onnx-asr >=3.10 floor.
///
/// Keep all three components pinned. On Windows, `uv venv --python 3.12`
/// records uv's floating `cpython-3.12-windows-x86_64-none` junction in
/// `pyvenv.cfg`. Windows restricted tokens (including WSL-launched native
/// qualification and agent processes) reject that junction as an untrusted
/// mount point after setup. An exact patch version binds the venv directly to
/// the versioned managed interpreter directory and remains usable in every
/// supported control path.
const PYTHON_VERSION: &str = "3.12.13";

/// CPU-only PyTorch wheel index for perception extras. A fresh user box has no
/// NVIDIA GPU and no reason to pull the ~2 GB CUDA build + nvidia-* deps that the
/// default PyPI torch drags in on Linux; the +cpu wheels here are small and exist
/// for win/mac/linux. Used only for the best-effort extras phases below.
const TORCH_CPU_INDEX: &str = "https://download.pytorch.org/whl/cpu";

/// Extras the UI and release gate rely on for local, non-agent perception:
/// Canary alignment, face/OCR redaction, auto-reframe/director sheets, scenes,
/// silence, and beats. Installed before the heavier optional fallbacks so resolver
/// churn there cannot block the normal user-facing tool setup.
const CORE_PERCEPTION_EXTRAS: &[&str] = &[
    "numpy<2.5",
    "torch",
    "torchvision",
    "torchaudio",
    "silero-vad",
    "soundfile",
    "scenedetect",
    "supervision",
    "rapidocr-onnxruntime",
    "sentencepiece",
];

/// Nice-to-have extras. The primary STT path is onnx-asr; MediaPipe face framing
/// is a precision enhancer with a saliency fallback. These must never block the
/// core sidecar setup.
const OPTIONAL_PERCEPTION_EXTRAS: &[&str] = &["whisperx", "mediapipe"];

/// Result of a successful provisioning — becomes the job result + drives the
/// doctor re-scan (the sidecar card flips missing → ready).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SetupOutcome {
    /// Absolute path to the venv python that now runs the sidecar.
    pub venv_python: String,
    /// uv version used.
    pub uv_version: Option<String>,
    /// True when the Parakeet model was pre-fetched (first transcribe is instant).
    pub model_warmed: bool,
    /// Whether onnx-asr imports in the new venv (the STT engine is ready). This is
    /// the CRITICAL outcome: when true, transcription works on the user's box.
    pub onnx_asr_ready: bool,
    /// Whether the FULL perception extras (whisperX fallback, auto-reframe detector,
    /// face framing, beat grid, OCR) installed. BEST-EFFORT: false here does NOT mean
    /// setup failed — transcription still works on the onnx-asr base; the richer
    /// instruments simply fall back to their ffmpeg/saliency paths.
    pub full_perception_ready: bool,
    /// Human note about the extras outcome (e.g. why they were skipped) — surfaced
    /// for the audit trail; empty when everything installed.
    pub extras_note: Option<String>,
}

/// Provision the perception venv. BLOCKING — call from a spawn_blocking task.
/// `warm_model` pre-downloads the Parakeet ONNX model so the first transcription
/// is instant (otherwise it lazy-downloads on first use). Every error is
/// actionable; nothing is left half-installed that the resolver would trust
/// (uv writes the venv atomically enough that a failed pip leaves an obviously
/// incomplete venv the doctor still reports as "deps missing").
pub fn setup_perception(warm_model: bool, progress: &ProgressFn) -> Result<SetupOutcome, CutError> {
    // ---- 0. locate the bundled requirements + the venv target ----------------
    let (_py, script) = cut_perception::sidecar_paths();
    let requirements = script
        .parent()
        .map(|d| d.join("requirements.txt"))
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            CutError::new(
                error_codes::IO,
                "perception requirements.txt not found beside instruments.py",
                format!("looked next to {}", script.display()),
            )
            .with_suggested_action("reinstall the app — the perception payload is missing")
        })?;
    // The FULL perception extras (whisperX fallback, auto-reframe detector, face
    // framing, beat grid, OCR) live in an OPTIONAL sibling file. Absent on an older
    // bundle that shipped only requirements.txt → we install just the base and the
    // extras phase is skipped (transcription still works).
    let requirements_full = script
        .parent()
        .map(|d| d.join("requirements-full.txt"))
        .filter(|p| p.is_file());
    let sidecar_dir = cut_perception::appdata_sidecar_dir().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-data perception directory (HOME/LOCALAPPDATA unset)",
            "cannot determine where to build the sidecar venv",
        )
    })?;
    std::fs::create_dir_all(&sidecar_dir).map_err(io_err("create perception dir"))?;
    let venv_dir = sidecar_dir.join(".venv");
    let venv_python = venv_python_path(&venv_dir);

    // ---- 1. uv: env override, else consented download (fetch.rs) ------------
    progress(0.02, "locating uv");
    let uv = resolve_uv(progress)?;
    let uv_version = run_capture(&uv, &["--version"])
        .ok()
        .map(|s| s.trim().to_string());

    // ---- 2. standalone CPython (uv-managed; no system python dependency) -----
    progress(0.22, "installing Python runtime");
    run_streaming(
        &uv,
        &["python", "install", PYTHON_VERSION],
        "uv python install",
        progress,
        0.22,
        0.40,
    )?;

    // ---- 3. create the venv on that interpreter ------------------------------
    progress(0.42, "creating venv");
    // A clean rebuild: remove any prior partial venv so pip starts fresh.
    let _ = std::fs::remove_dir_all(&venv_dir);
    run_streaming(
        &uv,
        &[
            "venv",
            venv_dir.to_str().unwrap_or_default(),
            "--python",
            PYTHON_VERSION,
        ],
        "uv venv",
        progress,
        0.42,
        0.48,
    )?;

    // ---- 4. install the BASE STT engine INTO that venv (MUST succeed) --------
    //
    // CRITICAL PATH: only onnx-asr (onnxruntime-based, ships prebuilt wheels for
    // win/mac/linux on cp312). `--only-binary :all:` forbids ANY source build, so a
    // fresh user box with no MSVC/LLVM can never hit pip's "build failures …"
    // dead-end — uv either installs the wheel or fails FAST with "no usable wheels"
    // (which we translate to a non-developer message). This is what makes
    // transcription "just work" on a clean machine.
    let vpy = venv_python.to_str().unwrap_or_default();
    progress(0.50, "installing the transcription engine");
    run_streaming(
        &uv,
        &[
            "pip",
            "install",
            "--python",
            vpy,
            "--only-binary",
            ":all:",
            "-r",
            requirements.to_str().unwrap_or_default(),
        ],
        "uv pip install",
        progress,
        0.50,
        0.60,
    )?;

    // ---- 5. verify onnx-asr imports (the STT engine is actually usable) ------
    progress(0.61, "verifying STT engine");
    let onnx_asr_ready = run_capture(
        &venv_python,
        &["-c", "import onnx_asr, onnxruntime; print('ok')"],
    )
    .map(|o| o.contains("ok"))
    .unwrap_or(false);

    // ---- 6. (optional) pre-fetch the Parakeet model --------------------------
    // Done BEFORE the heavy extras so the critical "engine + model" pair lands
    // first; the user can transcribe even if the extras phase is slow or skipped.
    let mut model_warmed = false;
    if warm_model && onnx_asr_ready {
        progress(0.62, "downloading Parakeet model (first run only)");
        // Loading the model with the hub downloader caches its ONNX files so the
        // first real transcription is instant. CPU provider everywhere (matches
        // instruments.py; correctness over the CoreML accel path).
        let warm = run_streaming(
            &venv_python,
            &[
                "-c",
                "import os,onnx_asr; \
                 onnx_asr.load_model(os.environ.get('SHELLX_CUT_STT_MODEL','nemo-parakeet-tdt-0.6b-v3'), \
                 providers=['CPUExecutionProvider']); print('warmed')",
            ],
            "model download",
            progress,
            0.62,
            0.76,
        );
        model_warmed = warm.is_ok();
    }

    // ---- 7. install the perception extras (BEST-EFFORT, split by risk) -------
    //
    // CORE: Canary MMS_FA alignment + auto-reframe detector + scene/silence/beat
    // grid + OCR. OPTIONAL: whisperX fallback + MediaPipe face precision. The split
    // is intentional: whisperX/pyannote resolver churn must not prevent normal
    // users from getting the compact, visible Environment tools installed. Both
    // phases force wheel-only CPU torch. A FAILURE remains NON-FATAL: the base STT
    // engine + model already work, so we record a note and continue rather than
    // throwing away a working install.
    let (full_perception_ready, extras_note) = if let Some(full) = requirements_full.as_ref() {
        progress(
            0.78,
            "installing perception tools (captions, scenes, silence, auto-reframe, OCR)",
        );
        let core_res = install_perception_packages(
            &uv,
            vpy,
            CORE_PERCEPTION_EXTRAS,
            "uv pip install core perception extras",
            progress,
            0.78,
            0.90,
        );
        match core_res {
            Ok(()) => {
                progress(0.91, "installing optional perception fallbacks");
                let optional_res = install_perception_packages(
                    &uv,
                    vpy,
                    OPTIONAL_PERCEPTION_EXTRAS,
                    "uv pip install optional perception extras",
                    progress,
                    0.91,
                    0.97,
                );
                match optional_res {
                    Ok(()) => (true, None),
                    Err(e) => {
                        let note = format!(
                            "the optional WhisperX/MediaPipe perception extras could not be installed ({}); \
                             the main perception tools are ready",
                            e.message
                        );
                        progress(0.97, &note);
                        (true, Some(note))
                    }
                }
            }
            Err(e) => {
                // Keep the working base; surface WHY the extras were skipped.
                let note = format!(
                    "the perception tools could not be installed ({}); \
                     transcription still works on the built-in engine. \
                     Bundled extras marker: {}",
                    e.message,
                    full.display()
                );
                progress(0.97, &note);
                (false, Some(note))
            }
        }
    } else {
        // Older bundle without the extras file — base-only is a valid, working state.
        (
            false,
            Some(
                "perception extras file not shipped in this build; \
                      transcription engine installed"
                    .to_string(),
            ),
        )
    };

    progress(1.0, "perception ready");
    Ok(SetupOutcome {
        venv_python: venv_python.display().to_string(),
        uv_version,
        model_warmed,
        onnx_asr_ready,
        full_perception_ready,
        extras_note,
    })
}

fn install_perception_packages(
    uv: &Path,
    vpy: &str,
    packages: &[&str],
    label: &'static str,
    progress: &ProgressFn,
    band_lo: f32,
    band_hi: f32,
) -> Result<(), CutError> {
    let mut args = vec![
        "pip",
        "install",
        "--python",
        vpy,
        "--only-binary",
        ":all:",
        "--extra-index-url",
        TORCH_CPU_INDEX,
        "--index-strategy",
        "unsafe-best-match",
    ];
    args.extend_from_slice(packages);
    run_streaming(uv, &args, label, progress, band_lo, band_hi)
}

// ===========================================================================
// The MATANYONE2 premium runtime provisioner (`system.setup_matte{model:matanyone}`)
// ===========================================================================
//
// Same uv-provisioned pattern as setup_perception, but builds a SEPARATE, isolated
// torch venv (kept apart from the perception venv so the premium tier can never
// perturb transcription/captions/reframe). Installs cu128 torch + the curated
// INFERENCE-ONLY deps (no thinplate[train]/PySide6/gradio/tensorboard) + the
// matanyone2 package (git, --no-deps), then fetches the 135 MB checkpoint. The
// caller gates this behind explicit NON-COMMERCIAL consent (NTU S-Lab License 1.0).

/// PyTorch CUDA wheels index for the pinned torch 2.8.0+cu128 configuration.
/// NVIDIA-targeted (the premium tier is GPU-realistic; CPU torch = unusably slow).
const TORCH_CU128_INDEX: &str = "https://download.pytorch.org/whl/cu128";

/// MatAnyone2's INFERENCE-ONLY dependencies (curated set; the upstream
/// core deps drag thinplate[training]/PySide6/gradio/tensorboard/cchardet we don't
/// need and that break a clean install).
const MATANYONE_DEPS: &[&str] = &[
    "opencv-python-headless",
    "tqdm",
    "imageio",
    "imageio-ffmpeg",
    "numpy",
    "Pillow",
    "hydra-core",
    "omegaconf",
    "einops",
    "kornia",
    "safetensors",
    "huggingface_hub",
    "requests",
    "av",
    "scipy",
    "gdown",
    "gitpython",
    "easydict",
];

/// The matanyone2 source, pinned to a validated commit. We install it EDITABLE
/// from a local extraction (NOT `git+`): the upstream wheel build hits a hatchling
/// `force-include` bug (it duplicates `matanyone2/config/__init__.py`), and an
/// editable install from a local dir skips the wheel build entirely.
const MATANYONE_SRC_URL: &str =
    "https://github.com/pq-yang/MatAnyone2/archive/e3370127319c63a6dc8a49c69de2d41d90137f91.tar.gz";

/// Fetch + extract the matanyone2 source with the venv's OWN python (stdlib
/// urllib+tarfile — needs NO system git or tar, so it's portable). argv: url, dest.
/// Extracts the single `MatAnyone2-<sha>/` top dir to `dest` (cleared first).
/// (Raw string so Python's indentation survives verbatim.)
const FETCH_SRC_PY: &str = r#"
import sys, os, shutil, tempfile, urllib.request, tarfile
url, dest = sys.argv[1], sys.argv[2]
with tempfile.TemporaryDirectory() as td:
    tgz = os.path.join(td, 's.tar.gz')
    urllib.request.urlretrieve(url, tgz)
    with tarfile.open(tgz) as t:
        t.extractall(td, filter='data')
    tops = [d for d in os.listdir(td) if os.path.isdir(os.path.join(td, d))]
    top = os.path.join(td, tops[0])
    if os.path.exists(dest):
        shutil.rmtree(dest)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    shutil.move(top, dest)
print('extracted', dest)
"#;

/// SAM2 (Apache-2.0) — the click-to-pick-subject seed for the premium matte.
/// Pinned to a validated commit and installed with the perception environment.
const SAM2_GIT: &str =
    "git+https://github.com/facebookresearch/sam2.git@2b90b9f5ceec907a1c18123530e92e794ad901a4";

/// The SAM2 weights (HF, ~80 MB, Apache-2.0), pre-fetched into `<matanyone>/hf` at
/// a pinned revision so `sam2_runner.py` loads them offline deterministically.
const SAM2_HF_ID: &str = "facebook/sam2-hiera-base-plus";
const SAM2_HF_REVISION: &str = "98efa66555fceff5f74ad281fb8003536dcfb6ff";

/// Pre-fetch the SAM2 weights into a controlled HF cache (argv: hf_home, repo_id, revision).
const PREFETCH_SAM2_PY: &str = r#"
import os, sys
os.environ['HF_HOME'] = sys.argv[1]
from huggingface_hub import snapshot_download
snapshot_download(sys.argv[2], revision=sys.argv[3])
print('sam2 weights fetched to', sys.argv[1])
"#;

/// Result of a successful premium provisioning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MatanyoneSetupOutcome {
    pub venv_python: String,
    pub checkpoint: String,
    pub uv_version: Option<String>,
    /// Whether `import matanyone2, torch` succeeds in the new venv.
    pub matanyone_ready: bool,
    /// Whether torch reports a usable CUDA device (false = CPU-only, slow).
    pub cuda_available: bool,
}

/// Provision the premium MatAnyone2 venv + checkpoint. BLOCKING — call from a
/// spawn_blocking task. NON-COMMERCIAL consent is enforced by the caller. Nothing
/// half-installed is trusted (the doctor re-scan reports the real state).
pub fn setup_matanyone(progress: &ProgressFn) -> Result<MatanyoneSetupOutcome, CutError> {
    let venv_dir = crate::matte::matanyone_venv_dir().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-data matanyone directory (HOME/LOCALAPPDATA unset)",
            "cannot determine where to build the premium venv",
        )
    })?;
    if let Some(parent) = venv_dir.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create matanyone dir"))?;
    }
    let venv_python = venv_python_path(&venv_dir);

    // ---- 1. uv (env override or consented download) --------------------------
    progress(0.02, "locating uv");
    let uv = resolve_uv(progress)?;
    let uv_version = run_capture(&uv, &["--version"])
        .ok()
        .map(|s| s.trim().to_string());

    // ---- 2. standalone CPython 3.12 (MatAnyone2 runtime) ---------------------
    progress(0.10, "installing Python runtime");
    run_streaming(
        &uv,
        &["python", "install", PYTHON_VERSION],
        "uv python install",
        progress,
        0.10,
        0.16,
    )?;

    // ---- 3. the isolated venv (clean rebuild) --------------------------------
    progress(0.17, "creating premium venv");
    let _ = std::fs::remove_dir_all(&venv_dir);
    run_streaming(
        &uv,
        &[
            "venv",
            venv_dir.to_str().unwrap_or_default(),
            "--python",
            PYTHON_VERSION,
        ],
        "uv venv",
        progress,
        0.17,
        0.20,
    )?;

    let vpy = venv_python.to_str().unwrap_or_default();

    // ---- 4. cu128 torch (the big one, ~3 GB) ---------------------------------
    progress(
        0.22,
        "installing PyTorch (CUDA, ~3 GB — this can take a few minutes)",
    );
    run_streaming(
        &uv,
        &[
            "pip",
            "install",
            "--python",
            vpy,
            "torch==2.8.0",
            "torchvision==0.23.0",
            "--index-url",
            TORCH_CU128_INDEX,
        ],
        "uv pip install torch",
        progress,
        0.22,
        0.55,
    )?;

    // ---- 5. curated inference deps -------------------------------------------
    progress(0.56, "installing MatAnyone2 dependencies");
    let mut deps_args: Vec<&str> = vec!["pip", "install", "--python", vpy];
    deps_args.extend_from_slice(MATANYONE_DEPS);
    run_streaming(&uv, &deps_args, "uv pip install deps", progress, 0.56, 0.70)?;

    // ---- 6. the matanyone2 package (pinned source → EDITABLE, no wheel build) -
    let src_dir = crate::matte::matanyone_dir()
        .map(|d| d.join("src"))
        .ok_or_else(|| {
            CutError::new(
                error_codes::IO,
                "no matanyone dir",
                "HOME/LOCALAPPDATA unset",
            )
        })?;
    let src_str = src_dir.to_str().unwrap_or_default();
    progress(0.71, "fetching MatAnyone2 source");
    run_streaming(
        &venv_python,
        &["-c", FETCH_SRC_PY, MATANYONE_SRC_URL, src_str],
        "fetch matanyone2 source",
        progress,
        0.71,
        0.76,
    )?;
    progress(0.77, "installing MatAnyone2");
    run_streaming(
        &uv,
        &[
            "pip",
            "install",
            "--python",
            vpy,
            "--no-deps",
            "-e",
            src_str,
        ],
        "uv pip install matanyone2",
        progress,
        0.77,
        0.80,
    )?;

    // ---- 7. the checkpoint (135 MB, sha-pinned) ------------------------------
    progress(0.80, "downloading the MatAnyone2 checkpoint (135 MB)");
    let checkpoint = crate::matte::install_matanyone_model(&|f, m| progress(0.80 + f * 0.08, m))?;

    // ---- 7b. SAM2 (Apache-2.0) — the click-to-pick-subject seed ---------------
    progress(0.88, "installing SAM2 (pick-which-subject)");
    run_streaming(
        &uv,
        &[
            "pip",
            "install",
            "--python",
            vpy,
            "--no-build-isolation",
            SAM2_GIT,
        ],
        "uv pip install sam2",
        progress,
        0.88,
        0.92,
    )?;
    let hf_dir = crate::matte::matanyone_dir().map(|d| d.join("hf"));
    if let Some(hf) = hf_dir.as_ref().and_then(|p| p.to_str()) {
        progress(0.93, "downloading the SAM2 weights (~80 MB)");
        run_streaming(
            &venv_python,
            &["-c", PREFETCH_SAM2_PY, hf, SAM2_HF_ID, SAM2_HF_REVISION],
            "fetch sam2 weights",
            progress,
            0.93,
            0.95,
        )?;
    }

    // ---- 8. verify the runtime imports + report CUDA -------------------------
    progress(0.96, "verifying the premium runtime");
    let probe = run_capture(
        &venv_python,
        &[
            "-c",
            "import matanyone2, sam2, torch; print('ok', torch.cuda.is_available())",
        ],
    )
    .unwrap_or_default();
    let matanyone_ready = probe.contains("ok");
    let cuda_available = probe.contains("True");

    progress(1.0, "premium background removal ready");
    Ok(MatanyoneSetupOutcome {
        venv_python: venv_python.display().to_string(),
        checkpoint: checkpoint.display().to_string(),
        uv_version,
        matanyone_ready,
        cuda_available,
    })
}

/// Platform venv python path (POSIX `.venv/bin/python`, Windows
/// `.venv\Scripts\python.exe`) — mirrors cut_perception::sidecar::venv_python.
fn venv_python_path(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// Resolve a uv binary: the SHELLX_CUT_UV override (test/dev), else a consented
/// download via the fetch allow-list (installs to `<tools>/uv/bin/uv[.exe]`).
fn resolve_uv(progress: &ProgressFn) -> Result<PathBuf, CutError> {
    if let Some(p) = std::env::var_os(ENV_UV) {
        let p = PathBuf::from(p);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    // Already installed by a prior setup? Reuse it (no re-download).
    if let Some(tools) = cut_media::toolpath::appdata_tools_dir() {
        let existing = tools.join("uv").join("bin").join(uv_exe());
        if existing.is_file() {
            return Ok(existing);
        }
    }
    // Consented download (sha256-verified, staged-then-atomic). Map its 0..1 into
    // our 0.02..0.20 band.
    fetch::install_tool("uv", &|f, m| progress(0.02 + f * 0.18, m))?;
    let tools = cut_media::toolpath::appdata_tools_dir().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no app-data tools dir after uv install",
            "HOME/LOCALAPPDATA unset",
        )
    })?;
    Ok(tools.join("uv").join("bin").join(uv_exe()))
}

fn uv_exe() -> &'static str {
    if cfg!(windows) {
        "uv.exe"
    } else {
        "uv"
    }
}

/// Run a command, streaming its stderr lines to `progress` (frac pinned at
/// `band_lo`, message = the live line) so the user sees real activity during a
/// multi-minute pip install. Returns an actionable error with the stderr tail on
/// a non-zero exit. `band_hi` is reported on success.
fn run_streaming(
    prog: &Path,
    args: &[&str],
    label: &str,
    progress: &ProgressFn,
    band_lo: f32,
    band_hi: f32,
) -> Result<(), CutError> {
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            CutError::new(
                error_codes::JOB_FAILED,
                format!("could not run {label}"),
                format!("{}: {e}", prog.display()),
            )
            .with_suggested_action("retry; if it persists, check disk space and network")
        })?;

    // Drain stderr live (uv/pip log there); keep a tail for error reporting.
    let stderr = child.stderr.take().expect("stderr piped");
    let mut tail: Vec<String> = Vec::new();
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            progress(band_lo, &format!("{label}: {trimmed}"));
            tail.push(trimmed.to_string());
            if tail.len() > 60 {
                tail.remove(0);
            }
        }
    }
    // Drain stdout too (so the pipe never blocks the child).
    if let Some(out) = child.stdout.take() {
        for _ in BufReader::new(out).lines().map_while(Result::ok) {}
    }
    let status = child.wait().map_err(|e| {
        CutError::new(
            error_codes::JOB_FAILED,
            format!("{label} wait failed"),
            e.to_string(),
        )
    })?;
    if !status.success() {
        let tail_txt = tail
            .iter()
            .rev()
            .take(12)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let (message, action) = classify_install_failure(&tail_txt, status.code());
        // Keep the raw stderr tail as the technical `cause` (audit/log), but lead
        // with a message + next step a non-developer can actually act on.
        return Err(
            CutError::new(error_codes::JOB_FAILED, message, tail_txt).with_suggested_action(action)
        );
    }
    progress(band_hi, label);
    Ok(())
}

/// Translate a uv/pip failure tail into a message + next-step that a NON-DEVELOPER
/// can act on. Raw package-manager build failures are not meaningful to a normal
/// user, so classify the common cases (no wheel for this platform, a doomed source
/// build, a network drop, low disk) and return plain-language guidance. Pure (no
/// I/O) so it is unit-testable. Returns (`message`, `suggested_action`).
fn classify_install_failure(tail: &str, exit_code: Option<i32>) -> (String, &'static str) {
    let lower = tail.to_lowercase();
    // 1. No prebuilt package for this exact machine (uv with --only-binary fails
    //    FAST here instead of trying a source build — the intended safe outcome).
    if lower.contains("no usable wheel")
        || lower.contains("no solution found")
        || lower.contains("no matching distribution")
        || lower.contains("only-binary")
        || lower.contains("are required for")
    {
        return (
            "A required component isn't available as a ready-to-install package for \
             your computer yet."
                .to_string(),
            "No action needed — ShellX Cut keeps its built-in transcription engine. \
             If transcription still shows as unavailable, choose \"Install captions\" \
             again.",
        );
    }
    // 2. A source build was attempted and failed (developer toolchain missing). With
    //    --only-binary this should no longer happen for our deps, but a stray sdist
    //    dependency could still trip it — give a real next step, not pip's jargon.
    if lower.contains("failed to build")
        || lower.contains("build failures")
        || lower.contains("metadata-generation-failed")
        || lower.contains("microsoft visual")
        || lower.contains("could not build wheels")
        || lower.contains("error: command")
    {
        return (
            "A component couldn't be prepared on this computer.".to_string(),
            "This is usually temporary. Check your internet connection and choose \
             \"Install captions\" again — you do NOT need to install any developer \
             tools.",
        );
    }
    // 3. Network interruption (the most common real-world cause of a half download).
    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("temporary failure in name resolution")
        || lower.contains("failed to resolve")
        || lower.contains("network")
        || lower.contains("ssl")
        || lower.contains("certificate")
    {
        return (
            "The download didn't finish.".to_string(),
            "This looks like a network interruption. Reconnect to the internet and \
             choose \"Install captions\" again.",
        );
    }
    // 4. Out of disk space.
    if lower.contains("no space left") || lower.contains("not enough space") {
        return (
            "There isn't enough free disk space to finish setting up perception.".to_string(),
            "Free up a few gigabytes of disk space and choose \"Install captions\" \
             again.",
        );
    }
    // 5. Anything else — still avoid raw jargon; nothing partial is trusted.
    (
        format!(
            "Setting up perception didn't finish (the installer stopped with code {}).",
            exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into())
        ),
        "Nothing half-installed is kept. Choose \"Install captions\" again; if it keeps \
         failing, check your internet connection and that you have a few gigabytes of \
         free disk space.",
    )
}

/// Run a command and return trimmed stdout (best-effort; used for version + the
/// onnx-asr import probe).
fn run_capture(prog: &Path, args: &[&str]) -> Result<String, CutError> {
    let out = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            CutError::new(
                error_codes::JOB_FAILED,
                "command failed to run",
                e.to_string(),
            )
        })?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn io_err(op: &'static str) -> impl Fn(std::io::Error) -> CutError {
    move |e| CutError::new(error_codes::IO, format!("{op} failed"), e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dev-jargon line the fresh-Windows user actually saw must NEVER reach the
    /// user verbatim; it has to map to a plain-language message + a real next step.
    #[test]
    fn reported_build_failure_line_is_humanized() {
        let raw = "uv pip install: hint: Build failures usually indicate a problem \
                   with the package or the build environment";
        let (msg, action) = classify_install_failure(raw, Some(1));
        assert!(
            !msg.to_lowercase().contains("build environment"),
            "must not echo pip's jargon as the headline: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("component"),
            "headline should be plain-language: {msg}"
        );
        assert!(
            action.to_lowercase().contains("install captions"),
            "must tell the user the concrete next step: {action}"
        );
        assert!(
            !action.to_lowercase().contains("developer tool")
                || action.to_lowercase().contains("do not"),
            "must reassure no dev tools are needed: {action}"
        );
    }

    /// `--only-binary` fails FAST with "no usable wheels" when a platform wheel is
    /// missing — that must read as a graceful fallback, not an error to act on.
    #[test]
    fn no_wheel_maps_to_graceful_fallback() {
        for raw in [
            "Because numba==0.53.1 has no usable wheels and you require numba==0.53.1",
            "error: No solution found when resolving dependencies",
            "Wheels are required for `llvmlite` because building from source is disabled",
        ] {
            let (msg, action) = classify_install_failure(raw, Some(1));
            assert!(
                msg.to_lowercase().contains("ready-to-install")
                    || msg.to_lowercase().contains("isn't available"),
                "no-wheel should read as 'not available yet': {msg}"
            );
            assert!(
                action.to_lowercase().contains("no action")
                    || action.to_lowercase().contains("built-in"),
                "no-wheel should reassure the base engine still works: {action}"
            );
        }
    }

    #[test]
    fn core_extras_are_isolated_from_optional_fallback_extras() {
        for required in [
            "torch",
            "torchvision",
            "torchaudio",
            "silero-vad",
            "scenedetect",
            "supervision",
            "rapidocr-onnxruntime",
        ] {
            assert!(
                CORE_PERCEPTION_EXTRAS.contains(&required),
                "core extras must install {required} before optional fallbacks"
            );
        }
        for optional in ["whisperx", "mediapipe"] {
            assert!(
                !CORE_PERCEPTION_EXTRAS.contains(&optional),
                "{optional} must not be able to block core perception setup"
            );
            assert!(
                OPTIONAL_PERCEPTION_EXTRAS.contains(&optional),
                "{optional} should remain available as an optional precision/fallback extra"
            );
        }
    }

    #[test]
    fn managed_python_is_patch_pinned_for_windows_venv_durability() {
        let parts = PYTHON_VERSION.split('.').collect::<Vec<_>>();
        assert_eq!(
            parts.len(),
            3,
            "uv must receive an exact patch version, not a floating Windows junction"
        );
        assert!(
            parts.iter().all(|part| part.parse::<u16>().is_ok()),
            "managed Python pin must contain only numeric version components"
        );
    }

    /// Network drops are the most common real cause of a half-finished download.
    #[test]
    fn network_failures_tell_user_to_reconnect() {
        for raw in [
            "error: failed to resolve host download.pytorch.org",
            "Connection timed out after 30000 ms",
            "SSL: CERTIFICATE_VERIFY_FAILED",
        ] {
            let (_msg, action) = classify_install_failure(raw, Some(1));
            assert!(
                action.to_lowercase().contains("internet")
                    || action.to_lowercase().contains("reconnect"),
                "network failure should point at connectivity: {action}"
            );
        }
    }

    /// Out-of-disk gets its own actionable message.
    #[test]
    fn disk_full_maps_to_free_space() {
        let (msg, action) =
            classify_install_failure("OSError: [Errno 28] No space left on device", Some(1));
        assert!(msg.to_lowercase().contains("disk space"), "{msg}");
        assert!(action.to_lowercase().contains("free up"), "{action}");
    }

    /// Unknown failures still avoid raw jargon and keep a safe next step.
    #[test]
    fn unknown_failure_is_still_actionable_without_jargon() {
        let (msg, action) = classify_install_failure("some totally novel error text", Some(7));
        assert!(
            msg.contains("code 7"),
            "should surface the exit code: {msg}"
        );
        assert!(
            action.to_lowercase().contains("install captions"),
            "should still give a retry path: {action}"
        );
    }
}
