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
//!   static binary, sha256-verified via the fetch.rs allow-list), have it select
//!   a standalone compatible CPython 3.12 patch, resolve its concrete executable,
//!   create the venv with that executable, and `uv pip install -r` the bundled
//!   perception requirement policy. The sidecar resolver then finds this venv at
//!   the app-data perception dir (cut_perception::appdata_sidecar_dir).
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
use std::path::{Path, PathBuf};
use std::process::Command;

/// Env override: use an EXISTING uv binary instead of downloading one. Test/dev
/// seam (the build box already has uv) — never a verb arg.
pub const ENV_UV: &str = "SHELLX_CUT_UV";

/// Python compatibility request for the sidecar venv. CPython 3.12 has wheels
/// for every dep (onnx-asr/onnxruntime, torch, mediapipe) on win/linux/mac and
/// satisfies the onnx-asr >=3.10 floor. `uv` selects the latest available 3.12
/// patch when it has to provision a new managed interpreter.
const PYTHON_REQUEST: &str = "3.12";

/// Full-policy readiness requires real imports, not merely a solved dependency
/// graph. It intentionally exercises torch/audio/vision together because those
/// packages can resolve to individually valid but incompatible wheel releases.
const ONNX_ASR_IMPORT_SENTINEL: &str = "shellx-cut-onnx-asr-import-ok";
const FULL_PERCEPTION_IMPORT_SENTINEL: &str = "shellx-cut-full-perception-import-ok";
const FULL_PERCEPTION_IMPORT_PROBE: &str = "import onnx_asr, onnxruntime; import torch, torchaudio, torchvision; import whisperx, soundfile, silero_vad, transformers, sentencepiece, supervision, mediapipe, cv2; from scenedetect import open_video, SceneManager; from scenedetect.detectors import ContentDetector; from rapidocr_onnxruntime import RapidOCR; print('shellx-cut-full-perception-import-ok')";

