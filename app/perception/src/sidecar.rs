//! sidecar.rs — python instrument orchestration (perception contract).
//!
//! Role: shell out to `py/instruments.py` (venv at app/perception/py/.venv),
//! one JSON-in/JSON-out CLI; validate the emitted PerceptionReport; cache by
//! asset hash (existing receipts/<asset>.perception.json with matching
//! asset_hash AND covering instrument set short-circuits the run). GPU
//! (RTX 5080 WSL) is the sidecar's business — Rust only orchestrates; the
//! sidecar auto-falls back to CPU and records the engine/device in the report.
//! Dependencies: std::process, types.rs, cut-core. Primary callers: server
//! media.transcribe / media.perception jobs (run on blocking threads — these
//! functions BLOCK for the duration of the python run).

use crate::types::{PerceptionReport, Transcript, PERCEPTION_SCHEMA};
use cut_core::{error_codes, CutError};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Progress sink the sidecar drives from the python child's stderr `PROGRESS`
/// lines. `f` is 0.0..1.0 within the CURRENT instrument run; `label` is a
/// short stage string (e.g. "transcribe:chunk 2/7"). Callers map `f` into a job
/// sub-band. Send + Sync because it is invoked from the stderr-draining thread.
pub type SidecarProgress<'a> = dyn Fn(f32, &str) + Send + Sync + 'a;

/// Parse a python `[instruments] PROGRESS <frac> <label>` stderr line into
/// `(frac, label)`. Returns None for any other line. The contract is emitted by
/// `instruments.py::_emit_progress`.
fn parse_progress(line: &str) -> Option<(f32, &str)> {
    let rest = line.split("PROGRESS ").nth(1)?;
    let mut it = rest.splitn(2, ' ');
    let frac: f32 = it.next()?.trim().parse().ok()?;
    let label = it.next().unwrap_or("").trim();
    Some((frac.clamp(0.0, 1.0), label))
}

/// Which instruments to run — media.transcribe runs Words only; media.
/// perception runs the full set (public verb contract); audio-only assets get AudioFull
/// (audio-only media guard: PySceneDetect on a WAV raises VideoOpenFailure and killed
/// the whole import chain — video instruments must not be requested for
/// media with no video stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentSet {
    /// whisperX words only (fast path for the transcribe verb).
    WordsOnly,
    /// words + silence + scenes + beats + loudness.
    Full,
    /// Everything except video instruments (scenes/black/frozen) — the full
    /// honest battery for `probe.kind == "audio"` assets.
    AudioFull,
    /// silence + scenes + loudness — the OUTPUT-fact set for verifying a RENDERED
    /// clip (lufs / black_or_frozen / uniform_border / silence_at_edges) WITHOUT
    /// the slow whisperX transcribe. Used by render.bundle, which renders several
    /// reframed clips per call and whose receipt never inspects the output's
    /// words (cut_on_word verifies the SOURCE render). Skipping "words" + "beats"
    /// avoids rerunning video-only instruments for audio analysis.
    RenderChecks,
    /// `subject` only — the on-demand auto-reframe analysis (YOLO-seg + ByteTrack +
    /// saliency → the normalized [`SubjectTrack`]). Heavy per-frame CV; requested
    /// ONLY by the reframe path, never at import. Video-only (the sidecar drops it
    /// for audio-only assets, like scenes).
    Subject,
}

impl InstrumentSet {
    /// Wire names sent to the sidecar (request "instruments" array).
    fn names(self) -> &'static [&'static str] {
        match self {
            InstrumentSet::WordsOnly => &["words"],
            InstrumentSet::Full => &["words", "silence", "scenes", "beats", "loudness"],
            InstrumentSet::AudioFull => &["words", "silence", "beats", "loudness"],
            InstrumentSet::RenderChecks => &["silence", "scenes", "loudness"],
            InstrumentSet::Subject => &["subject"],
        }
    }

    /// The right set for a probed asset kind ("audio" → AudioFull, everything
    /// else → Full). Single source of truth for the import chain and the
    /// media.perception verb.
    pub fn for_kind(kind: &str) -> Self {
        if kind == "audio" {
            InstrumentSet::AudioFull
        } else {
            InstrumentSet::Full
        }
    }
}

/// Env var: explicit full path to the python interpreter that runs the sidecar.
pub const ENV_PYTHON: &str = "SHELLX_CUT_PYTHON";
/// Env var: directory holding the sidecar payload (`instruments.py` + a `.venv`).
/// On the installed app the desktop shell points this at the `perception/` dir
/// staged beside cutd.exe; the bootstrap also writes the downloaded venv here.
pub const ENV_SIDECAR_DIR: &str = "SHELLX_CUT_SIDECAR_DIR";
/// Env var (defined in cut-media's toolpath): a directory with ffmpeg/ffprobe.
/// We forward it into the sidecar's child env and prepend it to PATH so
/// `instruments.py`'s bare `ffmpeg`/`ffprobe` calls hit the SAME binary the
/// Rust engine resolved — one ffmpeg for the whole app on a cold install.
pub const ENV_FFMPEG_DIR: &str = "SHELLX_CUT_FFMPEG_DIR";
const STALE_PACKAGE_CLEANUP_MARKER: &str = ".shellx-cut-stale-package-cleanup-v1";
const STALE_SIDECARENV_PACKAGES: &[&str] = &["torchcodec"];

/// The platform venv python relative path (POSIX `.venv/bin/python`,
/// Windows `.venv\Scripts\python.exe`).
fn venv_python(base: &Path) -> PathBuf {
    if cfg!(windows) {
        base.join(".venv").join("Scripts").join("python.exe")
    } else {
        base.join(".venv").join("bin").join("python")
    }
}

fn managed_venv_for_python(python: &Path) -> Option<PathBuf> {
    let appdata = appdata_sidecar_dir()?;
    let venv = appdata.join(".venv");
    python.starts_with(&venv).then_some(venv)
}

