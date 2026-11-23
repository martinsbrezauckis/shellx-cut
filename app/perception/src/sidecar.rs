//! sidecar.rs — python instrument orchestration (perception contract).
//!
//! Role: shell out to `py/instruments.py` (venv at app/perception/py/.venv),
//! one JSON-in/JSON-out CLI; validate the emitted PerceptionReport; cache by
//! asset hash (existing receipts/<asset>.perception.json with matching
//! asset_hash AND covering instrument set short-circuits the run). GPU
//! selection is the sidecar's business — Rust only orchestrates; the
//! sidecar auto-falls back to CPU and records the engine/device in the report.
//! Dependencies: std::process, types.rs, cut-core. Primary callers: server
//! media.transcribe / media.perception jobs (run on blocking threads — these
//! functions BLOCK for the duration of the python run).

use crate::types::{PerceptionReport, Transcript, PERCEPTION_SCHEMA};
use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
mod installed_layout_tests;
mod native_runtime;
mod owned;
mod python_missing;
#[cfg(test)]
mod python_missing_tests;
mod real_python;
/// True when `candidate` really runs as Python 3 (a bounded version probe).
pub use real_python::runs_as_python;
#[cfg(test)]
mod report_files_tests;
mod runtime_cache;
#[cfg(test)]
mod runtime_cache_tests;
use native_runtime::sidecar_runtime_unchecked;
pub use native_runtime::{
    apply_python_command_policy, configured_sidecar_python, native_runtime_context,
    premium_sidecar_runtime, python_command_args, sidecar_runtime, SidecarRuntime,
    PYTHONDONTWRITEBYTECODE_ENV,
};
pub use owned::{
    build_contact_sheet_owned, build_qc_sheet_owned, run_instruments_owned,
    run_instruments_owned_ephemeral, run_instruments_owned_progress, run_subject_owned,
    transcribe_owned_progress,
};
pub use python_missing::{
    captions_not_installed_error, captions_setup_action, check_interpreter,
    is_captions_not_installed, is_windows_store_shortcut_run, InterpreterProblem,
    CAPTIONS_NOT_INSTALLED_MESSAGE,
};
use runtime_cache::{cache_matches_runtime_context, runtime_context_provenance};
pub use runtime_cache::{effective_stt_selection, EffectiveSttSelection};
const MAX_PERCEPTION_REPORT_BYTES: u64 = 64 * 1024 * 1024;

/// Progress sink the sidecar drives from the python child's stderr `PROGRESS`
/// lines. `f` is 0.0..1.0 within the CURRENT instrument run; `label` is a
/// short stage string (e.g. "transcribe:chunk 2/7"). Callers retain it in an
/// [`Arc`] because the bounded child owner drains stderr on its worker thread.
pub type SidecarProgress = dyn Fn(f32, &str) + Send + Sync;

#[cfg(test)]
static NATIVE_RUNTIME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serialize the tests that read or change process environment. A test that
/// panics while holding the lock poisons it; the next test must still run, so
/// the guard is recovered instead of unwrapped.
#[cfg(test)]
pub(super) fn lock_native_runtime_env() -> std::sync::MutexGuard<'static, ()> {
    NATIVE_RUNTIME_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Puts the named environment variables back when dropped, including while a
/// failing test unwinds, so one failure cannot leak into the tests after it.
#[cfg(test)]
pub(super) struct EnvRestore(Vec<(String, Option<std::ffi::OsString>)>);

#[cfg(test)]
impl EnvRestore {
    pub(super) fn capture(names: &[&str]) -> Self {
        Self(
            names
                .iter()
                .map(|name| (name.to_string(), std::env::var_os(name)))
                .collect(),
        )
    }
}

#[cfg(test)]
impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (name, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(&name, value),
                None => std::env::remove_var(&name),
            }
        }
    }
}

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