/// Result of a successful provisioning — becomes the job result + drives the
/// doctor re-scan (the sidecar card flips missing → ready).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SetupOutcome {
    /// Absolute path to the venv python that now runs the sidecar.
    pub venv_python: String,
    /// Concrete uv-managed CPython executable used to create the venv. This is
    /// resolved after selection so Windows never stores uv's floating minor
    /// junction in `pyvenv.cfg`.
    pub managed_python: String,
    /// Actual `MAJOR.MINOR.PATCH` reported by the freshly-created venv Python.
    pub python_version: String,
    /// uv version used.
    pub uv_version: Option<String>,
    /// True when the Parakeet model was pre-fetched (first transcribe is instant).
    pub model_warmed: bool,
    /// Whether onnx-asr imports in the new venv (the STT engine is ready). This is
    /// the CRITICAL outcome: when true, transcription works on the user's box.
    pub onnx_asr_ready: bool,
    /// Whether the FULL perception extras (whisperX fallback, auto-reframe detector,
    /// face framing, beat grid, OCR) passed dependency checking and their required
    /// module-import probe. This is package/runtime admission only; native product
    /// actions still require their own host qualification. BEST-EFFORT: false here
    /// does NOT mean setup failed — transcription still works on the onnx-asr base.
    pub full_perception_ready: bool,
    /// Human note about the extras outcome (e.g. why they were skipped) — surfaced
    /// for the audit trail; empty when everything installed.
    pub extras_note: Option<String>,
    /// Package-manager readback from the completed environment in `name==version`
    /// form. This records the actually resolved versions without treating a
    /// generated lock as a product contract.
    pub package_versions: Vec<String>,
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
    // The FULL perception policy (whisperX fallback, auto-reframe detector, face
    // framing, beat grid, OCR) is bundled beside the base policy. It is resolved
    // at consent time by uv against ready-to-install wheels; an older bundle
    // without this source input remains a valid base-only installation.
    let requirements_full = script.parent().and_then(|dir| {
        let path = dir.join("requirements-full.txt");
        path.is_file().then_some(path)
    });
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
    let managed_python = select_managed_python(&uv, progress, 0.22, 0.40)?;

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
            managed_python_arg(&managed_python)?,
        ],
        "uv venv",
        progress,
        0.42,
        0.48,
    )?;

    let python_version = confirmed_python_version(&venv_python)?;

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
    let base_args = base_perception_install_args(vpy, &requirements)?;
    run_streaming(
        &uv,
        &base_args,
        "uv pip install transcription engine",
        progress,
        0.50,
        0.60,
    )?;
    verify_perception_dependencies(&uv, vpy, "base transcription engine", progress, 0.60, 0.605)?;

    // ---- 5. verify onnx-asr imports (the STT engine is actually usable) ------
    progress(0.61, "verifying STT engine");
    let initial_onnx_asr_ready = onnx_asr_import_ready(&venv_python);

    // ---- 6. (optional) pre-fetch the Parakeet model --------------------------
    // Done BEFORE the heavy extras so the critical "engine + model" pair lands
    // first; the user can transcribe even if the extras phase is slow or skipped.
    let mut model_warmed = false;
    if warm_model && initial_onnx_asr_ready {
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

    // ---- 7. install the perception extras (BEST-EFFORT, current wheels) -------
    //
    // This intentionally resolves the bundled policy at consent time rather than
    // shipping a platform-specific generated lock. Wheel-only installation and
    // uv's explicit CPU torch backend keep a fresh machine off source builds and
    // CUDA/NVIDIA payloads. An installer failure remains best-effort only when the
    // final shared-venv dependency and STT checks still pass.
    let full_install = requirements_full.as_ref().map(|requirements_full| {
        progress(
            0.78,
            "installing perception tools (captions, scenes, silence, auto-reframe, OCR)",
        );
        full_perception_install_args(vpy, requirements_full).and_then(|args| {
            run_streaming(
                &uv,
                &args,
                "uv pip install perception tools",
                progress,
                0.78,
                0.96,
            )
        })
    });

    // The extras share this venv. Even an optional-install failure can have
    // changed its resolved graph, so every successful outcome needs a final
    // dependency check and STT import before the actual final package receipt.
    progress(0.965, "checking final perception dependencies");
    verify_perception_dependencies(
        &uv,
        vpy,
        "final perception environment",
        progress,
        0.965,
        0.97,
    )?;
    progress(0.975, "verifying final transcription engine");
    verify_final_onnx_asr_import(&venv_python)?;
    let onnx_asr_ready = true;

    let (full_perception_ready, extras_note) = match full_install {
        Some(Ok(())) => match verify_full_perception_imports(&venv_python) {
            Ok(()) => (true, None),
            Err(e) => {
                let note = format!(
                    "the current compatible perception tools installed but did not pass their module-import check ({}); \
                     transcription still works on the built-in engine",
                    e.message
                );
                progress(0.98, &note);
                (false, Some(note))
            }
        },
        Some(Err(e)) => {
            let note = format!(
                "the current compatible perception tools could not be installed ({}); \
                 transcription still works on the built-in engine",
                e.message
            );
            progress(0.98, &note);
            (false, Some(note))
        }
        None => (
            false,
            Some(
                "perception extras file not shipped in this build; \
                 transcription engine installed"
                    .to_string(),
            ),
        ),
    };
    let package_versions = installed_package_versions(&uv, vpy)?;

    progress(1.0, "perception ready");
    Ok(SetupOutcome {
        venv_python: venv_python.display().to_string(),
        managed_python: managed_python.path.display().to_string(),
        python_version,
        uv_version,
        model_warmed,
        onnx_asr_ready,
        full_perception_ready,
        extras_note,
        package_versions,
    })
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
    let managed_python = select_managed_python(&uv, progress, 0.10, 0.16)?;

    // ---- 3. the isolated venv (clean rebuild) --------------------------------
    progress(0.17, "creating premium venv");
    let _ = std::fs::remove_dir_all(&venv_dir);
    run_streaming(
        &uv,
        &[
            "venv",
            venv_dir.to_str().unwrap_or_default(),
            "--python",
            managed_python_arg(&managed_python)?,
        ],
        "uv venv",
        progress,
        0.17,
        0.20,
    )?;

    let vpy = venv_python.to_str().unwrap_or_default();

    // ---- 4. platform-selected torch (the big one, ~3 GB on CUDA) ------------
    // macOS must use its platform build from uv's default index. The cu128
    // index is NVIDIA-only, so keeping it for macOS makes the ordinary installer
    // select an inapplicable package source. Windows and Linux retain the pinned
    // CUDA route; this is source selection, not a compatibility claim.
    let macos = cfg!(target_os = "macos");
    progress(
        0.22,
        if macos {
            "installing PyTorch for macOS — this can take a few minutes"
        } else {
            "installing PyTorch (CUDA, ~3 GB — this can take a few minutes)"
        },
    );
    let torch_args = premium_torch_install_args(vpy, macos);
    run_streaming(
        &uv,
        &torch_args,
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
    )?;
    let cuda_available = parse_matanyone_probe(&probe)?;

    progress(1.0, "premium background removal ready");
    Ok(MatanyoneSetupOutcome {
        venv_python: venv_python.display().to_string(),
        checkpoint: checkpoint.display().to_string(),
        uv_version,
        // An unsuccessful or malformed import probe now returns Err above, so a
        // successful setup result is an actual import-ready result.
        matanyone_ready: true,
        cuda_available,
    })
}