fn site_packages_dirs(venv: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let windows = venv.join("Lib").join("site-packages");
    if windows.is_dir() {
        dirs.push(windows);
    }
    let lib = venv.join("lib");
    if let Ok(entries) = std::fs::read_dir(lib) {
        for entry in entries.flatten() {
            let path = entry.path().join("site-packages");
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    dirs
}

fn stale_package_entry(name: &str, packages: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    packages.iter().any(|pkg| {
        lower == *pkg
            || lower.starts_with(&format!("{pkg}-"))
            || lower.starts_with(&format!("{pkg}."))
    })
}

fn remove_stale_packages_from_site_packages(
    site_packages: &Path,
    packages: &[&str],
) -> std::io::Result<usize> {
    let mut removed = 0;
    for entry in std::fs::read_dir(site_packages)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !stale_package_entry(name, packages) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
        removed += 1;
    }
    Ok(removed)
}

fn cleanup_stale_managed_venv_packages(python: &Path) -> std::io::Result<usize> {
    let Some(venv) = managed_venv_for_python(python) else {
        return Ok(0);
    };
    let marker = venv.join(STALE_PACKAGE_CLEANUP_MARKER);
    if marker.is_file() {
        return Ok(0);
    }
    let mut removed = 0;
    for site_packages in site_packages_dirs(&venv) {
        removed +=
            remove_stale_packages_from_site_packages(&site_packages, STALE_SIDECARENV_PACKAGES)?;
    }
    std::fs::write(marker, format!("removed={removed}\n"))?;
    Ok(removed)
}

/// Per-user app-data dir where the first-run bootstrap installs the sidecar
/// payload + venv (mirrors cut-media's `appdata_tools_dir`; kept independent so
/// cut-perception need not depend on cut-media). Public so the server's
/// `system.setup_perception` provisioner knows where to build the `.venv`.
pub fn appdata_sidecar_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("ShellX Cut").join("perception"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("ShellX Cut")
                .join("perception")
        })
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|base| base.join("shellx-cut").join("perception"))
    }
}

// ---------------------------------------------------------------------------
// STT model / language setting — a user-chosen transcription model
// (e.g. parakeet-tdt v2 English vs v3 ~25-language multilingual) + an optional
// language hint, persisted in the perception app-data dir and injected into the
// sidecar process env (SHELLX_CUT_STT_MODEL / SHELLX_CUT_STT_LANG) so the python
// instrument transcribes with the chosen model. Lives HERE (next to the spawn)
// so the sidecar can read it directly without a server dependency; the server's
// `system.set_stt_model` verb writes it and `system.doctor` reports it.
// ---------------------------------------------------------------------------

/// Path of the persisted STT setting (JSON `{model?, language?}`).
pub fn stt_settings_path() -> Option<PathBuf> {
    stt_settings_path_with(
        std::env::var_os("SHELLX_CUT_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        appdata_sidecar_dir(),
    )
}

fn stt_settings_path_with(
    shellx_cut_home: Option<PathBuf>,
    appdata_sidecar: Option<PathBuf>,
) -> Option<PathBuf> {
    shellx_cut_home
        .map(|root| root.join("preferences").join("stt.json"))
        .or_else(|| appdata_sidecar.map(|dir| dir.join("stt.json")))
}

/// Pure parse of the STT setting JSON → (model, language). Empty strings → None.
fn parse_stt_setting(json: &str) -> (Option<String>, Option<String>) {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let pick = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    (pick("model"), pick("language"))
}

/// Read the persisted STT (model, language) setting, or (None, None) if unset.
pub fn read_stt_setting() -> (Option<String>, Option<String>) {
    stt_settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| parse_stt_setting(&s))
        .unwrap_or((None, None))
}

