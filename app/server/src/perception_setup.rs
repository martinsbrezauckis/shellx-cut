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
use crate::jobs::JobCancellation;
use cut_core::{error_codes, CutError};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;

mod pe_imports;
mod windows_runtime;

#[cfg(all(test, unix))]
mod cancel_tests;

thread_local! {
    /// The cancellation of the job whose blocking worker is running a setup on
    /// this thread. Every helper below (uv, pip, the model download) runs on that
    /// one thread, so a thread-local reaches all of them without threading a
    /// parameter through each stage.
    static SETUP_CANCELLATION: RefCell<Option<JobCancellation>> = const { RefCell::new(None) };
}

/// Makes `cancellation` the one every helper command of this thread obeys until
/// dropped.
struct CancellationScope;

impl CancellationScope {
    fn enter(cancellation: &JobCancellation) -> Self {
        SETUP_CANCELLATION.with(|slot| *slot.borrow_mut() = Some(cancellation.clone()));
        Self
    }
}

impl Drop for CancellationScope {
    fn drop(&mut self) {
        SETUP_CANCELLATION.with(|slot| *slot.borrow_mut() = None);
    }
}

fn setup_cancelled() -> bool {
    SETUP_CANCELLATION.with(|slot| slot.borrow().as_ref().is_some_and(|c| c.is_cancelled()))
}

/// The refusal a cancelled setup ends with. Code `job_cancelled`, so the job
/// record reads cancelled and not failed.
fn cancelled_error() -> CutError {
    CutError::new(
        "job_cancelled",
        "Install captions was cancelled",
        "the install was stopped; nothing was left half-installed",
    )
    .with_suggested_action("Choose Install captions again whenever you want to retry")
}

fn check_cancelled() -> Result<(), CutError> {
    if setup_cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

/// Run one helper under the shared owned-process contract with this setup's
/// cancellation: a cancel kills the helper's whole process tree (uv, pip, the
/// model download and their children) and returns at once.
fn run_setup_command(command: &mut Command, what: &str) -> Result<std::process::Output, CutError> {
    let probe = SETUP_CANCELLATION.with(|slot| slot.borrow().clone());
    let control = cut_media::ffmpeg::OwnedProcessControl::bounded(
        std::time::Duration::from_secs(30 * 60),
        move || probe.as_ref().is_some_and(|c| c.is_cancelled()),
    );
    cut_media::ffmpeg::run_owned_command(command, &control, what)
}

/// Removes the venv this setup is building unless the setup finished. The
/// resolver trusts an existing `.venv` interpreter, so a cancelled or failed
/// install must not leave one behind that later reads as installed.
struct PartialVenv<'a> {
    dir: &'a Path,
    armed: bool,
}

impl Drop for PartialVenv<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(self.dir);
        }
    }
}

/// Env override: use an EXISTING uv binary instead of downloading one. Test/dev
/// seam (the build box already has uv) — never a verb arg.
pub const ENV_UV: &str = "SHELLX_CUT_UV";

/// Python compatibility request for the sidecar venv. CPython 3.12 has wheels
/// for every dep (onnx-asr/onnxruntime, torch, mediapipe) on win/linux/mac and
/// satisfies the onnx-asr >=3.10 floor. `uv` selects the latest available 3.12
/// patch when it has to provision a new managed interpreter.
/// What the job says while it downloads the uv installer: a cancel cannot
/// interrupt the download and takes effect right after it.
const UV_DOWNLOAD_STEP: &str =
    "downloading the installer, about 20 MB; a cancel stops right after it";

const PYTHON_REQUEST: &str = "3.12";