/// Select the reviewed PyTorch source by product target. The platform boolean
/// is injected so the policy stays unit-testable on every development host.
fn premium_torch_install_args(venv_python: &str, macos: bool) -> Vec<&str> {
    let mut args = vec![
        "pip",
        "install",
        "--python",
        venv_python,
        "torch==2.8.0",
        "torchvision==0.23.0",
    ];
    if !macos {
        args.extend(["--index-url", TORCH_CU128_INDEX]);
    }
    args
}

/// Interpret the exact stdout emitted by the final premium-runtime probe.
/// A setup job must not be marked successful when its required imports fail or
/// when the probe does not identify its device state.
fn parse_matanyone_probe(probe: &str) -> Result<bool, CutError> {
    match probe.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["ok", "True"] => Ok(true),
        ["ok", "False"] => Ok(false),
        _ => Err(
            CutError::new(
                error_codes::SIDECAR,
                "premium background-removal runtime failed verification",
                format!("expected `ok True` or `ok False` from the runtime probe; got {probe:?}"),
            )
            .with_suggested_action(
                "retry the premium installation; if it persists, use the standard Background Removal tool",
            ),
        ),
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedPython {
    path: PathBuf,
    version: String,
}

/// Select a concrete uv-managed CPython 3.12 executable, installing the latest
/// compatible patch only when no usable managed interpreter exists.
///
/// The pre-install lookup retains the Windows recovery path from older uv
/// releases: a floating `cpython-3.12-windows-*` alias could be a plain
/// directory when the account could not create a junction. If its versioned
/// interpreter is already usable, we resolve it and never ask uv to replace the
/// broken alias. `--resolve-links` is mandatory: the subsequent venv receives
/// the real executable, never the floating alias.
fn select_managed_python(
    uv: &Path,
    progress: &ProgressFn,
    band_lo: f32,
    band_hi: f32,
) -> Result<ManagedPython, CutError> {
    if let Some(installed) = find_managed_python(uv)? {
        progress(
            band_hi,
            &format!("Python {} runtime already installed", installed.version),
        );
        return Ok(installed);
    }

    run_streaming(
        uv,
        &["python", "install", PYTHON_REQUEST],
        "uv python install",
        progress,
        band_lo,
        band_hi,
    )?;
    find_managed_python(uv)?.ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "uv installed Python but did not expose a usable CPython 3.12 executable",
            "the post-install managed Python lookup returned no resolved executable",
        )
        .with_suggested_action(
            "Choose Install captions again; if it persists, reinstall the captions runtime",
        )
    })
}

/// Return a concrete managed CPython selected by uv, or None when uv has no
/// installed compatible runtime. A malformed successful lookup is an error: it
/// must never fall through to venv creation with an alias or an arbitrary path.
fn find_managed_python(uv: &Path) -> Result<Option<ManagedPython>, CutError> {
    let mut probe = Command::new(uv);
    probe.args([
        "python",
        "find",
        "--managed-python",
        "--no-python-downloads",
        "--no-project",
        "--resolve-links",
        PYTHON_REQUEST,
    ]);
    let output =
        crate::dispatch::run_bounded_foreground_command(&mut probe, "managed Python lookup")
            .map_err(|e| {
                CutError::new(
                    error_codes::JOB_FAILED,
                    "could not locate the managed Python runtime",
                    format!("{}: {e}", uv.display()),
                )
                .with_suggested_action(
                    "Choose Install captions again; if it persists, reinstall the captions runtime",
                )
            })?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = managed_python_path(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "managed Python lookup returned no executable path",
            "uv reported success without a concrete Python executable",
        )
        .with_suggested_action(
            "Choose Install captions again; if it persists, reinstall the captions runtime",
        )
    })?;
    if !path.is_file() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "managed Python lookup returned an unusable executable",
            format!("{} is not a file", path.display()),
        )
        .with_suggested_action(
            "Choose Install captions again; if it persists, reinstall the captions runtime",
        ));
    }
    let output = run_capture(&path, &["--version"])?;
    managed_python_identity(path, &output)
        .map(Some)
        .map_err(|cause| {
            CutError::new(
                error_codes::SIDECAR,
                "managed Python is not a compatible CPython 3.12 runtime",
                cause,
            )
            .with_suggested_action(
                "Choose Install captions again; if it persists, reinstall the captions runtime",
            )
        })
}