/// Persist the STT (model, language) setting. Passing both None CLEARS it (the
/// file is written as `{}`, so the next run falls back to the default model).
pub fn write_stt_setting(model: Option<&str>, language: Option<&str>) -> std::io::Result<()> {
    let path = stt_settings_path().ok_or_else(|| {
        std::io::Error::other("cannot resolve perception app-data dir (HOME/LOCALAPPDATA unset)")
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut obj = serde_json::Map::new();
    if let Some(m) = model.filter(|s| !s.is_empty()) {
        obj.insert("model".into(), serde_json::Value::String(m.to_string()));
    }
    if let Some(l) = language.filter(|s| !s.is_empty()) {
        obj.insert("language".into(), serde_json::Value::String(l.to_string()));
    }
    let body = serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap_or_default();
    std::fs::write(path, body)
}

/// Set the STT model/language env on a perception command from explicit values.
/// Factored out of the spawn so it is unit-testable via `Command::get_envs()`.
pub fn apply_stt_env(cmd: &mut Command, model: Option<&str>, language: Option<&str>) {
    if let Some(m) = model.filter(|s| !s.is_empty()) {
        cmd.env("SHELLX_CUT_STT_MODEL", m);
    }
    if let Some(l) = language.filter(|s| !s.is_empty()) {
        cmd.env("SHELLX_CUT_STT_LANG", l);
    }
}

/// Candidate sidecar-payload base dirs (each may hold `instruments.py` + `.venv`)
/// in precedence order. The dev checkout (`CARGO_MANIFEST_DIR/py`) is included
/// only when the running executable is itself inside the repo workspace. This
/// matters for installed macOS builds made on the same machine that holds the
/// source checkout: `/Applications/ShellX Cut.app` must not silently bind to
/// that checkout's stale `.venv` and trigger Apple's Command Line Tools prompt
/// on launch.
fn sidecar_base_dirs() -> Vec<PathBuf> {
    let env_dir = std::env::var_os(ENV_SIDECAR_DIR)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    let exe = std::env::current_exe().ok();
    sidecar_base_dirs_with(env_dir, exe.as_deref(), appdata_sidecar_dir())
}

fn sidecar_base_dirs_with(
    env_dir: Option<PathBuf>,
    current_exe: Option<&Path>,
    appdata_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // 1. explicit override.
    if let Some(d) = env_dir {
        dirs.push(d);
    }
    // 2. a `perception/` payload beside the running exe (installed layout —
    //    the installer stages instruments.py + requirements there).
    if let Some(exe) = current_exe.and_then(|p| p.parent()) {
        dirs.push(exe.join("perception"));
        // 2b. macOS bundle layout: executables live in Contents/MacOS while
        //     bundle resources land in Contents/Resources. The desktop shell
        //     bridges this with SHELLX_CUT_SIDECAR_DIR when IT spawns cutd,
        //     but a STANDALONE bundled engine (an agent running
        //     `…/Contents/MacOS/cutd serve` directly — a documented headless
        //     use) must find the payload without that env. Harmless on other
        //     platforms (the dir simply doesn't exist).
        if let Some(contents) = exe.parent() {
            dirs.push(contents.join("Resources").join("perception"));
        }
    }
    // 3. the app-data sidecar dir (bootstrap-downloaded venv).
    if let Some(d) = appdata_dir {
        dirs.push(d);
    }
    // 4. the dev checkout — compile-time path, only for repo-launched dev runs.
    if let Some(d) = dev_checkout_sidecar_dir_for_exe(current_exe) {
        dirs.push(d);
    }
    dirs
}

fn dev_checkout_sidecar_dir_for_exe(current_exe: Option<&Path>) -> Option<PathBuf> {
    let exe = current_exe?;
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_workspace = manifest.parent()?;
    exe.starts_with(app_workspace).then(|| manifest.join("py"))
}

fn installed_macos_runtime_without_dev_checkout() -> bool {
    cfg!(target_os = "macos")
        && std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(|exe| dev_checkout_sidecar_dir_for_exe(Some(exe)))
            .is_none()
}

fn fallback_python_for_runtime(installed_macos: bool, appdata_dir: Option<PathBuf>) -> PathBuf {
    if installed_macos {
        return appdata_dir
            .map(|d| venv_python(&d))
            .unwrap_or_else(|| PathBuf::from("shellx-cut-python-not-configured"));
    }
    let bare = if cfg!(windows) { "python" } else { "python3" };
    PathBuf::from(bare)
}

/// Resolve the sidecar entrypoint `(python, instruments.py)` for the CURRENT
/// install layout (fixes the cold-install bug where the old code trusted
/// the build box's `CARGO_MANIFEST_DIR` venv path, which does not exist on an
/// installed app and made the sidecar unfindable).
///
/// Resolution (highest specificity wins):
///   python: SHELLX_CUT_PYTHON override → the `.venv` of the first base dir that
///           has one → bare `python3`/`python` only for developer/runtime surfaces
///           where that cannot trigger Apple's CLT prompt. Installed macOS builds
///           fall back to the expected app-data venv path so a missing sidecar is
///           a normal "not configured" spawn error, not an OS installer dialog.
///   script: `instruments.py` from the first base dir that contains it; if none
///           do, the dev-checkout path (so the existing "script not found"
///           error stays accurate on the dev box).
/// Public so the server can health-check / doctor the sidecar at startup.
pub fn sidecar_paths() -> (PathBuf, PathBuf) {
    let bases = sidecar_base_dirs();

    // --- script: first base dir that actually carries instruments.py ---------
    let script = bases
        .iter()
        .map(|b| b.join("instruments.py"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| bases.last().unwrap().join("instruments.py"));

    // --- python: explicit override, else first base dir with a venv ----------
    if let Some(p) = std::env::var_os(ENV_PYTHON) {
        if !p.is_empty() {
            return (PathBuf::from(p), script);
        }
    }
    if let Some(py) = sidecar_python_from_bases(&bases) {
        return (py, script);
    }
    (
        fallback_python_for_runtime(
            installed_macos_runtime_without_dev_checkout(),
            appdata_sidecar_dir(),
        ),
        script,
    )
}

/// Resolve only an app-managed or explicitly configured sidecar Python.
///
/// `system.doctor` runs on launch. On clean macOS, spawning bare `python3`
/// opens Apple's Command Line Tools installer prompt, so passive environment
/// scans must not use the PATH fallback or the build machine's checkout unless
/// this process is actually a repo-launched dev binary.
pub fn configured_sidecar_python() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(ENV_PYTHON) {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    sidecar_python_from_bases(&sidecar_base_dirs())
}

fn sidecar_python_from_bases(bases: &[PathBuf]) -> Option<PathBuf> {
    sidecar_python_from_bases_for_platform(bases, cfg!(target_os = "macos"))
}

fn sidecar_python_from_bases_for_platform(bases: &[PathBuf], macos: bool) -> Option<PathBuf> {
    bases
        .iter()
        .map(|b| venv_python(b))
        .find(|p| venv_python_is_usable_for_platform(p, macos))
}

fn venv_python_is_usable_for_platform(python: &Path, macos: bool) -> bool {
    if !python.exists() {
        return false;
    }
    if !macos {
        return true;
    }
    !is_macos_apple_python_stub_venv(python)
}

fn is_macos_apple_python_stub_venv(python: &Path) -> bool {
    let Some(bin_dir) = python.parent() else {
        return false;
    };
    let Some(venv_dir) = bin_dir.parent() else {
        return false;
    };
    let cfg = std::fs::read_to_string(venv_dir.join("pyvenv.cfg")).unwrap_or_default();
    cfg.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "home = /usr/bin" || trimmed.starts_with("executable = /usr/bin/python")
    }) || resolves_to_usr_bin_python(python)
}

fn resolves_to_usr_bin_python(python: &Path) -> bool {
    let resolved = std::fs::canonicalize(python).unwrap_or_else(|_| python.to_path_buf());
    resolved == Path::new("/usr/bin/python")
        || resolved == Path::new("/usr/bin/python3")
        || resolved
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("python3."))
            && resolved.parent() == Some(Path::new("/usr/bin"))
}

/// Run the sidecar over `media_path`, writing/refreshing
/// `<receipts_dir>/<asset_id>.perception.json`. Cache: when that file already
/// exists AND its asset_hash equals `asset_hash` AND its `instruments_run`
/// covers the requested set, returns it without running python.
/// `model` overrides the whisperX model (default "small" for dev, perception contract).
pub fn run_instruments(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
) -> Result<PerceptionReport, CutError> {
    run_instruments_progress(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        None,
    )
}