/// Full-policy readiness requires real imports, not merely a solved dependency
/// graph. It intentionally exercises torch/audio/vision together because those
/// packages can resolve to individually valid but incompatible wheel releases.
const ONNX_ASR_IMPORT_SENTINEL: &str = "shellx-cut-onnx-asr-import-ok";
const BASE_STT_IMPORT_PROBE: &str =
    "import onnx_asr, onnxruntime, numpy, soundfile; print('shellx-cut-onnx-asr-import-ok')";
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
    /// True when the model loaded successfully during setup.
    pub model_warmed: bool,
    /// Optional model preparation failure, preserved separately from successful
    /// package installation. None when warming was skipped or succeeded.
    pub model_warm_error: Option<CutError>,
    /// Whether the direct base STT modules import in the new venv. This does not
    /// prove model loading or transcription; those can still fail independently.
    pub onnx_asr_ready: bool,
    /// Whether the FULL perception extras (whisperX fallback, auto-reframe detector,
    /// face framing, beat grid, OCR) passed dependency checking and their required
    /// module-import probe. This is package/runtime admission only; native product
    /// actions still require their own host qualification. BEST-EFFORT: false here
    /// does NOT mean the base dependency installation failed.
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
/// `warm_model` attempts to download and load the ONNX speech model.
/// Model preparation failures are retained separately from dependency setup. Every error is
/// actionable; nothing is left half-installed that the resolver would trust
/// (uv writes the venv atomically enough that a failed pip leaves an obviously
/// incomplete venv the doctor still reports as "deps missing").
///
/// `cancellation` is the job's: a cancel stops the running helper's process tree
/// at once, ends the setup as `job_cancelled` and removes the unfinished venv.
pub fn setup_perception(
    warm_model: bool,
    progress: &ProgressFn,
    cancellation: &JobCancellation,
) -> Result<SetupOutcome, CutError> {
    let _scope = CancellationScope::enter(cancellation);
    // Windows: the packages need Microsoft's Visual C++ runtime; refuse before
    // downloading anything when it is missing (vc_runtime.rs).
    crate::vc_runtime::require_for("Captions and transcription")?;
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
    check_cancelled()?;
    let uv_version = run_capture(&uv, &["--version"])
        .ok()
        .map(|s| s.trim().to_string());
    check_cancelled()?;

    // ---- 2. standalone CPython (uv-managed; no system python dependency) -----
    progress(0.22, "installing Python runtime");
    let managed_python = select_managed_python(&uv, progress, 0.22, 0.40)?;

    // ---- 3. create the venv on that interpreter ------------------------------
    progress(0.42, "creating venv");
    // A clean rebuild: remove any prior partial venv so pip starts fresh.
    let _ = std::fs::remove_dir_all(&venv_dir);
    let mut partial_venv = PartialVenv {
        dir: &venv_dir,
        armed: true,
    };
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
    progress(0.61, "verifying base transcription engine");
    windows_runtime::check_environment(&venv_dir, &managed_python.path, "base")?;
    verify_base_onnx_asr_import(&venv_python)?;

    // ---- 5. install the perception extras (BEST-EFFORT, current wheels) -------
    //
    // This intentionally resolves the bundled policy at consent time rather than
    // shipping a platform-specific generated lock. Wheel-only installation and
    // uv's explicit CPU torch backend keep a fresh machine off source builds and
    // CUDA/NVIDIA payloads. An installer failure remains best-effort only when the
    // final shared-venv dependency and STT checks still pass.
    let full_install = requirements_full.as_ref().map(|requirements_full| {
        progress(
            0.62,
            "installing perception tools (captions, scenes, silence, auto-reframe, OCR)",
        );
        full_perception_install_args(vpy, requirements_full).and_then(|args| {
            run_streaming(
                &uv,
                &args,
                "uv pip install perception tools",
                progress,
                0.62,
                0.86,
            )
        })
    });

    // The extras share this venv. Even an optional-install failure can have
    // changed its resolved graph, so every successful outcome needs a final
    // dependency check and STT import before the actual final package receipt.
    // A cancel during the optional install must end the whole setup, not be
    // swallowed as a best-effort extras failure.
    check_cancelled()?;
    progress(0.865, "checking final perception dependencies");
    verify_perception_dependencies(
        &uv,
        vpy,
        "final perception environment",
        progress,
        0.865,
        0.88,
    )?;
    progress(0.885, "verifying final transcription engine");
    let final_runtime =
        windows_runtime::check_environment(&venv_dir, &managed_python.path, "perception tools");
    verify_final_onnx_asr_import(&venv_python)?;
    let onnx_asr_ready = true;

    let (full_perception_ready, extras_note) = match full_install {
        Some(Ok(())) => {
            match final_runtime.and_then(|()| verify_full_perception_imports(&venv_python)) {
                Ok(()) => (true, None),
                Err(e) => {
                    let note = format!(
                    "the current compatible perception tools installed but did not pass their module-import check ({}); \
                     the base transcription dependencies are installed",
                    e.message
                );
                    progress(0.90, &note);
                    (false, Some(note))
                }
            }
        }
        Some(Err(e)) => {
            let note = format!(
                "the current compatible perception tools could not be installed ({}); \
                 the base transcription dependencies are installed",
                e.message
            );
            progress(0.90, &note);
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

    // ---- 6. optional model load against the final installed environment -----
    let mut model_warmed = false;
    let mut model_warm_error = None;
    let selection = cut_perception::effective_stt_selection();
    if let Some(model_id) = onnx_warm_model_id(selection.model.as_deref()).filter(|_| warm_model) {
        progress(0.91, "preparing speech model");
        // Setup and real transcription share the same ordinary local model
        // layout; Runner's admitted immutable models remain a separate route.
        let model_script = script.with_file_name("ordinary_speech_models.py");
        let model_root = sidecar_dir.join("models");
        let warm = run_streaming(
            &venv_python,
            &[
                model_script.to_str().unwrap_or_default(),
                "--prepare",
                "--model",
                model_id,
                "--root",
                model_root.to_str().unwrap_or_default(),
            ],
            "model download",
            progress,
            0.91,
            0.99,
        );
        // A cancelled model download is a cancelled install, not a warning.
        check_cancelled()?;
        match warm {
            Ok(()) => model_warmed = true,
            Err(error) => model_warm_error = Some(error),
        }
    }
    check_cancelled()?;
    partial_venv.armed = false;

    progress(
        1.0,
        if model_warm_error.is_some() {
            "caption tools installed; speech model preparation failed"
        } else {
            "caption tools installed"
        },
    );
    Ok(SetupOutcome {
        venv_python: venv_python.display().to_string(),
        managed_python: managed_python.path.display().to_string(),
        python_version,
        uv_version,
        model_warmed,
        model_warm_error,
        onnx_asr_ready,
        full_perception_ready,
        extras_note,
        package_versions,
    })
}

// Match the ordinary transcription selector's explicit model families. Whisper
// uses its own runtime and prepares on first transcription, not through ONNX.
fn onnx_warm_model_id(selected: Option<&str>) -> Option<&str> {
    let selected = selected.map(str::trim).filter(|value| !value.is_empty());
    let model = selected.unwrap_or("nemo-parakeet-tdt-0.6b-v3");
    let lowered = model.to_ascii_lowercase();
    if lowered.starts_with("whisper-") || lowered.starts_with("whisperx-") {
        None
    } else if lowered == "canary"
        || lowered.starts_with("canary-")
        || lowered.starts_with("nemo-canary")
        || lowered.starts_with("canary@")
    {
        let canary = model.split('@').next().unwrap_or(model).trim();
        if matches!(
            canary.to_ascii_lowercase().as_str(),
            "canary" | "canary-1b-v2"
        ) {
            Some("nemo-canary-1b-v2")
        } else {
            Some(canary)
        }
    } else {
        Some(model)
    }
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
const MATANYONE_SRC_SHA256: &str =
    "2dc341b915d8cc887e3b889fa9d03f950f6de7c13cee800bee9748d5aa3c7e18";
const MATANYONE_SRC_ROOT: &str = "MatAnyone2-e3370127319c63a6dc8a49c69de2d41d90137f91";
const MATANYONE_SRC_MAX_BYTES: &str = "33554432";

/// Check the pinned source before extraction with the venv's Python stdlib.
const FETCH_SRC_PY: &str = r#"
import hashlib, os, shutil, sys, tarfile, tempfile, urllib.request
url, dest, expected, archive_limit, source_limit, root = sys.argv[1:]
archive_limit, source_limit = int(archive_limit), int(source_limit)

def reject(reason):
    raise SystemExit('MatAnyone2 source rejected: ' + reason)

parent = os.path.dirname(os.path.abspath(dest))
os.makedirs(parent, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='.matanyone-source-', dir=parent) as td:
    tgz = os.path.join(td, 'source.tar.gz')
    size = 0
    with urllib.request.urlopen(url, timeout=60) as response, open(tgz, 'xb') as output:
        while chunk := response.read(65536):
            size += len(chunk)
            if size > archive_limit:
                reject('compressed download exceeds the size limit')
            output.write(chunk)

    records, seen, members = [], set(), []
    extracted = 0
    with tarfile.open(tgz, 'r:gz') as archive:
        for member in archive:
            if len(members) >= 1000:
                reject('archive has too many entries')
            members.append(member)
            name = member.name.rstrip('/') if member.isdir() else member.name
            if name == root and member.isdir():
                relative = ''
            else:
                parts = name.split('/')
                if (len(parts) < 2 or parts[0] != root or
                        any(part in ('', '.', '..') for part in parts[1:]) or
                        '\\' in name or ':' in name):
                    reject('unexpected archive layout: ' + name)
                relative = '/'.join(parts[1:])
            if relative.casefold() in seen:
                reject('duplicate archive path: ' + (relative or root))
            seen.add(relative.casefold())
            if member.isdir():
                continue
            if not member.isfile():
                reject('unexpected archive member type: ' + relative)
            if member.size > source_limit - extracted:
                reject('extracted source exceeds the size limit')
            file_hash = hashlib.sha256()
            actual = 0
            with archive.extractfile(member) as source:
                while chunk := source.read(65536):
                    actual += len(chunk)
                    if actual > member.size:
                        reject('archive member exceeds its declared size')
                    file_hash.update(chunk)
            if actual != member.size:
                reject('archive member is incomplete')
            extracted += actual
            records.append((relative, file_hash.digest()))
        tree = hashlib.sha256()
        for relative, digest in sorted(records):
            tree.update(relative.encode('utf-8') + b'\0' + digest)
        if tree.hexdigest() != expected:
            reject('source SHA-256 mismatch: expected ' + expected + ', got ' + tree.hexdigest())
        archive.extractall(td, members=members, filter='data')
    if os.path.lexists(dest):
        shutil.rmtree(dest)
    shutil.move(os.path.join(td, root), dest)
print('extracted', dest)
"#;

#[cfg(all(test, target_os = "linux"))]
#[path = "matte_source_archive_tests.rs"]
mod matte_source_archive_tests;

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
///
/// `cancellation` is the job's, with the same rules as [`setup_perception`]: a cancel
/// stops the running helper's process tree at once, ends the setup as
/// `job_cancelled` and removes the unfinished premium venv.
pub fn setup_matanyone(
    progress: &ProgressFn,
    cancellation: &JobCancellation,
) -> Result<MatanyoneSetupOutcome, CutError> {
    let _scope = CancellationScope::enter(cancellation);
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
    check_cancelled()?;
    let uv_version = run_capture(&uv, &["--version"])
        .ok()
        .map(|s| s.trim().to_string());
    check_cancelled()?;

    // ---- 2. standalone CPython 3.12 (MatAnyone2 runtime) ---------------------
    progress(0.10, "installing Python runtime");
    let managed_python = select_managed_python(&uv, progress, 0.10, 0.16)?;

    // ---- 3. the isolated venv (clean rebuild) --------------------------------
    progress(0.17, "creating premium venv");
    let _ = std::fs::remove_dir_all(&venv_dir);
    let mut partial_venv = PartialVenv {
        dir: &venv_dir,
        armed: true,
    };
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
    progress(0.21, "verifying MatAnyone2 source");
    run_streaming(
        &venv_python,
        &[
            "-c",
            FETCH_SRC_PY,
            MATANYONE_SRC_URL,
            src_str,
            MATANYONE_SRC_SHA256,
            MATANYONE_SRC_MAX_BYTES,
            MATANYONE_SRC_MAX_BYTES,
            MATANYONE_SRC_ROOT,
        ],
        "verify matanyone2 source",
        progress,
        0.21,
        0.22,
    )
    .map_err(|e| {
        CutError::new(
            error_codes::JOB_FAILED,
            "Premium background removal could not verify its source; nothing was installed",
            format!("{}: {}", e.message, e.cause),
        )
        .with_suggested_action("Try installing premium background removal again")
    })?;

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
    check_cancelled()?;
    progress(0.80, "downloading the MatAnyone2 checkpoint (135 MB)");
    let checkpoint = crate::matte::install_matanyone_model(&|f, m| {
        // The download cannot be interrupted: say so while a cancel waits for it.
        if m.starts_with("downloading") {
            progress(
                0.80 + f * 0.08,
                &format!("{m}; a cancel stops right after it"),
            )
        } else {
            progress(0.80 + f * 0.08, m)
        }
    })?;
    check_cancelled()?;

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
    check_cancelled()?;
    partial_venv.armed = false;

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
    // An override counts only when absolute: a relative value would be read
    // against the current folder, which may be a received project.
    if let Some(p) = std::env::var_os(ENV_UV) {
        let p = PathBuf::from(p);
        if p.is_absolute() {
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
    // The download cannot be interrupted, so the step says so: a job_cancel_pending
    // answer quotes this text while the cancel waits for it.
    progress(0.02, UV_DOWNLOAD_STEP);
    fetch::install_tool("uv", &|f, _| progress(0.02 + f * 0.18, UV_DOWNLOAD_STEP))?;
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
    windows_runtime::apply_uv_environment(&mut probe, uv);
    probe.args([
        "python",
        "find",
        "--managed-python",
        "--no-python-downloads",
        "--no-project",
        "--resolve-links",
        PYTHON_REQUEST,
    ]);
    let output = run_setup_command(&mut probe, "managed Python lookup").map_err(|e| {
        if setup_cancelled() {
            return cancelled_error();
        }
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

/// Ok when the base modules import; Err carries the interpreter's last error
/// line (for example "ImportError: DLL load failed ...") for the refusal.
fn onnx_asr_import_ready(python: &Path) -> Result<(), String> {
    match run_capture(python, &["-c", BASE_STT_IMPORT_PROBE]) {
        Ok(output) if has_final_stdout_sentinel(&output, ONNX_ASR_IMPORT_SENTINEL) => Ok(()),
        Ok(output) => Err(last_line(&output)),
        Err(error) => Err(last_line(&error.cause)),
    }
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("the import check printed nothing")
        .to_string()
}

fn has_final_stdout_sentinel(output: &str, sentinel: &str) -> bool {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        == Some(sentinel)
}

/// The required base import follows the base dependency check, before optional
/// tools can change the shared venv graph.
fn verify_base_onnx_asr_import(python: &Path) -> Result<(), CutError> {
    verify_onnx_asr_import(python, "base", "the base transcription environment")
}

/// The required base import is repeated after the best-effort full install because
/// both policies share a venv. A changed graph must never produce a success result
/// that relies on a stale pre-extras probe.
fn verify_final_onnx_asr_import(python: &Path) -> Result<(), CutError> {
    verify_onnx_asr_import(python, "final", "the perception environment")
}

fn verify_onnx_asr_import(python: &Path, stage: &str, repair_target: &str) -> Result<(), CutError> {
    onnx_asr_import_ready(python)
        .map_err(|reason| transcription_import_failure(python, stage, repair_target, &reason))
}

fn transcription_import_failure(
    python: &Path,
    stage: &str,
    repair_target: &str,
    reason: &str,
) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        format!(
            "the {stage} perception environment cannot import the transcription engine: {reason}"
        ),
        format!("{}: {reason}", python.display()),
    )
    .with_suggested_action(format!(
        "Choose Install captions again to rebuild {repair_target}"
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
    windows_runtime::apply_uv_environment(&mut command, prog);
    command.args(args);
    let output = run_setup_command(&mut command, label).map_err(|e| {
        if setup_cancelled() {
            return cancelled_error();
        }
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
    // 4. Cut's own folder could not be written: another program holds a file
    //    (antivirus, a second setup), access is denied, or something already
    //    stands where a folder must be created. Name that, not the network.
    if lower.contains("failed to create directory")
        || lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("being used by another process")
        || lower.contains("os error 5)")
        || lower.contains("os error 32)")
        || lower.contains("os error 183)")
    {
        return (
            "Captions could not write their files into ShellX Cut's folder.".to_string(),
            "Make sure no other program (for example an antivirus scan) is using ShellX Cut's \
             folder, then choose \"Install captions\" again. If it keeps failing, reinstall \
             ShellX Cut.",
        );
    }
    // 5. Out of disk space.
    if lower.contains("no space left") || lower.contains("not enough space") {
        return (
            "There isn't enough free disk space to finish setting up perception.".to_string(),
            "Free up a few gigabytes of disk space and choose \"Install captions\" \
             again.",
        );
    }
    // 6. Anything else — still avoid raw jargon; nothing partial is trusted.
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
    windows_runtime::apply_uv_environment(&mut command, prog);
    command.args(args);
    let out = run_setup_command(&mut command, "perception setup probe").map_err(|e| {
        if setup_cancelled() {
            return cancelled_error();
        }
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
    #[cfg(target_os = "linux")]
    #[test]
    fn source_archive_pin_and_caps() {
        use sha2::{Digest, Sha256};
        let dir = tempfile::tempdir().unwrap();
        let mut outcomes = Vec::new();
        for (body, valid_pin, compressed_cap, source_cap, archive_root) in [
            ("changed", false, "1048576", "1048576", MATANYONE_SRC_ROOT),
            ("pinned", true, "1", "1048576", MATANYONE_SRC_ROOT),
            ("pinned", true, "1048576", "1", MATANYONE_SRC_ROOT),
            ("pinned", true, "1048576", "1048576", "unexpected-root"),
            ("pinned", true, "1048576", "1048576", MATANYONE_SRC_ROOT),
        ] {
            let archive = dir.path().join(format!("source-{}.tar.gz", outcomes.len()));
            let dest = dir.path().join(format!("src-{}", outcomes.len()));
            let built = Command::new("python3").arg("-c")
                .arg("import io,sys,tarfile; p,root,body=sys.argv[1:]; data=body.encode(); t=tarfile.open(p,'w:gz'); m=tarfile.TarInfo(root+'/hello.txt'); m.size=len(data); t.addfile(m,io.BytesIO(data)); t.close()")
                .arg(&archive).arg(archive_root).arg(body).output().unwrap();
            assert!(built.status.success(), "{built:?}");
            let mut tree = Sha256::new();
            tree.update(b"hello.txt\0");
            tree.update(Sha256::digest(body.as_bytes()));
            let digest = if valid_pin {
                format!("{:x}", tree.finalize())
            } else {
                "0".repeat(64)
            };
            let result = Command::new("python3")
                .arg("-c")
                .arg(FETCH_SRC_PY)
                .arg(format!("file://{}", archive.display()))
                .arg(&dest)
                .arg(digest)
                .arg(compressed_cap)
                .arg(source_cap)
                .arg(MATANYONE_SRC_ROOT)
                .output()
                .unwrap();
            outcomes.push((result.status.success(), dest.exists()));
            if result.status.success() {
                assert_eq!(
                    std::fs::read(dest.join("hello.txt")).unwrap(),
                    body.as_bytes()
                );
            } else {
                assert!(String::from_utf8_lossy(&result.stderr).contains("source rejected"));
            }
        }
        assert_eq!(
            outcomes,
            [
                (false, false),
                (false, false),
                (false, false),
                (false, false),
                (true, true)
            ]
        );
    }

    #[test]
    fn model_warm_selection_preserves_ordinary_runtime_choice() {
        use super::onnx_warm_model_id;
        assert_eq!(onnx_warm_model_id(None), Some("nemo-parakeet-tdt-0.6b-v3"));
        for model in [
            "nemo-parakeet-tdt-0.6b-v2",
            "nemo-canary-1b-v2",
            "owner/custom-onnx",
        ] {
            assert_eq!(onnx_warm_model_id(Some(model)), Some(model));
        }
        for alias in ["canary", "Canary-1b-v2", "canary@cpu"] {
            assert_eq!(onnx_warm_model_id(Some(alias)), Some("nemo-canary-1b-v2"));
        }
        for whisper in ["whisperx-large-v3", "Whisper-large-v3@cpu"] {
            assert_eq!(onnx_warm_model_id(Some(whisper)), None);
        }
    }

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
    fn base_transcription_probe_covers_direct_runtime_modules() {
        assert!(BASE_STT_IMPORT_PROBE.contains("import onnx_asr, onnxruntime, numpy, soundfile"));
        assert!(BASE_STT_IMPORT_PROBE.contains(ONNX_ASR_IMPORT_SENTINEL));
    }

    #[test]
    fn transcription_import_failure_identifies_the_install_phase() {
        let reason = "ImportError: DLL load failed while importing onnxruntime_pybind11_state";
        let base = transcription_import_failure(
            Path::new("/venv/python"),
            "base",
            "the base transcription environment",
            reason,
        );
        assert_eq!(
            base.message,
            format!(
                "the base perception environment cannot import the transcription engine: {reason}"
            )
        );
        assert_eq!(base.cause, format!("/venv/python: {reason}"));
        assert_eq!(
            base.suggested_action.as_deref(),
            Some("Choose Install captions again to rebuild the base transcription environment")
        );

        let final_environment = transcription_import_failure(
            Path::new("/venv/python"),
            "final",
            "the perception environment",
            "ModuleNotFoundError: No module named 'onnx_asr'",
        );
        assert!(final_environment.message.starts_with(
            "the final perception environment cannot import the transcription engine: "
        ));
    }

    #[test]
    fn import_probe_failure_keeps_the_interpreter_error_line() {
        assert_eq!(
            last_line(
                "Traceback (most recent call last):\n  File x\nImportError: DLL load failed\n\n"
            ),
            "ImportError: DLL load failed"
        );
        assert_eq!(last_line(""), "the import check printed nothing");
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
        let root = tempfile::tempdir().unwrap();
        let managed = root.path().join("managed");
        let python = managed.join("cpython-3.12.14").join("python");
        assert_eq!(
            managed_python_identity(python.clone(), "Python 3.12.14\n"),
            Ok(ManagedPython {
                path: python.clone(),
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
                managed_python_identity(managed.join("python"), invalid).is_err(),
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
        assert!(parse_matanyone_probe("ok True\n").unwrap());
        assert!(!parse_matanyone_probe("ok False\n").unwrap());
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
    fn a_folder_that_cannot_be_written_is_named_not_the_network() {
        let raw = "error: failed to create directory `C:\\Users\\p\\AppData\\Local\\ShellX Cut\\perception\\python`: Cannot create a file when that file already exists. (os error 183)";
        let (msg, action) = classify_install_failure(raw, Some(2));
        assert!(msg.contains("could not write their files"), "{msg}");
        assert!(action.contains("Install captions"), "{action}");
        assert!(!action.to_lowercase().contains("internet"), "{action}");
        let (msg, _) = classify_install_failure(
            "error: failed to remove file: Access is denied. (os error 5)",
            Some(2),
        );
        assert!(msg.contains("could not write their files"), "{msg}");
    }

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