/// Formats normal exit codes and, on Unix, the numeric terminating signal.
/// `ExitStatus::code()` is None for a signalled child, which used to erase the
/// only useful evidence in sidecar failures such as a native Python abort.
fn describe_exit_status(status: &ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    "unknown termination".to_string()
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
    // 2. the packaged `perception/` payload for the running exe, on every
    //    package layout (see `packaged_resource_dirs_for_exe`). The desktop shell
    //    also names it in SHELLX_CUT_SIDECAR_DIR, but a hand-started engine (an
    //    agent running `/usr/bin/cutd serve`, `...\cutd.exe`, `Contents/MacOS/cutd`)
    //    must find it without that variable.
    if let Some(exe) = current_exe {
        dirs.extend(packaged_payload_dirs_for_exe(exe));
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

/// Where a packaged app keeps the resource folder `name` (`perception`,
/// `agent-docs`, `ui-dist`) relative to the engine executable, one entry per
/// package layout. The ONE rule for every packaged resource the engine reads:
///   - Windows NSIS folder: `<install>\\cutd.exe` + `<install>\\<name>`
///   - macOS bundle: `Contents/MacOS/cutd` + `Contents/Resources/<name>`
///   - Linux DEB/RPM: `/usr/bin/cutd` + `/usr/lib/ShellX Cut/<name>`
/// Derived from the executable's own location only; an environment variable can
/// still override a caller's result, but is never needed.
pub fn packaged_resource_dirs_for_exe(exe: &Path, name: &str) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut dirs = vec![dir.join(name)];
    if let Some(prefix) = dir.parent() {
        dirs.push(prefix.join("Resources").join(name));
        dirs.push(prefix.join("lib").join("ShellX Cut").join(name));
    }
    dirs
}

/// The packaged resource folder `name` of the engine at `exe`: the first layout
/// entry that holds `marker`. `None` for a source build, a test binary or a
/// copied engine with no payload.
pub fn packaged_resource_dir_for_exe(exe: &Path, name: &str, marker: &str) -> Option<PathBuf> {
    packaged_resource_dirs_for_exe(exe, name)
        .into_iter()
        .find(|dir| dir.join(marker).is_file())
}

/// The packaged `perception` payload folders (see above).
fn packaged_payload_dirs_for_exe(exe: &Path) -> Vec<PathBuf> {
    packaged_resource_dirs_for_exe(exe, "perception")
}

/// Is the engine at `exe` the installed app's? The fact it knows about itself:
/// the packaged `perception` payload (instruments.py) sits where the package
/// layouts above put it relative to the executable. It is derived from the
/// executable's own location, never from an environment variable (which a
/// broken or hostile setup controls). A source build or a test binary has no
/// such payload. An installed app never runs a Python from PATH: it runs the one
/// Install captions provisions.
fn is_installed_layout(exe: &Path) -> bool {
    !is_cargo_build_exe(exe)
        && packaged_payload_dirs_for_exe(exe)
            .iter()
            .any(|dir| dir.join("instruments.py").is_file())
}

/// First line cargo writes into the `CACHEDIR.TAG` of every target folder.
const CARGO_CACHEDIR_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// Is `exe` a binary of a cargo build, `<target>/{debug,release}/cutd` or
/// `<target>/<triple>/{debug,release}/cutd`? Tauri copies the whole resource map
/// (including `perception/instruments.py`) into the target folder next to the
/// engine when the desktop crate shares its target folder, which would make a
/// developer's source-built engine look installed. Both facts must hold: the
/// executable sits in a folder named `debug` or `release` AND the target root
/// above it carries cargo's own `CACHEDIR.TAG`. A real install keeps `cutd` in a
/// folder of its own (`/usr/bin`, `<install>\\`, `Contents/MacOS`), so planting a
/// tag there changes nothing; making an installed engine count as a source
/// build would need the executable itself to live under a `debug`/`release`
/// folder whose parent a writer has tagged, and that writer could replace the
/// executable anyway.
fn is_cargo_build_exe(exe: &Path) -> bool {
    let is_profile = |dir: &Path| {
        matches!(
            dir.file_name().and_then(|name| name.to_str()),
            Some("debug" | "release")
        )
    };
    let Some(profile_dir) = exe.parent().filter(|dir| is_profile(dir)) else {
        return false;
    };
    let Some(above) = profile_dir.parent() else {
        return false;
    };
    // `--target <triple>` adds one folder between the target root and the profile.
    [Some(above), above.parent()]
        .into_iter()
        .flatten()
        .any(|root| {
            std::fs::read(root.join("CACHEDIR.TAG"))
                .map(|bytes| bytes.starts_with(CARGO_CACHEDIR_SIGNATURE.as_bytes()))
                .unwrap_or(false)
        })
}

fn installed_payload_beside_exe() -> bool {
    std::env::current_exe()
        .ok()
        .is_some_and(|exe| is_installed_layout(&exe))
}

/// The last rung. Installed: the app's own (possibly not yet provisioned)
/// environment, so a missing one is the plain "captions are not installed"
/// refusal. Source build or test run: a Python on PATH that really runs, else
/// the same managed path.
fn fallback_python_for_runtime(installed: bool, appdata_dir: Option<PathBuf>) -> PathBuf {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    fallback_python_with_path(installed, appdata_dir, &path_var)
}

/// [`fallback_python_for_runtime`] over an explicit PATH value.
fn fallback_python_with_path(
    installed: bool,
    appdata_dir: Option<PathBuf>,
    path_var: &std::ffi::OsStr,
) -> PathBuf {
    let managed = || {
        appdata_dir
            .map(|d| venv_python(&d))
            // Absolute and absent: never a bare name a relative PATH entry
            // (the current folder) could answer.
            .unwrap_or_else(|| {
                cut_media::toolpath::not_installed_program("shellx-cut-python-not-configured")
            })
    };
    if installed {
        return managed();
    }
    real_python::real_python_in(path_var).unwrap_or_else(managed)
}

/// Resolve the sidecar entrypoint `(python, instruments.py)` for the CURRENT
/// install layout (fixes the cold-install bug where the old code trusted
/// the build box's `CARGO_MANIFEST_DIR` venv path, which does not exist on an
/// installed app and made the sidecar unfindable).
///
/// Resolution (highest specificity wins):
///   python: SHELLX_CUT_PYTHON override → the `.venv` of the first base dir that
///           has one → (source builds and tests only) a Python on PATH that
///           really runs (see real_python.rs) → the expected app-data venv path,
///           so a missing sidecar is the plain "captions are not installed"
///           refusal, never an OS installer dialog or the Microsoft Store shortcut.
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
        fallback_python_for_runtime(installed_payload_beside_exe(), appdata_sidecar_dir()),
        script,
    )
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
    run_instruments_with_owner(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        None,
        None,
        true,
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
    progress: Option<Arc<SidecarProgress>>,
) -> Result<PerceptionReport, CutError> {
    run_instruments_with_owner(
        media_path,
        receipts_dir,
        asset_id,
        asset_hash,
        set,
        model,
        progress,
        None,
        true,
    )
}