fn managed_python_path(stdout: &str) -> Option<PathBuf> {
    let path = PathBuf::from(stdout.trim());
    (!path.as_os_str().is_empty()).then_some(path)
}

fn managed_python_identity(path: PathBuf, version_output: &str) -> Result<ManagedPython, String> {
    if !path.is_absolute() {
        return Err(format!(
            "resolved managed Python path is not absolute: {}",
            path.display()
        ));
    }
    let version = parse_compatible_python_version(version_output)
        .ok_or_else(|| format!("expected CPython {PYTHON_REQUEST}.*; got {version_output:?}"))?;
    Ok(ManagedPython { path, version })
}

fn managed_python_arg(managed: &ManagedPython) -> Result<&str, CutError> {
    managed.path.to_str().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "managed Python path is not valid text",
            managed.path.display().to_string(),
        )
        .with_suggested_action(
            "Choose Install captions again; if it persists, reinstall the captions runtime",
        )
    })
}

fn confirmed_python_version(python: &Path) -> Result<String, CutError> {
    let output = run_capture(python, &["--version"])?;
    parse_compatible_python_version(&output).ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "the new perception environment did not use CPython 3.12",
            format!("{} reported {output:?}", python.display()),
        )
        .with_suggested_action(
            "Choose Install captions again; if it persists, reinstall the captions runtime",
        )
    })
}

fn parse_compatible_python_version(output: &str) -> Option<String> {
    let reported = output.trim().strip_prefix("Python ")?;
    let version = reported.split_whitespace().next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u16>().ok()?;
    let minor = parts.next()?.parse::<u16>().ok()?;
    let patch = parts.next()?.parse::<u16>().ok()?;
    if parts.next().is_some() || (major, minor) != (3, 12) {
        return None;
    }
    Some(format!("{major}.{minor}.{patch}"))
}

/// Arguments shared by the required base setup. Wheel-only is a product policy:
/// the installer never compiles an arbitrary source package on a user device.
fn base_perception_install_args<'a>(
    venv_python: &'a str,
    requirements: &'a Path,
) -> Result<Vec<&'a str>, CutError> {
    Ok(vec![
        "pip",
        "install",
        "--python",
        venv_python,
        "--only-binary",
        ":all:",
        "--strict",
        "-r",
        requirements_arg(requirements)?,
    ])
}

/// Arguments for the optional tools. `--torch-backend cpu` is uv's explicit
/// PyTorch-family source selection: it leaves ordinary dependencies on the
/// normal index while choosing CPU wheels for torch/vision/audio.
fn full_perception_install_args<'a>(
    venv_python: &'a str,
    requirements: &'a Path,
) -> Result<Vec<&'a str>, CutError> {
    Ok(vec![
        "pip",
        "install",
        "--python",
        venv_python,
        "--only-binary",
        ":all:",
        "--torch-backend",
        "cpu",
        "--strict",
        "-r",
        requirements_arg(requirements)?,
    ])
}

fn requirements_arg(requirements: &Path) -> Result<&str, CutError> {
    requirements.to_str().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "perception requirements path is not valid text",
            requirements.display().to_string(),
        )
    })
}

/// Dependency checks are explicit evidence. Base failures stop setup; optional
/// failures produce a best-effort note and never become a false readiness claim.
fn verify_perception_dependencies(
    uv: &Path,
    venv_python: &str,
    label: &str,
    progress: &ProgressFn,
    band_lo: f32,
    band_hi: f32,
) -> Result<(), CutError> {
    run_streaming(
        uv,
        &["pip", "check", "--python", venv_python],
        &format!("uv pip check {label}"),
        progress,
        band_lo,
        band_hi,
    )
}