/// Progress-aware variant of [`run_instruments`]: `progress` (when Some) is
/// driven from the python child's `PROGRESS` stderr lines during the run, so a
/// long transcription reports real sub-progress instead of a frozen number
///. All other behaviour (cache, validate, persist) is identical.
pub fn run_instruments_progress(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
    progress: Option<&SidecarProgress>,
) -> Result<PerceptionReport, CutError> {
    // ---- cache check -----------------------------------------------------
    if let Some(cached) = load_report(receipts_dir, asset_id)? {
        let covers = set
            .names()
            .iter()
            .all(|n| cached.instruments_run.iter().any(|r| r == n));
        if cached.asset_hash == asset_hash && covers {
            tracing::debug!(asset_id, "perception cache hit");
            return Ok(cached);
        }
    }

    // ---- spawn python (shared helper) ------------------------------------
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": asset_id,
        "asset_hash": asset_hash,
        "instruments": set.names(),
        "whisper_model": model.unwrap_or("small"),
    });
    tracing::info!(asset_id, instruments = ?set.names(), "running perception sidecar");
    let stdout = spawn_sidecar_streaming(&request, progress)?;

    // ---- validate --------------------------------------------------------
    let report: PerceptionReport = serde_json::from_str(stdout.trim()).map_err(|e| {
        CutError::new(
            error_codes::SIDECAR,
            "sidecar emitted invalid PerceptionReport JSON",
            format!(
                "{e}; first 200 chars: {}",
                stdout.chars().take(200).collect::<String>()
            ),
        )
    })?;
    if report.schema != PERCEPTION_SCHEMA {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "sidecar emitted wrong schema",
            format!("found '{}', expected '{PERCEPTION_SCHEMA}'", report.schema),
        ));
    }
    if report.asset_hash != asset_hash {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "sidecar echoed a different asset_hash",
            format!("sent '{asset_hash}', got '{}'", report.asset_hash),
        ));
    }

    // ---- persist ---------------------------------------------------------
    // ProjectStore creates receipts/ before any perception job starts. Do not
    // recreate it here: project.delete may remove the project while a sidecar
    // is still finishing, and a late create_dir_all would resurrect a deleted
    // .cutproj with only stale analysis files inside it.
    require_live_receipts_dir(receipts_dir)?;
    let path = receipts_dir.join(format!("{asset_id}.perception.json"));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&report).map_err(CutError::from)?,
    )?;
    tracing::info!(asset_id, path = %path.display(), "perception report written");
    Ok(report)
}

fn require_live_receipts_dir(receipts_dir: &Path) -> Result<(), CutError> {
    if receipts_dir.is_dir() {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::IO,
        "project receipt directory disappeared before analysis completed",
        format!(
            "{} is no longer an existing directory; the project may have been deleted",
            receipts_dir.display()
        ),
    ))
}

/// Spawn the Python perception sidecar with a JSON `request` on stdin, return its
/// stdout on success. Shared by `run_instruments`, `run_subject`, and
/// `build_contact_sheet`. Forwards the engine-resolved ffmpeg dir (so the sidecar's
/// bare ffmpeg/ffprobe calls hit the same binary), and on a non-zero exit lifts the
/// sidecar's `{"error":{...}}` contract message (or the stderr tail) into a CutError.
fn spawn_sidecar(request: &serde_json::Value) -> Result<String, CutError> {
    spawn_sidecar_streaming(request, None)
}

/// Streaming variant of [`spawn_sidecar`]: drains the child's stderr line-by-line
/// on a scoped thread, forwarding `PROGRESS <frac> <label>` lines to `progress`
/// while stdout is read concurrently on this thread. Concurrent drain is required
/// — the sidecar emits its (potentially >64 KB) report JSON to stdout in one write
/// at the very end, which would deadlock a serial "read stderr then stdout" loop
/// once the stdout pipe buffer fills. On a non-zero exit it lifts the
/// sidecar's `{"error":{...}}` contract (or the stderr tail) into a CutError.
fn spawn_sidecar_streaming(
    request: &serde_json::Value,
    progress: Option<&SidecarProgress>,
) -> Result<String, CutError> {
    let (python, script) = sidecar_paths();
    if !script.exists() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "instruments.py not found",
            format!("expected at {}", script.display()),
        )
        .with_suggested_action("check the app/perception/py checkout"));
    }
    match cleanup_stale_managed_venv_packages(&python) {
        Ok(removed) if removed > 0 => {
            tracing::info!(
                removed,
                "removed stale packages from managed perception venv"
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "could not remove stale packages from managed perception venv");
        }
    }
    let mut cmd = Command::new(&python);
    cmd.arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(ffdir) = std::env::var_os(ENV_FFMPEG_DIR) {
        if !ffdir.is_empty() {
            cmd.env(ENV_FFMPEG_DIR, &ffdir);
            let sep = if cfg!(windows) { ";" } else { ":" };
            let new_path = match std::env::var_os("PATH") {
                Some(p) => {
                    let mut s = ffdir.clone();
                    s.push(sep);
                    s.push(p);
                    s
                }
                None => ffdir.clone(),
            };
            cmd.env("PATH", new_path);
        }
    }
    // inject the user-chosen STT model / language so instruments.py
    // transcribes with it (the python reads SHELLX_CUT_STT_MODEL; the whisperX
    // fallback reads SHELLX_CUT_STT_LANG). Unset → the python's built-in default.
    {
        let (model, language) = read_stt_setting();
        apply_stt_env(&mut cmd, model.as_deref(), language.as_deref());
    }
    let suggested_action = if cfg!(windows) {
        "perception/transcription needs the Python sidecar. Run the ShellX Cut \
         bootstrap (Tools > Set up perception), or manually: \
         py -3 -m venv \"%LOCALAPPDATA%\\ShellX Cut\\perception\\.venv\" && \
         \"%LOCALAPPDATA%\\ShellX Cut\\perception\\.venv\\Scripts\\pip\" install \
         -r requirements.txt"
    } else {
        "create the venv: python3 -m venv app/perception/py/.venv && \
         .venv/bin/pip install -r app/perception/py/requirements.txt"
    };
    let mut child = cmd.spawn().map_err(|e| {
        CutError::new(
            error_codes::SIDECAR,
            "failed to spawn perception sidecar",
            format!("{}: {e}", python.display()),
        )
        .with_suggested_action(suggested_action)
    })?;
    // Write the (small) request and CLOSE stdin so the child's `json.load(stdin)`
    // returns. Done before reading output — the request fits the pipe buffer, so
    // this cannot deadlock against the child, which reads stdin first.
    child
        .stdin
        .take()
        .expect("stdin piped above")
        .write_all(request.to_string().as_bytes())
        .map_err(|e| {
            CutError::new(
                error_codes::SIDECAR,
                "failed writing sidecar request",
                e.to_string(),
            )
        })?;

    let stdout_pipe = child.stdout.take().expect("stdout piped above");
    let stderr_pipe = child.stderr.take().expect("stderr piped above");
    let mut stdout = String::new();
    // Drain stderr on a scoped thread (parse PROGRESS → callback, keep a tail for
    // error reporting) while this thread reads stdout to EOF — no deadlock.
    let stderr_tail = std::thread::scope(|scope| -> Result<Vec<String>, CutError> {
        let drain = scope.spawn(|| {
            let mut tail: Vec<String> = Vec::new();
            for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                if let (Some(cb), Some((frac, label))) = (progress, parse_progress(&line)) {
                    cb(frac, label);
                }
                tail.push(line);
                if tail.len() > 40 {
                    tail.remove(0);
                }
            }
            tail
        });
        BufReader::new(stdout_pipe)
            .read_to_string(&mut stdout)
            .map_err(|e| {
                CutError::new(
                    error_codes::SIDECAR,
                    "sidecar stdout read failed",
                    e.to_string(),
                )
            })?;
        Ok(drain.join().unwrap_or_default())
    })?;

    let status = child
        .wait()
        .map_err(|e| CutError::new(error_codes::SIDECAR, "sidecar wait failed", e.to_string()))?;
    if !status.success() {
        let cause = serde_json::from_str::<serde_json::Value>(stdout.trim())
            .ok()
            .and_then(|v| v.get("error").cloned())
            .map(|e| e.to_string())
            .unwrap_or_else(|| {
                let tail: Vec<&str> = stderr_tail
                    .iter()
                    .rev()
                    .take(8)
                    .map(String::as_str)
                    .collect();
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            });
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!("perception sidecar failed (exit {:?})", status.code()),
            cause,
        ));
    }
    Ok(stdout)
}