fn run_instruments_with_owner(
    media_path: &Path,
    receipts_dir: &Path,
    asset_id: &str,
    asset_hash: &str,
    set: InstrumentSet,
    model: Option<&str>,
    progress: Option<Arc<SidecarProgress>>,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
    persist: bool,
) -> Result<PerceptionReport, CutError> {
    // Resolve the context before considering a cached report. A supplied malformed
    // locator is authoritative and must not be hidden by a legacy cache hit.
    let runtime = sidecar_runtime_unchecked()?;
    let stt_selection = effective_stt_selection();
    let runtime_context = runtime_context_provenance(&runtime, set, &stt_selection)?;

    // ---- cache check -----------------------------------------------------
    if persist {
        if let Some(cached) = load_report(receipts_dir, asset_id)? {
            let covers = set
                .names()
                .iter()
                .all(|n| cached.instruments_run.iter().any(|r| r == n));
            if cached.asset_hash == asset_hash
                && covers
                && cache_matches_runtime_context(&cached, runtime_context.as_ref())
            {
                tracing::debug!(asset_id, "perception cache hit");
                return Ok(cached);
            }
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
    let stdout = spawn_sidecar_streaming_with_runtime(
        &request,
        progress,
        control,
        &runtime,
        &stt_selection,
    )?;

    // ---- validate --------------------------------------------------------
    let mut report: PerceptionReport = serde_json::from_str(stdout.trim()).map_err(|e| {
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
    report.runtime_context = runtime_context;

    // ---- persist ---------------------------------------------------------
    // ProjectStore creates receipts/ before any perception job starts. Do not
    // recreate it here: project.delete may remove the project while a sidecar
    // is still finishing, and a late create_dir_all would resurrect a deleted
    // .cutproj with only stale analysis files inside it.
    if persist {
        require_live_receipts_dir(receipts_dir)?;
        let path = receipts_dir.join(format!("{asset_id}.perception.json"));
        // Replaces the name: never writes through a link or shared file.
        cut_core::store::replace_project_file(
            &path,
            serde_json::to_string_pretty(&report)
                .map_err(CutError::from)?
                .as_bytes(),
        )?;
        cut_media::cache_trust::remember(&path);
        tracing::info!(asset_id, path = %path.display(), "perception report written");
    }
    Ok(report)
}

fn require_live_receipts_dir(receipts_dir: &Path) -> Result<(), CutError> {
    // Item 162: the folder itself, never followed: a link or junction put at
    // `receipts` after the project was opened would send the report write
    // (a temporary file plus a rename) into a folder outside the project.
    let plain = std::fs::symlink_metadata(receipts_dir).is_ok_and(|meta| {
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && !cut_core::reparse::names_another_entry(&meta)
    });
    if plain {
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
    spawn_sidecar_streaming(request, None, None)
}

/// Bounded variant of [`spawn_sidecar`]: drains stderr while forwarding
/// `PROGRESS <frac> <label>` lines live through `progress`. The shared owner
/// writes the JSON request off-thread, drains both pipes concurrently, and
/// closes/reaps its complete child tree before returning. On a non-zero exit it
/// lifts the sidecar's `{"error":{...}}` contract (or the stderr tail) into a
/// `CutError`.
fn spawn_sidecar_streaming(
    request: &serde_json::Value,
    progress: Option<Arc<SidecarProgress>>,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
) -> Result<String, CutError> {
    let runtime = sidecar_runtime()?;
    let stt_selection = effective_stt_selection();
    spawn_sidecar_streaming_with_runtime(request, progress, control, &runtime, &stt_selection)
}

fn spawn_sidecar_streaming_with_runtime(
    request: &serde_json::Value,
    progress: Option<Arc<SidecarProgress>>,
    control: Option<&cut_media::ffmpeg::OwnedProcessControl>,
    runtime: &SidecarRuntime,
    stt_selection: &EffectiveSttSelection,
) -> Result<String, CutError> {
    // Before anything else: no Python is "captions not installed", not a spawn error.
    runtime.ensure_runnable()?;
    let native_context = runtime.native_context.is_some();
    let python = &runtime.python;
    let script = &runtime.script;
    if !script.exists() {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "instruments.py not found",
            format!("expected at {}", script.display()),
        )
        .with_suggested_action(
            "The captions files are missing from this install: reinstall ShellX Cut, then choose Install captions",
        ));
    }
    if !native_context {
        match cleanup_stale_managed_venv_packages(python) {
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
    }
    let mut cmd = Command::new(python);
    apply_python_command_policy(&mut cmd, native_context);
    cmd.arg(script);
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
        apply_stt_env(
            &mut cmd,
            stt_selection.model.as_deref(),
            stt_selection.language.as_deref(),
        );
    }
    let suggested_action = captions_setup_action();
    let default_control;
    let control = match control {
        Some(control) => control,
        None => {
            default_control = cut_media::ffmpeg::OwnedProcessControl::bounded(
                Duration::from_secs(30 * 60),
                || false,
            )
            .with_output_cap(8 * 1024 * 1024);
            &default_control
        }
    };
    let stderr_line = progress.map(|callback| {
        Arc::new(move |line: &str| {
            if let Some((fraction, label)) = parse_progress(line) {
                callback(fraction, label);
            }
        }) as cut_media::ffmpeg::ProcessStderrLineObserver
    });
    let request = request.to_string();
    let output = match cut_media::ffmpeg::run_owned_command_with_input(
        &mut cmd,
        request.as_bytes(),
        control,
        "run perception sidecar",
        stderr_line,
    ) {
        Ok(output) => output,
        Err(error)
            if error.code == error_codes::RENDER_CANCELLED
                || error.message == "operation timed out" =>
        {
            return Err(error);
        }
        Err(error) => {
            return Err(CutError::new(
                error_codes::SIDECAR,
                "failed to run perception sidecar",
                format!("{}: {} ({})", python.display(), error.message, error.cause),
            )
            .with_suggested_action(suggested_action));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        // The Windows Store python shortcut is not a Python: say what is
        // missing, not its text.
        if !native_context
            && is_windows_store_shortcut_run(
                output.status.code(),
                &format!("{stdout}{}", String::from_utf8_lossy(&output.stderr)),
            )
        {
            return Err(captions_not_installed_error(
                &InterpreterProblem::WindowsStoreShortcut,
            ));
        }
        let cause = serde_json::from_str::<serde_json::Value>(stdout.trim())
            .ok()
            .and_then(|v| v.get("error").cloned())
            .map(|e| e.to_string())
            .unwrap_or_else(|| {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let tail: Vec<&str> = stderr.lines().rev().take(8).collect();
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            });
        return Err(CutError::new(
            error_codes::SIDECAR,
            format!(
                "perception sidecar failed ({})",
                describe_exit_status(&output.status)
            ),
            cause,
        )
        .with_suggested_action(
            "Install or repair the captions tools in Settings > AI & transcription, or call \
             system.setup_perception and wait for its job. Then retry.",
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
    owned::run_subject_with_owner(media_path, asset_hash, preset, direction, None)
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
    owned::build_contact_sheet_with_owner(media_path, out_dir, preset, None)
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
    owned::build_qc_sheet_with_owner(media_path, out_dir, preset, None)
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
    progress: Option<Arc<SidecarProgress>>,
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
    // Replaces the name: never writes through a link or shared file.
    cut_core::store::replace_project_file(
        &path,
        serde_json::to_string_pretty(&transcript)
            .map_err(CutError::from)?
            .as_bytes(),
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
/// Item 155: the one gate for every reader: a report this engine did not
/// write (it came with the project, or changed since) counts as not analysed
/// on this computer and is None, never data; callers then say "run
/// media.perception" or run the analysis as a job.
/// Validates the schema tag; a mismatched schema is an error, not a silent None.
/// The report must be an ordinary single-link file: a link, Windows junction
/// point or file shared with another path is refused without being opened
/// through, and the read stops at the report byte limit.
pub fn load_report(
    receipts_dir: &Path,
    asset_id: &str,
) -> Result<Option<PerceptionReport>, CutError> {
    use cut_core::store::{plain_file_state, read_bounded_cache, BoundedCacheRead, PlainFileState};
    let path = receipts_dir.join(format!("{asset_id}.perception.json"));
    let not_plain = || {
        CutError::new(
            error_codes::SIDECAR,
            "perception report is not a plain file",
            path.display().to_string(),
        )
    };
    match plain_file_state(&path) {
        PlainFileState::Missing => return Ok(None),
        PlainFileState::NotPlain => return Err(not_plain()),
        PlainFileState::Plain => {}
    }
    // Read once: the engine-origin check and the parse use the same bytes, so
    // a file replaced after the check cannot be the one parsed.
    let bytes = match read_bounded_cache(&path, MAX_PERCEPTION_REPORT_BYTES) {
        BoundedCacheRead::Bytes(bytes) => bytes,
        // Only an over-limit report this engine wrote is an error; any other
        // over-limit file is simply not a report made on this computer.
        BoundedCacheRead::Oversized if !cut_media::cache_trust::trusted(&path) => return Ok(None),
        BoundedCacheRead::Oversized => {
            return Err(CutError::new(
                error_codes::SIDECAR,
                "perception report exceeds its byte limit",
                path.display().to_string(),
            ))
        }
        BoundedCacheRead::Unavailable => return Err(not_plain()),
    };
    if !cut_media::cache_trust::trusted_bytes(&path, &bytes) {
        return Ok(None);
    }
    let report: PerceptionReport = serde_json::from_slice(&bytes).map_err(CutError::from)?;
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

    #[cfg(unix)]
    #[test]
    fn failed_analysis_keeps_cause_and_names_the_captions_repair_action() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("instruments.py");
        std::fs::write(
            &script,
            "import sys\nprint('{\"error\":\"missing captions dependency\"}')\nsys.exit(1)\n",
        )
        .unwrap();
        let runtime = SidecarRuntime {
            python: std::path::PathBuf::from("python3"),
            script,
            native_context: None,
        };
        let error = spawn_sidecar_streaming_with_runtime(
            &serde_json::json!({}),
            None,
            None,
            &runtime,
            &EffectiveSttSelection {
                model: None,
                language: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, error_codes::SIDECAR);
        assert!(
            error.cause.contains("missing captions dependency"),
            "{error:?}"
        );
        let action = error.suggested_action.as_deref().unwrap_or_default();
        assert!(action.contains("system.setup_perception"), "{error:?}");
        assert!(
            action.contains("Settings > AI & transcription"),
            "{error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sidecar_exit_status_keeps_a_normal_numeric_code() {
        let status = Command::new("sh")
            .args(["-c", "exit 23"])
            .status()
            .expect("sh must start for status formatting");
        assert_eq!(describe_exit_status(&status), "exit 23");
    }

    #[cfg(unix)]
    #[test]
    fn sidecar_exit_status_keeps_the_unix_signal_number() {
        use std::os::unix::process::ExitStatusExt;

        let status = Command::new("sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .expect("sh must start for signal status formatting");
        let signal = status.signal().expect("child must terminate by signal");
        assert_eq!(describe_exit_status(&status), format!("signal {signal}"));
    }

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
        let _guard = lock_native_runtime_env();
        let _restore = EnvRestore::capture(&[ENV_PYTHON]);
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
        assert_eq!(configured_sidecar_python().unwrap(), Some(explicit));
    }

    // Builds a UNIX venv layout (`.venv/bin/python`) to exercise the macOS
    // stub-skipping logic. On Windows the resolver correctly looks for
    // `.venv/Scripts/python.exe` instead, finds nothing, and returns None — so
    // the fixture, not the logic, is what fails there. Gated to unix, where the
    // layout it writes is the real one.
    #[cfg(unix)]
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
            cut_media::toolpath::not_installed_program("shellx-cut-python-not-configured")
        );
        // A source build or test run: a Python on PATH that really runs, else the
        // app's own (not yet provisioned) environment; never an unverified command.
        let fallback = fallback_python_for_runtime(false, Some(appdata.clone()));
        assert!(
            fallback == venv_python(&appdata) || real_python::runs_as_python(&fallback),
            "{}",
            fallback.display()
        );
    }

    /// The last rung of the ladder is a Python that runs, never a command that
    /// only has the right name (the Windows Store shortcut, Apple's stub).
    #[cfg(unix)]
    #[test]
    fn path_python_is_used_only_when_it_really_runs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let appdata = dir.path().join("appdata");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let python = bin.join("python3");
        let write = |body: &str| {
            std::fs::write(&python, body).unwrap();
            std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        // A shortcut that answers with a hint and exit 9009 is not a Python.
        write("#!/bin/sh\necho 'Python was not found'\nexit 9009\n");
        assert_eq!(
            fallback_python_with_path(false, Some(appdata.clone()), bin.as_os_str()),
            venv_python(&appdata),
            "a stub on PATH must not be used"
        );
        // The same name answering like Python is used (a new PATH value, so the
        // per-PATH cache is not reused).
        write("#!/bin/sh\necho '(3, 12)'\n");
        let bin2 = dir.path().join("bin2");
        std::fs::create_dir_all(&bin2).unwrap();
        std::fs::copy(&python, bin2.join("python3")).unwrap();
        assert_eq!(
            fallback_python_with_path(false, Some(appdata.clone()), bin2.as_os_str()),
            bin2.join("python3")
        );
        // An installed app never takes a Python from PATH.
        assert_eq!(
            fallback_python_with_path(true, Some(appdata.clone()), bin2.as_os_str()),
            venv_python(&appdata)
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
            runtime_context: None,
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
    fn legacy_cache_hit_skips_python() {
        let _guard = lock_native_runtime_env();
        let _restore = EnvRestore::capture(&[cut_native_runtime_context::CONTEXT_ENV]);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        let dir = tempfile::tempdir().unwrap();
        let r = report(&["words", "silence", "scenes", "beats", "loudness"]);
        std::fs::write(
            dir.path().join("a1.perception.json"),
            serde_json::to_string(&r).unwrap(),
        )
        .unwrap();
        // Stands in for this engine's own write (item 155).
        cut_media::cache_trust::remember(&dir.path().join("a1.perception.json"));
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
        assert!(got.runtime_context.is_none());
    }

    #[test]
    fn invalid_prepared_context_rejects_matching_legacy_cache() {
        let _guard = lock_native_runtime_env();
        let _restore = EnvRestore::capture(&[cut_native_runtime_context::CONTEXT_ENV]);
        let dir = tempfile::tempdir().unwrap();
        let r = report(&["words", "silence", "scenes", "beats", "loudness"]);
        std::fs::write(
            dir.path().join("a1.perception.json"),
            serde_json::to_string(&r).unwrap(),
        )
        .unwrap();
        let invalid = dir.path().join("invalid-context.json");
        std::fs::write(&invalid, b"not json").unwrap();
        std::env::set_var(cut_native_runtime_context::CONTEXT_ENV, invalid);

        let error = run_instruments(
            Path::new("/definitely/missing.mp4"),
            dir.path(),
            "a1",
            "sha256:cafe",
            InstrumentSet::Full,
            None,
        )
        .expect_err("a supplied invalid context must reject before cache reuse");
        assert!(error.message.contains("prepared native runtime rejected"));
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
        let _guard = lock_native_runtime_env();
        let _restore = EnvRestore::capture(&[cut_native_runtime_context::CONTEXT_ENV]);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
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
        // Stands in for this engine's own write (item 155).
        cut_media::cache_trust::remember(&dir.path().join("a1.perception.json"));
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
    /// Item 155: a report that came with the project (its asset_hash copied
    /// from project.json) is not a cache hit: the run goes to the sidecar,
    /// which here fails on the missing media, proving the plant was not used.
    #[test]
    fn planted_report_is_not_a_cache_hit() {
        let _guard = lock_native_runtime_env();
        let _restore = EnvRestore::capture(&[cut_native_runtime_context::CONTEXT_ENV]);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
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
        );
        assert!(got.is_err(), "a planted report must not be reused");
        // The single gate every reader goes through: not analysed here = None.
        assert!(load_report(dir.path(), "a1").unwrap().is_none());
    }

    /// Item 162: a receipts folder that became a link is not written through.
    #[cfg(unix)]
    #[test]
    fn linked_receipts_folder_is_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let receipts = dir.path().join("receipts");
        std::os::unix::fs::symlink(&outside, &receipts).unwrap();
        assert!(require_live_receipts_dir(&receipts).is_err());
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert!(require_live_receipts_dir(&plain).is_ok());
    }

    /// The report checked as this engine's is the report parsed: a file
    /// swapped in between the two is never returned. A second thread keeps
    /// replacing the recorded report with another one; every load returns the
    /// recorded report or nothing.
    #[test]
    fn a_report_replaced_after_its_check_is_never_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a1.perception.json");
        let recorded = serde_json::to_vec(&report(&["words"])).unwrap();
        let mut planted_report = report(&["words"]);
        planted_report.source_path = "/planted.mp4".into();
        let planted = serde_json::to_vec(&planted_report).unwrap();
        std::fs::write(&path, &recorded).unwrap();
        cut_media::cache_trust::remember(&path);

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let swapper = {
            let (stop, dir, path) = (stop.clone(), dir.path().to_path_buf(), path.clone());
            let (recorded, planted) = (recorded.clone(), planted.clone());
            std::thread::spawn(move || {
                let staging = dir.join("staging.json");
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for body in [&planted, &recorded] {
                        // A failed replace (a reader holds the file on
                        // Windows) only means fewer swaps.
                        let _ = std::fs::write(&staging, body);
                        let _ = std::fs::rename(&staging, &path);
                    }
                }
            })
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut loaded = 0;
        let mut planted_seen = 0;
        while std::time::Instant::now() < deadline {
            if let Ok(Some(got)) = load_report(dir.path(), "a1") {
                loaded += 1;
                if got.source_path == "/planted.mp4" {
                    planted_seen += 1;
                }
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        swapper.join().unwrap();
        assert_eq!(
            planted_seen, 0,
            "{planted_seen} of {loaded} loads returned a report this engine did not write"
        );
    }

    #[test]
    fn load_report_rejects_wrong_schema() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a1.perception.json"),
            r#"{"schema":"other/9","asset_hash":"x","source_path":"y"}"#,
        )
        .unwrap();
        cut_media::cache_trust::remember(&dir.path().join("a1.perception.json"));
        assert!(load_report(dir.path(), "a1").is_err());
    }

    #[test]
    fn load_report_rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(dir.path().join("a1.perception.json")).unwrap();
        file.set_len(MAX_PERCEPTION_REPORT_BYTES + 1).unwrap();
        cut_media::cache_trust::remember(&dir.path().join("a1.perception.json"));
        assert!(load_report(dir.path(), "a1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn load_report_rejects_symlink_leaf() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.json");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("a1.perception.json")).unwrap();
        assert!(load_report(dir.path(), "a1").is_err());
    }
}