fn onnx_asr_import_ready(python: &Path) -> bool {
    run_capture(
        python,
        &[
            "-c",
            "import onnx_asr, onnxruntime; print('shellx-cut-onnx-asr-import-ok')",
        ],
    )
    .map(|output| has_final_stdout_sentinel(&output, ONNX_ASR_IMPORT_SENTINEL))
    .unwrap_or(false)
}

fn has_final_stdout_sentinel(output: &str, sentinel: &str) -> bool {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        == Some(sentinel)
}

/// The required base import is repeated after the best-effort full install because
/// both policies share a venv. A changed graph must never produce a success result
/// that relies on a stale pre-extras probe.
fn verify_final_onnx_asr_import(python: &Path) -> Result<(), CutError> {
    if onnx_asr_import_ready(python) {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::SIDECAR,
        "the final perception environment can no longer import the transcription engine",
        python.display().to_string(),
    )
    .with_suggested_action(
        "Choose Install captions again; the optional tools were not marked ready",
    ))
}

/// Full extras are admitted only when their actual modules load together. This
/// deliberately precedes native product qualification; a successful import does
/// not claim that a model, media fixture, or native UI flow has run on this host.
fn verify_full_perception_imports(python: &Path) -> Result<(), CutError> {
    let output = run_capture(python, &["-c", FULL_PERCEPTION_IMPORT_PROBE])?;
    if has_final_stdout_sentinel(&output, FULL_PERCEPTION_IMPORT_SENTINEL) {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::SIDECAR,
        "the installed perception tools did not pass their module-import check",
        format!("{} returned {output:?}", python.display()),
    )
    .with_suggested_action(
        "Transcription remains available. Choose Install captions again to retry the optional tools",
    ))
}

/// Return the package manager's actual resolved versions after dependency
/// validation. This is receipt data, not a generated lock or future install input.
fn installed_package_versions(uv: &Path, venv_python: &str) -> Result<Vec<String>, CutError> {
    let output = run_capture(uv, &["pip", "freeze", "--python", venv_python, "--strict"])?;
    Ok(parse_package_versions(&output))
}

fn parse_package_versions(freeze: &str) -> Vec<String> {
    freeze
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.split_once("==")
                .is_some_and(|(name, version)| !name.is_empty() && !version.is_empty())
        })
        .map(ToOwned::to_owned)
        .collect()
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
    let mut command = Command::new(prog);
    command.args(args);
    let output =
        crate::dispatch::run_bounded_foreground_command(&mut command, label).map_err(|e| {
            CutError::new(
                error_codes::JOB_FAILED,
                format!("could not run {label}"),
                format!("{}: {e}", prog.display()),
            )
            .with_suggested_action("retry; if it persists, check disk space and network")
        })?;

    // The common owner has already drained capped stderr and reaped the whole
    // tree. Replay its retained status lines through the established progress
    // surface so setup errors remain actionable without an unbounded child.
    let mut tail: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            progress(band_lo, &format!("{label}: {trimmed}"));
            tail.push(trimmed.to_string());
            if tail.len() > 60 {
                tail.remove(0);
            }
        }
    }
    if !output.status.success() {
        let tail_txt = tail
            .iter()
            .rev()
            .take(12)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let (message, action) = classify_install_failure(&tail_txt, output.status.code());
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
        "Setup did not complete. Choose \"Install captions\" again; if it keeps failing, \
         check your internet connection and that you have a few gigabytes of free disk space.",
    )
}