/// Director-model path: run ONLY the `subject` instrument with an
/// explicit `preset` (fixes the preset never reaching class selection on the old
/// `run_instruments(Subject)` path) and an optional per-scene `direction` brief from
/// the foundation model. Always a fresh run (no cache — reframe is explicit, and
/// preset/direction are variants; the reframe temp's asset_id is unique anyway).
/// Returns the SubjectTrack the moving-crop render consumes.
pub fn run_subject(
    media_path: &Path,
    asset_hash: &str,
    preset: &str,
    direction: Option<serde_json::Value>,
) -> Result<crate::types::SubjectTrack, CutError> {
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": "reframe-subject",
        "asset_hash": asset_hash,
        "instruments": ["subject"],
        "subject_preset": preset,
        "direction": direction,
    });
    tracing::info!(
        preset,
        directed = direction.is_some(),
        "running subject instrument"
    );
    let stdout = spawn_sidecar(&request)?;
    let report: PerceptionReport = serde_json::from_str(stdout.trim()).map_err(|e| {
        CutError::new(
            error_codes::SIDECAR,
            "sidecar emitted invalid PerceptionReport JSON",
            format!(
                "{e}; first 200 chars: {}",
                stdout.chars().take(200).collect::<String>()
            ),
        )
    })?;
    if report.schema != PERCEPTION_SCHEMA {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "sidecar emitted wrong schema",
            format!("found '{}', expected '{PERCEPTION_SCHEMA}'", report.schema),
        ));
    }
    report.subject_track.ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "subject instrument produced no track",
            "the perception sidecar may be missing the CV deps (torchvision/supervision)",
        )
    })
}

/// Director-model: build the SPARSE per-scene contact sheet (one annotated keyframe
/// per scene + candidate subjects labeled A/B/C left→right) the foundation model
/// reads to direct the whole clip in ONE call. Writes `contact_sheet.jpg` into
/// `out_dir` and returns the raw sidecar JSON (`contact_sheet` path + per-scene
/// `scenes[].candidates`). Not a PerceptionReport — its own document shape.
pub fn build_contact_sheet(
    media_path: &Path,
    out_dir: &Path,
    preset: &str,
) -> Result<serde_json::Value, CutError> {
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": "reframe-contact",
        "asset_hash": "",
        "contact_sheet": out_dir,
        "subject_preset": preset,
    });
    tracing::info!(preset, "building director contact sheet");
    let stdout = spawn_sidecar(&request)?;
    let sheet: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|e| {
        CutError::new(
            error_codes::SIDECAR,
            "contact-sheet sidecar emitted invalid JSON",
            format!(
                "{e}; first 200 chars: {}",
                stdout.chars().take(200).collect::<String>()
            ),
        )
    })?;
    Ok(sheet)
}

/// Director-model v2 QC: review a reframed OUTPUT — per-scene frames + composition
/// hints (subject_present, face centering, headroom, `needs_review`) tiled into a
/// review sheet the model reads to judge + correct the framing. Returns the raw
/// sidecar JSON (`qc_sheet` path + per-scene `scenes` + `review_count`).
pub fn build_qc_sheet(
    media_path: &Path,
    out_dir: &Path,
    preset: &str,
) -> Result<serde_json::Value, CutError> {
    let request = serde_json::json!({
        "media_path": media_path.canonicalize().unwrap_or_else(|_| media_path.to_path_buf()),
        "asset_id": "reframe-qc",
        "asset_hash": "",
        "qc_sheet": out_dir,
        "subject_preset": preset,
    });
    tracing::info!(preset, "building director QC sheet");
    let stdout = spawn_sidecar(&request)?;
    serde_json::from_str(stdout.trim()).map_err(|e| {
        CutError::new(
            error_codes::SIDECAR,
            "qc-sheet sidecar emitted invalid JSON",
            format!(
                "{e}; first 200 chars: {}",
                stdout.chars().take(200).collect::<String>()
            ),
        )
    })
}

/// Transcribe only (public verb contract media.transcribe): runs InstrumentSet::WordsOnly
/// and also writes `<receipts_dir>/<asset_id>.words.json` (the
/// Asset::transcript target the UI fetches).
pub fn transcribe(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    model: Option<&str>,
) -> Result<Transcript, CutError> {
    transcribe_progress(media_path, receipts_dir, asset_id, asset_hash, model, None)
}

/// Progress-aware [`transcribe`]: `progress` (when Some) is driven from the
/// python child's `PROGRESS` stderr lines, so the enrich job shows real
/// transcription sub-progress on long footage.
pub fn transcribe_progress(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    model: Option<&str>,
    progress: Option<&SidecarProgress>,
) -> Result<Transcript, CutError> {
    let report = run_instruments_progress(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        InstrumentSet::WordsOnly,
        model,
        progress,
    )?;
    let transcript = transcript_or_empty(report, asset_id)?;
    let path = receipts_dir.join(format!("{asset_id}.words.json"));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&transcript).map_err(CutError::from)?,
    )?;
    Ok(transcript)
}

/// Resolve a transcript from a perception report. A no-audio clip (video-only
/// b-roll / silent render) or audio with no detectable speech legitimately
/// yields NO words: the sidecar drops "words" from instruments_run to signal an
/// honest skip → return an empty transcript rather than failing the import chain
/// (the honest no-speech path). "words" present in instruments_run but absent
/// from the report IS a real sidecar bug → hard error.
fn transcript_or_empty(report: PerceptionReport, asset_id: &str) -> Result<Transcript, CutError> {
    match report.words {
        Some(t) => Ok(t),
        None if !report.instruments_run.iter().any(|r| r == "words") => Ok(Transcript {
            asset: asset_id.to_string(),
            model: "none@no-speech".into(),
            language: None,
            words: vec![],
        }),
        None => Err(CutError::new(
            error_codes::SIDECAR,
            "sidecar ran but produced no transcript",
            "words instrument was requested (and reported as run) yet 'words' is missing from the report",
        )),
    }
}