/// Run a command and return stdout only when the probe command succeeded. Import
/// readiness and package receipts must never accept output from a nonzero process.
fn run_capture(prog: &Path, args: &[&str]) -> Result<String, CutError> {
    let mut command = Command::new(prog);
    command.args(args);
    let out =
        crate::dispatch::run_bounded_foreground_command(&mut command, "perception setup probe")
            .map_err(|e| {
                CutError::new(
                    error_codes::JOB_FAILED,
                    "command failed to run",
                    e.to_string(),
                )
            })?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::JOB_FAILED,
            "perception setup probe failed",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
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
    fn perception_install_policy_uses_ready_wheels_and_cpu_torch() {
        let base = base_perception_install_args("/venv/python", Path::new("requirements.txt"))
            .expect("base args");
        assert_eq!(
            base,
            vec![
                "pip",
                "install",
                "--python",
                "/venv/python",
                "--only-binary",
                ":all:",
                "--strict",
                "-r",
                "requirements.txt",
            ]
        );
        let full = full_perception_install_args("/venv/python", Path::new("requirements-full.txt"))
            .expect("full args");
        assert!(full
            .windows(2)
            .any(|pair| pair == ["--only-binary", ":all:"]));
        assert!(full
            .windows(2)
            .any(|pair| pair == ["--torch-backend", "cpu"]));
        assert!(full.contains(&"--strict"));
        assert!(!full.contains(&"--index-strategy"));
    }

    #[test]
    fn full_perception_readiness_requires_real_cross_package_imports() {
        assert!(FULL_PERCEPTION_IMPORT_PROBE.contains("import torch, torchaudio, torchvision"));
        assert!(FULL_PERCEPTION_IMPORT_PROBE.contains("import whisperx"));
        assert!(FULL_PERCEPTION_IMPORT_PROBE.contains("from rapidocr_onnxruntime import RapidOCR"));
        assert!(FULL_PERCEPTION_IMPORT_PROBE.contains("cv2"));
        assert!(FULL_PERCEPTION_IMPORT_PROBE
            .contains("from scenedetect.detectors import ContentDetector"));
        assert!(FULL_PERCEPTION_IMPORT_PROBE.contains(FULL_PERCEPTION_IMPORT_SENTINEL));
        assert!(has_final_stdout_sentinel(
            "normal import output\nshellx-cut-full-perception-import-ok\n",
            FULL_PERCEPTION_IMPORT_SENTINEL,
        ));
        assert!(!has_final_stdout_sentinel(
            "shellx-cut-full-perception-import-ok\nmore output\n",
            FULL_PERCEPTION_IMPORT_SENTINEL,
        ));
    }

    #[test]
    fn resolved_package_receipt_keeps_only_name_version_rows() {
        assert_eq!(
            parse_package_versions(
                "# comment\nonnx-asr==0.11.0\ntorch @ https://example.invalid/wheel\nnumpy==2.4.6\n",
            ),
            vec!["onnx-asr==0.11.0", "numpy==2.4.6"]
        );
    }

    #[test]
    fn managed_python_request_keeps_the_reviewed_cp312_compatibility_line() {
        assert_eq!(PYTHON_REQUEST, "3.12");
    }

    #[test]
    fn managed_python_identity_requires_a_concrete_cp312_executable() {
        assert_eq!(
            managed_python_identity(
                PathBuf::from("/managed/cpython-3.12.14/bin/python"),
                "Python 3.12.14\n",
            ),
            Ok(ManagedPython {
                path: PathBuf::from("/managed/cpython-3.12.14/bin/python"),
                version: "3.12.14".to_string(),
            })
        );
        for invalid in [
            "Python 3.13.0\n",
            "Python 3.11.9\n",
            "Python 3.12\n",
            "not Python\n",
        ] {
            assert!(
                managed_python_identity(PathBuf::from("/managed/python"), invalid).is_err(),
                "wrong or absent Python patch must be rejected: {invalid:?}"
            );
        }
        assert!(managed_python_identity(PathBuf::from("python"), "Python 3.12.14\n").is_err());
        assert_eq!(managed_python_path(" \r\n\t"), None);
    }

    #[test]
    fn premium_torch_source_is_platform_specific() {
        let macos = premium_torch_install_args("/venv/bin/python", true);
        assert!(
            !macos.contains(&TORCH_CU128_INDEX),
            "macOS must not select NVIDIA's cu128 wheel index"
        );

        for target in ["windows", "linux"] {
            let args = premium_torch_install_args("C:/venv/python", false);
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--index-url", TORCH_CU128_INDEX]),
                "{target} must retain the reviewed CUDA wheel source"
            );
        }
    }

    #[test]
    fn premium_probe_requires_import_ready_output() {
        assert_eq!(parse_matanyone_probe("ok True\n").unwrap(), true);
        assert_eq!(parse_matanyone_probe("ok False\n").unwrap(), false);
        for invalid in ["", "ok", "not ok", "ok maybe"] {
            assert!(
                parse_matanyone_probe(invalid).is_err(),
                "malformed or failed probe must fail setup: {invalid:?}"
            );
        }
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