/// Load an existing perception.json for an asset (None when not yet run).
/// Validates the schema tag; a mismatched schema is an error, not a silent None.
pub fn load_report(
    receipts_dir: &Path,
    asset_id: &str,
) -> Result<Option<PerceptionReport>, CutError> {
    let path = receipts_dir.join(format!("{asset_id}.perception.json"));
    if !path.exists() {
        return Ok(None);
    }
    let report: PerceptionReport =
        serde_json::from_str(&std::fs::read_to_string(&path)?).map_err(CutError::from)?;
    if report.schema != PERCEPTION_SCHEMA {
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!("perception schema mismatch in {}", path.display()),
            format!(
                "found '{}', expected '{PERCEPTION_SCHEMA}' — re-run media.perception",
                report.schema
            ),
        ));
    }
    Ok(Some(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_analysis_never_recreates_a_deleted_project() {
        let parent = tempfile::tempdir().unwrap();
        let project = parent.path().join("deleted.cutproj");
        let receipts = project.join("receipts");
        std::fs::create_dir_all(&receipts).unwrap();
        std::fs::remove_dir_all(&project).unwrap();

        let error = require_live_receipts_dir(&receipts)
            .expect_err("a deleted project must reject late receipt persistence");
        assert_eq!(error.code, error_codes::IO);
        assert!(error.message.contains("disappeared"), "{error:?}");
        assert!(
            !project.exists(),
            "the persistence guard must not recreate the deleted .cutproj"
        );
    }

    /// the STT setting JSON parses (model + language; empties → None).
    #[test]
    fn stt_setting_parses() {
        assert_eq!(
            parse_stt_setting(r#"{"model":"nemo-parakeet-tdt-0.6b-v3","language":"de"}"#),
            (Some("nemo-parakeet-tdt-0.6b-v3".into()), Some("de".into()))
        );
        assert_eq!(parse_stt_setting("{}"), (None, None));
        assert_eq!(
            parse_stt_setting(r#"{"model":"  ","language":""}"#),
            (None, None)
        );
        assert_eq!(parse_stt_setting("not json"), (None, None));
        // model alone is valid (language optional).
        assert_eq!(
            parse_stt_setting(r#"{"model":"whisperx-small"}"#),
            (Some("whisperx-small".into()), None)
        );
    }

    /// apply_stt_env sets ONLY the provided vars on the command, so the
    /// spawned python sees the chosen model/language (verified without spawning
    /// via Command::get_envs()).
    #[test]
    fn stt_env_injection() {
        use std::ffi::OsStr;
        let envs = |c: &Command| -> std::collections::HashMap<String, Option<String>> {
            c.get_envs()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.map(|x| x.to_string_lossy().into_owned()),
                    )
                })
                .collect()
        };
        // Both set.
        let mut c = Command::new("true");
        apply_stt_env(&mut c, Some("nemo-parakeet-tdt-0.6b-v3"), Some("fr"));
        let e = envs(&c);
        assert_eq!(
            e.get("SHELLX_CUT_STT_MODEL")
                .and_then(|v| v.clone())
                .as_deref(),
            Some("nemo-parakeet-tdt-0.6b-v3")
        );
        assert_eq!(
            e.get("SHELLX_CUT_STT_LANG")
                .and_then(|v| v.clone())
                .as_deref(),
            Some("fr")
        );
        // None / empty → no env set (python falls back to its default).
        let mut c2 = Command::new("true");
        apply_stt_env(&mut c2, None, Some(""));
        assert!(!c2
            .get_envs()
            .any(|(k, _)| k == OsStr::new("SHELLX_CUT_STT_MODEL")));
        assert!(!c2
            .get_envs()
            .any(|(k, _)| k == OsStr::new("SHELLX_CUT_STT_LANG")));
    }

    #[test]
    fn configured_sidecar_python_never_returns_bare_python_fallback() {
        let old_py = std::env::var_os(ENV_PYTHON);
        let tmp = tempfile::tempdir().unwrap();
        std::env::remove_var(ENV_PYTHON);

        assert_eq!(
            sidecar_python_from_bases(&[tmp.path().join("perception")]),
            None,
            "passive scans must not fall back to bare python3/python"
        );

        let venv_py = if cfg!(windows) {
            tmp.path()
                .join("ready")
                .join(".venv")
                .join("Scripts")
                .join("python.exe")
        } else {
            tmp.path()
                .join("ready")
                .join(".venv")
                .join("bin")
                .join("python")
        };
        std::fs::create_dir_all(venv_py.parent().unwrap()).unwrap();
        std::fs::write(&venv_py, "").unwrap();
        assert_eq!(
            sidecar_python_from_bases(&[tmp.path().join("empty"), tmp.path().join("ready")]),
            Some(venv_py)
        );

        let explicit = tmp.path().join("custom-python");
        std::env::set_var(ENV_PYTHON, &explicit);
        assert_eq!(configured_sidecar_python(), Some(explicit));

        match old_py {
            Some(v) => std::env::set_var(ENV_PYTHON, v),
            None => std::env::remove_var(ENV_PYTHON),
        }
    }

    #[test]
    fn macos_sidecar_python_skips_apple_stub_venvs() {
        let tmp = tempfile::tempdir().unwrap();
        let stale = tmp.path().join("stale");
        let fresh = tmp.path().join("fresh");
        let stale_py = stale.join(".venv").join("bin").join("python");
        let fresh_py = fresh.join(".venv").join("bin").join("python");
        std::fs::create_dir_all(stale_py.parent().unwrap()).unwrap();
        std::fs::create_dir_all(fresh_py.parent().unwrap()).unwrap();
        std::fs::write(&stale_py, "").unwrap();
        std::fs::write(&fresh_py, "").unwrap();
        std::fs::write(
            stale.join(".venv").join("pyvenv.cfg"),
            "home = /usr/bin\nexecutable = /usr/bin/python3.12\n",
        )
        .unwrap();
        std::fs::write(
            fresh.join(".venv").join("pyvenv.cfg"),
            "home = /Users/example/.local/share/uv/python/cpython-3.12\n",
        )
        .unwrap();

        assert_eq!(
            sidecar_python_from_bases_for_platform(&[stale.clone(), fresh.clone()], true),
            Some(fresh_py),
            "installed macOS must skip venvs backed by Apple's python stub"
        );
        assert_eq!(
            sidecar_python_from_bases_for_platform(&[stale], true),
            None,
            "a stale Apple-stub venv must report missing instead of opening the CLT prompt"
        );
    }

    #[test]
    fn standalone_macos_bundle_cutd_sees_contents_resources_perception() {
        // A bundled cutd run DIRECTLY (no desktop shell, no env) must probe the
        // macOS bundle's Contents/Resources/perception — that is where tauri
        // stages the payload, one level up + sideways from Contents/MacOS.
        let exe = PathBuf::from("/Applications/ShellX Cut.app/Contents/MacOS/cutd");
        let dirs = sidecar_base_dirs_with(None, Some(&exe), None);
        let want = PathBuf::from("/Applications/ShellX Cut.app/Contents/Resources/perception");
        assert!(
            dirs.iter().any(|p| p == &want),
            "bundle-sibling Resources/perception missing from the ladder: {dirs:?}"
        );
    }

    #[test]
    fn installed_app_does_not_use_compile_time_sidecar_checkout() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let app_workspace = manifest.parent().expect("manifest has app parent");
        let dev_exe = app_workspace.join("target").join("debug").join("cutd");
        assert_eq!(
            dev_checkout_sidecar_dir_for_exe(Some(&dev_exe)),
            Some(manifest.join("py")),
            "repo-launched dev binaries can use the source sidecar"
        );

        let installed_exe = if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\ShellX Cut\cutd.exe")
        } else {
            PathBuf::from("/Applications/ShellX Cut.app/Contents/MacOS/cutd")
        };
        assert_eq!(
            dev_checkout_sidecar_dir_for_exe(Some(&installed_exe)),
            None,
            "installed apps must not use the build checkout sidecar"
        );

        let appdata = tempfile::tempdir().unwrap();
        let dirs = sidecar_base_dirs_with(
            None,
            Some(&installed_exe),
            Some(appdata.path().to_path_buf()),
        );
        assert!(
            !dirs.iter().any(|p| p == &manifest.join("py")),
            "installed sidecar base dirs must exclude the compile-time checkout"
        );
    }

    #[test]
    fn installed_macos_fallback_python_is_appdata_venv_not_bare_python() {
        let appdata =
            PathBuf::from("/Users/test/Library/Application Support/ShellX Cut/perception");
        assert_eq!(
            fallback_python_for_runtime(true, Some(appdata.clone())),
            venv_python(&appdata),
            "installed macOS sidecar attempts should fail against the managed venv path, not spawn bare python3"
        );
        assert_eq!(
            fallback_python_for_runtime(true, None),
            PathBuf::from("shellx-cut-python-not-configured")
        );
        assert_eq!(
            fallback_python_for_runtime(false, Some(appdata)),
            PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
            "dev/non-mac runtime keeps the historical bare fallback"
        );
    }

    #[test]
    fn isolated_shellx_home_owns_stt_preferences_without_hiding_shared_sidecar_tools() {
        let isolated = PathBuf::from("/tmp/shellx-cut-isolated");
        let shared = PathBuf::from("/Users/test/Library/Application Support/ShellX Cut/perception");
        assert_eq!(
            stt_settings_path_with(Some(isolated.clone()), Some(shared.clone())),
            Some(isolated.join("preferences").join("stt.json")),
            "an isolated engine must not read or mutate the signed-in user's STT preference"
        );
        assert_eq!(
            stt_settings_path_with(None, Some(shared.clone())),
            Some(shared.join("stt.json")),
            "normal installed runs preserve the existing app-data preference location"
        );
        assert_eq!(
            sidecar_base_dirs_with(None, None, Some(shared.clone())).last(),
            Some(&shared),
            "preference isolation does not move or hide the installed perception runtime"
        );
    }

    #[test]
    fn stale_package_cleanup_removes_only_known_stale_entries() {
        let dir = tempfile::tempdir().unwrap();
        let site = dir.path().join("site-packages");
        std::fs::create_dir_all(site.join("torchcodec")).unwrap();
        std::fs::create_dir_all(site.join("torchcodec-0.6.0.dist-info")).unwrap();
        std::fs::write(site.join("torchcodec.py"), "").unwrap();
        std::fs::create_dir_all(site.join("torch")).unwrap();
        std::fs::create_dir_all(site.join("torchaudio")).unwrap();
        std::fs::create_dir_all(site.join("not_torchcodec")).unwrap();

        let removed =
            remove_stale_packages_from_site_packages(&site, STALE_SIDECARENV_PACKAGES).unwrap();

        assert_eq!(removed, 3);
        assert!(!site.join("torchcodec").exists());
        assert!(!site.join("torchcodec-0.6.0.dist-info").exists());
        assert!(!site.join("torchcodec.py").exists());
        assert!(site.join("torch").exists());
        assert!(site.join("torchaudio").exists());
        assert!(site.join("not_torchcodec").exists());
    }

    /// Minimal valid persisted report for cache tests.
    fn report(instruments: &[&str]) -> PerceptionReport {
        PerceptionReport {
            schema: PERCEPTION_SCHEMA.into(),
            asset_hash: "sha256:cafe".into(),
            source_path: "/gone.mp4".into(),
            instruments_run: instruments.iter().map(|s| s.to_string()).collect(),
            words: None,
            silences: vec![],
            scenes: vec![],
            beats: None,
            loudness: None,
            black_spans: vec![],
            frozen_spans: vec![],
            content_bbox: None,
            subject_track: None,
            speaker_turns: vec![],
            diarization: None,
        }
    }

    /// A no-audio/no-speech clip has "words"
    /// dropped from instruments_run → empty transcript (not an import failure);
    /// "words" still listed as run but absent IS a sidecar bug → error.
    #[test]
    fn transcript_empty_when_words_skipped_but_errors_when_dropped() {
        // Skipped (no audio): "words" not in instruments_run → empty transcript.
        let skipped = transcript_or_empty(report(&["silence", "scenes"]), "a1").unwrap();
        assert!(skipped.words.is_empty());
        assert_eq!(skipped.asset, "a1");
        // Bug: "words" reported as run yet missing → hard error.
        let bug = transcript_or_empty(report(&["words", "silence"]), "a1");
        assert!(bug.is_err(), "missing words while listed as run must error");
        // Present words pass through unchanged.
        let mut r = report(&["words"]);
        r.words = Some(Transcript {
            asset: "a1".into(),
            model: "whisperx-small@cpu".into(),
            language: Some("en".into()),
            words: vec![],
        });
        assert_eq!(
            transcript_or_empty(r, "a1").unwrap().model,
            "whisperx-small@cpu"
        );
    }

    /// Cache round-trip: a persisted report with matching hash + instrument
    /// coverage is returned WITHOUT spawning python (proved by pointing at a
    /// media path that does not exist — a real run would have to fail).
    #[test]
    fn cache_hit_skips_python() {
        let dir = tempfile::tempdir().unwrap();
        let r = report(&["words", "silence", "scenes", "beats", "loudness"]);
        std::fs::write(
            dir.path().join("a1.perception.json"),
            serde_json::to_string(&r).unwrap(),
        )
        .unwrap();
        let got = run_instruments(
            Path::new("/definitely/missing.mp4"),
            dir.path(),
            "a1",
            "sha256:cafe",
            InstrumentSet::Full,
            None,
        )
        .expect("cache hit must not need the media file");
        assert_eq!(got.asset_hash, "sha256:cafe");
    }

    /// A WordsOnly cached report must NOT satisfy a Full request: the
    /// missing-media run then errors, which proves the cache was bypassed.
    #[test]
    fn partial_cache_does_not_satisfy_full() {
        let dir = tempfile::tempdir().unwrap();
        let r = report(&["words"]);
        std::fs::write(
            dir.path().join("a1.perception.json"),
            serde_json::to_string(&r).unwrap(),
        )
        .unwrap();
        let got = run_instruments(
            Path::new("/definitely/missing.mp4"),
            dir.path(),
            "a1",
            "sha256:cafe",
            InstrumentSet::Full,
            None,
        );
        assert!(got.is_err(), "partial cache must force a re-run");
    }

    /// audio-only media guard: AudioFull never requests video instruments, and a cached
    /// audio report (no "scenes") satisfies an AudioFull request — but must
    /// NOT satisfy a Full one.
    #[test]
    fn audio_set_skips_scenes_and_caches_correctly() {
        assert!(
            !InstrumentSet::AudioFull.names().contains(&"scenes"),
            "audio set must not request video instruments"
        );
        assert_eq!(InstrumentSet::for_kind("audio"), InstrumentSet::AudioFull);
        assert_eq!(InstrumentSet::for_kind("video"), InstrumentSet::Full);
        let dir = tempfile::tempdir().unwrap();
        let r = report(&["words", "silence", "beats", "loudness"]);
        std::fs::write(
            dir.path().join("a1.perception.json"),
            serde_json::to_string(&r).unwrap(),
        )
        .unwrap();
        // Cache hit for AudioFull (missing media path proves no python run).
        run_instruments(
            Path::new("/definitely/missing.wav"),
            dir.path(),
            "a1",
            "sha256:cafe",
            InstrumentSet::AudioFull,
            None,
        )
        .expect("audio cache must satisfy an AudioFull request");
        // The same cached report must not satisfy Full.
        assert!(run_instruments(
            Path::new("/definitely/missing.wav"),
            dir.path(),
            "a1",
            "sha256:cafe",
            InstrumentSet::Full,
            None,
        )
        .is_err());
    }

    /// Stale hash (file changed since analysis) also bypasses the cache.
    #[test]
    fn stale_hash_bypasses_cache() {
        let dir = tempfile::tempdir().unwrap();
        let r = report(&["words", "silence", "scenes", "beats", "loudness"]);
        std::fs::write(
            dir.path().join("a1.perception.json"),
            serde_json::to_string(&r).unwrap(),
        )
        .unwrap();
        let got = run_instruments(
            Path::new("/definitely/missing.mp4"),
            dir.path(),
            "a1",
            "sha256:OTHER",
            InstrumentSet::Full,
            None,
        );
        assert!(got.is_err(), "stale hash must force a re-run");
    }

    /// Schema-mismatched persisted report is an error, not a silent None.
    #[test]
    fn load_report_rejects_wrong_schema() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a1.perception.json"),
            r#"{"schema":"other/9","asset_hash":"x","source_path":"y"}"#,
        )
        .unwrap();
        assert!(load_report(dir.path(), "a1").is_err());
    }
}
