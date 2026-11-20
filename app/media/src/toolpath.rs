//! toolpath.rs — single source of truth for "where do the external tools live".
//!
//! ROLE
//!   ShellX Cut shells out to two heavy, non-bundled-in-our-installer tools:
//!     * ffmpeg / ffprobe — media probe / proxy / render / frame grabs
//!     * python (+ instruments.py venv) — the perception sidecar
//!   On the DEV box both are on PATH (or in a compile-time venv). On a COLD
//!   Windows notebook neither is, and the
//!   old code assumed PATH / a `CARGO_MANIFEST_DIR` venv path that does not
//!   exist on an installed app. This module centralizes resolution so every
//!   caller — the Rust media engine, the perception sidecar, and the python
//!   script via an env var — agree on ONE ffmpeg.
//!
//! RESOLUTION ORDER (highest specificity wins; PATH is the LAST resort, never
//! the assumption documented in docs/public/BUILDING.md):
//!   1. explicit env override   — SHELLX_CUT_FFMPEG / _FFPROBE (a full path),
//!                                or SHELLX_CUT_FFMPEG_DIR (a directory holding
//!                                ffmpeg[.exe] + ffprobe[.exe]).
//!   2. a tool dir beside the exe — installed layout: `ffmpeg/` next to
//!                                cutd.exe (future "bundle in installer" route,
//!                                already wired so it costs nothing to flip).
//!   3. the app-data tools dir   — where the first-run bootstrap DOWNLOADS
//!                                ffmpeg on a consented cold-install (the
//!                                chosen route keeps our license open by
//!                                never shipping GPL libx264 in our artifact).
//!   4. system PATH              — the first ABSOLUTE PATH entry holding it,
//!                                started by its full path (relative and empty
//!                                PATH entries are never searched).
//!
//! WHY THIS SEAM (the load-bearing design point)
//!   All three packaging futures cost ~nothing because each just populates a
//!   different rung: download-to-appdata (rung 3), bundle-in-installer (rung 2),
//!   dev-PATH (rung 4). We are NOT architected against bundling — flipping to a
//!   bundled ffmpeg later is one installer line, no code change here.
//!
//! HONESTY CONTRACT
//!   A tool is never started by its bare name: a bare name would also search
//!   relative PATH entries, i.e. the current folder, which may be a received
//!   project. When no rung finds it, resolution returns the absolute place the
//!   download would install it (program_path::not_installed); starting that
//!   fails as "not found" and the engine's existing "not installed" refusal
//!   follows. `doctor()` lets callers report the resolved path
//!   (or "not found") up-front — the desktop bootstrap state and the
//!   `tools.doctor` verb use it so a cold notebook is told EXACTLY what is
//!   missing and how to fix it, never a silent failure.
//!
//! Dependencies: std only. Callers: ffmpeg.rs (ffmpeg_bin/ffprobe_bin delegate
//! here), cut-perception sidecar (python + ffmpeg-dir env injection), cutd
//! `tools.doctor` verb, the desktop shell (sets the env overrides on the
//! spawned engine).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod cache;
mod program_path;
mod selection;
pub(crate) use cache::ToolCache;
pub use cache::{generation as tools_generation, refresh as refresh_media_tools};
pub use program_path::{
    absolute_entries, absolute_path_value, child_path_value, find_on_path,
    harden_process_search_path, not_installed as not_installed_program,
};
pub use selection::{
    automatic_pick, automatic_pick_for, ffmpeg_selection, is_cargo_build_exe, AutomaticPick,
    FfmpegSelection, SelectionReason,
};

/// Env var: explicit full path to the ffmpeg executable (highest precedence).
pub const ENV_FFMPEG: &str = "SHELLX_CUT_FFMPEG";
/// Env var: explicit full path to the ffprobe executable.
pub const ENV_FFPROBE: &str = "SHELLX_CUT_FFPROBE";
/// Env var: a directory containing ffmpeg[.exe] + ffprobe[.exe]. Used both as a
/// resolution rung AND as the value the perception sidecar prepends to PATH so
/// `instruments.py`'s bare `ffmpeg` calls hit the SAME binary as Rust.
pub const ENV_FFMPEG_DIR: &str = "SHELLX_CUT_FFMPEG_DIR";
/// Env var: turns the engine's AUTOMATIC pick of the best HARDWARE-capable
/// installed ffmpeg on (`1`) or off (`0`). Unset, the pick is ON for every
/// engine except a source build in a cargo target folder (tests and developer
/// builds stay on the deterministic ladder) — the same whether the desktop
/// shell or a person/agent started the engine (see `selection.rs`). An explicit
/// `SHELLX_CUT_FFMPEG` or the person's choice always wins; software-only
/// machines keep the ladder's build (auto only ever switches UP to hardware).
pub const ENV_FFMPEG_AUTO: &str = "SHELLX_CUT_FFMPEG_AUTO";

/// Platform executable name for a tool stem ("ffmpeg" → "ffmpeg.exe" on win).
fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// The per-user app-data tools dir where the first-run bootstrap downloads
/// ffmpeg (and, later, the python env). Mirrors where the desktop shell writes:
///   Windows: %LOCALAPPDATA%\ShellX Cut\tools
///   macOS:   ~/Library/Application Support/ShellX Cut/tools
///   Linux:   $XDG_DATA_HOME/shellx-cut/tools  (or ~/.local/share/...)
/// Returns None when no home/appdata var is set (then this rung is simply
/// skipped — resolution falls through to PATH). A relative value is not a
/// location (it would be read against the current folder) and counts as unset.
pub fn appdata_tools_dir() -> Option<PathBuf> {
    let absolute = |name: &str| program_path::absolute_setting(std::env::var_os(name));
    if cfg!(windows) {
        absolute("LOCALAPPDATA").map(|p| p.join("ShellX Cut").join("tools"))
    } else if cfg!(target_os = "macos") {
        absolute("HOME").map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("ShellX Cut")
                .join("tools")
        })
    } else {
        absolute("XDG_DATA_HOME")
            .or_else(|| absolute("HOME").map(|h| h.join(".local/share")))
            .map(|base| base.join("shellx-cut").join("tools"))
    }
}

/// The program an explicit per-tool variable names: None when unset; the path
/// when absolute; the absolute not-installed place when relative (a relative
/// value would be read against the current folder, so it is never started).
fn explicit_program(env: &str, stem: &str) -> Option<OsString> {
    let value = std::env::var_os(env).filter(|v| !v.is_empty())?;
    Some(
        program_path::absolute_setting(Some(value))
            .unwrap_or_else(|| program_path::not_installed(stem))
            .into_os_string(),
    )
}

// ── Manual ffmpeg override (the UI "Change ffmpeg" control) ────────────────────
//
// The user can pick a specific ffmpeg; it is persisted to a tiny file under
// SHELLX_CUT_HOME/preferences when set, otherwise beside the app-data tools
// dir (survives app-bundle reinstalls). The value in use is cached together
// with the HW-capability caches (hwencode::hw_caps / gpu_filters_available) in
// one tools generation, so they stay CONSISTENT with the binary in use; a
// doctor re-scan or system.set_ffmpeg calls [`refresh_media_tools`], which moves
// the generation when the choice or the installed tools changed. No restart is
// needed. An explicit `SHELLX_CUT_FFMPEG` env still wins over this.

/// The persisted-override file (one line: the chosen ffmpeg's absolute path).
/// `<SHELLX_CUT_HOME>/preferences/ffmpeg-override` when configured, otherwise
/// `<app-data>/ShellX Cut/ffmpeg-override`. None when neither root exists.
fn override_file() -> Option<PathBuf> {
    override_file_with(
        std::env::var_os("SHELLX_CUT_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        appdata_tools_dir(),
    )
}

fn override_file_with(
    shellx_cut_home: Option<PathBuf>,
    appdata_tools: Option<PathBuf>,
) -> Option<PathBuf> {
    shellx_cut_home
        .map(|root| root.join("preferences").join("ffmpeg-override"))
        .or_else(|| {
            appdata_tools.and_then(|tools| tools.parent().map(|root| root.join("ffmpeg-override")))
        })
}

/// The persisted manual ffmpeg choice read FRESH from disk (trimmed; None when
/// unset/empty/unreadable). The doctor + `system.set_ffmpeg` report this so the
/// UI shows the current setting; resolution uses the generation-cached
/// [`ffmpeg_override`].
pub fn read_override_setting() -> Option<String> {
    let s = std::fs::read_to_string(override_file()?).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Persist (`Some`) or clear (`None`/empty) the manual ffmpeg override. Takes
/// effect at the next [`refresh_media_tools`] (system.set_ffmpeg re-scans right
/// away). The caller validates that the path is a runnable ffmpeg first.
pub fn write_override_setting(path: Option<&str>) -> std::io::Result<()> {
    let Some(p) = override_file() else {
        return Ok(()); // no app-data dir (unusual) — nothing to persist
    };
    match path {
        Some(v) if !v.trim().is_empty() => {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&p, v.trim())
        }
        _ => match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()), // already absent = already "no override"
        },
    }
}

/// The manual override in effect for the current tools generation.
/// A stale path that no longer points at a file is IGNORED (so a deleted ffmpeg
/// can't break every op — resolution falls through to auto/ladder).
fn ffmpeg_override() -> Option<OsString> {
    static OVERRIDE: ToolCache<Option<OsString>> = ToolCache::new();
    OVERRIDE.get_or_init(|| {
        let s = read_override_setting()?;
        let p = PathBuf::from(&s);
        (p.is_absolute() && p.is_file()).then(|| OsString::from(s))
    })
}

/// The directory of the currently-running executable (cutd.exe in the installed
/// app). Resolution rung 2 ("beside the exe") looks for a `ffmpeg/` subdir here.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Candidate directories that may hold ffmpeg/ffprobe, in precedence order
/// (excluding the explicit per-exe env overrides, handled separately). Each is
/// probed for the platform exe name; the first hit wins.
fn ffmpeg_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // rung 1b: SHELLX_CUT_FFMPEG_DIR as a directory.
    if let Some(d) = std::env::var_os(ENV_FFMPEG_DIR) {
        let d = PathBuf::from(d);
        if !d.as_os_str().is_empty() {
            dirs.push(d.clone());
            // BtbN zips extract to a `bin/` subfolder — accept that shape too.
            dirs.push(d.join("bin"));
        }
    }
    // rung 2: a `ffmpeg/` (and ffmpeg/bin) folder beside the running exe.
    if let Some(dir) = exe_dir() {
        dirs.push(dir.join("ffmpeg"));
        dirs.push(dir.join("ffmpeg").join("bin"));
    }
    // rung 3: the app-data tools dir (where bootstrap downloads land).
    if let Some(tools) = appdata_tools_dir() {
        dirs.push(tools.join("ffmpeg"));
        dirs.push(tools.join("ffmpeg").join("bin"));
    }
    // rung 3b (macOS): Homebrew / common system locations. A GUI .app launched
    // from Finder gets a STRIPPED PATH (/usr/bin:/bin:/usr/sbin:/sbin) — it does
    // NOT inherit the shell's PATH, so a `brew install ffmpeg` (the standard Mac
    // way) lands in /opt/homebrew/bin (Apple Silicon) or /usr/local/bin (Intel)
    // and rung-4 PATH lookup would MISS it. Check those explicitly so the
    // brew-installed binary is found without the user fiddling with PATH.
    dirs.extend(macos_homebrew_dirs());
    // Only absolute folders: a relative SHELLX_CUT_FFMPEG_DIR would be read
    // against the current folder.
    dirs.retain(|dir| dir.is_absolute());
    dirs
}

/// Homebrew locations checked on macOS, most capable first. `ffmpeg-full` is
/// keg-only: Homebrew installs it under `opt/ffmpeg-full/bin` and does not link
/// it into `bin`, so the recommended install would otherwise never be found.
/// It comes before the linked plain `ffmpeg`, which lacks libass, vid.stab and
/// zimg (the caption, stabilize and color-managed render filters).
pub fn macos_homebrew_dirs() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    [
        "/opt/homebrew/opt/ffmpeg-full/bin",
        "/usr/local/opt/ffmpeg-full/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

/// Resolve a tool ("ffmpeg" or "ffprobe") to a runnable program path.
/// `explicit_env` is the per-exe full-path override (rung 1a). Returns a full
/// path when found at rungs 1–3, else the BARE name (rung 4 = PATH lookup).
fn resolve_tool(stem: &str, explicit_env: &str) -> OsString {
    // rung 1a: explicit full path to THIS exe (absolute only).
    if let Some(p) = explicit_program(explicit_env, stem) {
        return p;
    }
    if let Some(found) = program_path::find_in(&ffmpeg_search_dirs(), stem) {
        return found.into_os_string();
    }
    // rung 4: the first ABSOLUTE PATH entry that holds it, by its full path, so
    // the program started is the one the gate admitted. Never the bare name: a
    // bare name would also search relative PATH entries (the current folder).
    program_path::find_on_path(stem)
        .unwrap_or_else(|| program_path::not_installed(stem))
        .into_os_string()
}

/// Program path for ffmpeg: always absolute (see the honesty contract above).
/// `Command::new` takes `AsRef<OsStr>`, so callers pass this straight through.
///
/// Resolution precedence:
///   1. explicit `SHELLX_CUT_FFMPEG` env override — always wins (power users / CI);
///   2. the persisted MANUAL choice (the UI "Change ffmpeg" control, system.set_ffmpeg);
///   3. AUTO (the default; see [`automatic_pick`]) — the best HARDWARE-capable
///      installed ffmpeg, so GPU "just works" with no user action;
///   4. the resolution ladder (bundled / beside-exe / app-data / PATH).
/// Auto never disturbs a software-only machine (it only switches UP to a hardware
/// build) and neither auto nor a manual choice overrides the env.
pub fn ffmpeg() -> OsString {
    choose_ffmpeg().0
}

/// The one choice behind [`ffmpeg`] and [`ffmpeg_selection`], with the step
/// that decided it, so the doctor's "why" can never disagree with the build
/// actually used.
fn choose_ffmpeg() -> (OsString, selection::Step) {
    use selection::Step;
    let has_env = std::env::var_os(ENV_FFMPEG).is_some_and(|p| !p.is_empty());
    if has_env {
        return (resolve_tool("ffmpeg", ENV_FFMPEG), Step::Environment);
    }
    // 2. the user's persisted manual choice wins over auto + the ladder.
    if let Some(p) = ffmpeg_override() {
        return (p, Step::Chosen);
    }
    // 3. auto best-hardware.
    if auto_enabled() {
        if let Some(best) = auto_best_ffmpeg() {
            return (best, Step::AutomaticHardware);
        }
    }
    (resolve_tool("ffmpeg", ENV_FFMPEG), Step::SearchOrder)
}

/// True when the automatic pick is on for this engine ([`automatic_pick`]).
fn auto_enabled() -> bool {
    automatic_pick().is_on()
}

/// The best HARDWARE-capable installed ffmpeg, or None when no installed ffmpeg
/// has hardware acceleration (then we stay on the deterministic ladder — software
/// renders are unchanged). Probed ONCE and cached for the process lifetime.
///
/// Recursion-safe: `ffmpeg_candidates()` resolves via the LADDER (not this
/// auto-aware path) and `probe_ffmpeg_caps` runs each binary by EXPLICIT path
/// (never `ffmpeg()`), so initialising this cache cannot re-enter `ffmpeg()`.
fn auto_best_ffmpeg() -> Option<OsString> {
    static BEST: ToolCache<Option<OsString>> = ToolCache::new();
    BEST.get_or_init(|| {
        let best = all_candidate_caps().iter().max_by_key(|c| c.rank())?;
        // Only auto-switch to a HARDWARE-capable build; software-only stays on the
        // ladder default so deterministic software renders never silently change.
        best.hw.any().then(|| OsString::from(best.path.clone()))
    })
}

/// A render-feature an ffmpeg binary may or may not provide. Caption burn-in needs
/// libass; stabilize needs libvidstab. A build can be the FASTEST (hardware) yet
/// lack these — Homebrew dropped libass from plain ffmpeg 8.x — so the render-time
/// selector picks per-feature, not purely by encode speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegFeature {
    /// libass-backed `subtitles`/`ass` filter — caption burn-in.
    Libass,
    /// `vidstabtransform`/`vidstabdetect` — edit.stabilize.
    Vidstab,
    /// `zscale` (libzimg) — color-managed renders. A project working/output space
    /// (or a tagged clip input) other than rec709 emits a `zscale` colorspace hop
    /// (render::colorspace_filter); a build without libzimg fails that render exit-8
    /// with "No such filter: 'zscale'". Homebrew's plain ffmpeg 8.x dropped libzimg,
    /// so a HW-capable build can lack it — same per-feature selection concern as
    /// libass: the render-time selector picks a zscale-capable build for a
    /// color-managed render even when a faster build lacks zscale.
    Zscale,
}

/// Every discoverable ffmpeg's capabilities, PROBED ONCE per tools generation
/// (the probe runs test encodes — expensive). Shared by auto-best-HW and the
/// feature-aware selector so each binary is probed at most once per generation.
/// The list is leaked per generation; the generation only moves when the
/// installed tools change, so this is a handful of small lists per process.
fn all_candidate_caps() -> &'static [crate::hwencode::FfmpegCaps] {
    static CAPS: ToolCache<&'static [crate::hwencode::FfmpegCaps]> = ToolCache::new();
    CAPS.get_or_init(|| {
        let caps: Vec<_> = ffmpeg_candidates()
            .iter()
            .map(|p| crate::hwencode::probe_ffmpeg_caps(p))
            .collect();
        Box::leak(caps.into_boxed_slice())
    })
}

/// True when `caps` provides render-feature `f`.
fn caps_has(caps: &crate::hwencode::FfmpegCaps, f: FfmpegFeature) -> bool {
    match f {
        FfmpegFeature::Libass => caps.libass,
        FfmpegFeature::Vidstab => caps.vidstab,
        FfmpegFeature::Zscale => caps.zscale,
    }
}

/// ffmpeg path for a render that REQUIRES `features` (caption burn-in ⇒ Libass,
/// stabilize ⇒ Vidstab, color-managed ⇒ Zscale). Among discoverable binaries it
/// picks the highest-acceleration build that provides ALL required features — so
/// captions/stabilize never silently fail just because the FASTEST build dropped
/// a library, while still preferring a hardware build WHEN it also has the
/// feature.
///
/// Precedence: an explicit `SHELLX_CUT_FFMPEG` env is an exact runtime pin and
/// always wins (the media gate refuses up front when it lacks a feature). The
/// persisted manual choice wins whenever it has the features; when it lacks one,
/// an installed build that has it serves this render only, so a chosen build
/// without vid.stab does not make Stabilize fail while a capable build exists.
/// Falls back to [`ffmpeg`] when no discoverable binary has the features.
pub fn ffmpeg_for(features: &[FfmpegFeature]) -> OsString {
    if std::env::var_os(ENV_FFMPEG).is_some_and(|p| !p.is_empty()) {
        return resolve_tool("ffmpeg", ENV_FFMPEG);
    }
    if features.is_empty() {
        return ffmpeg();
    }
    if let Some(chosen) = ffmpeg_override() {
        if build_has(&chosen, features) {
            return chosen;
        }
    }
    let best = all_candidate_caps()
        .iter()
        .filter(|c| c.version.is_some() && features.iter().all(|f| caps_has(c, *f)))
        .max_by_key(|c| c.rank());
    match best {
        Some(c) => OsString::from(c.path.clone()),
        // No feature-capable build anywhere — the normal pick (the gate refuses first).
        None => ffmpeg(),
    }
}

/// Whether the ffmpeg at `program` lists every filter `features` need, from
/// one `-filters` run per build per tools generation.
fn build_has(program: &std::ffi::OsStr, features: &[FfmpegFeature]) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    type Filters = (bool, bool, bool);
    static SEEN: Mutex<Option<(u64, HashMap<OsString, Filters>)>> = Mutex::new(None);
    let generation = tools_generation();
    let cached = {
        let seen = SEEN.lock().unwrap_or_else(|p| p.into_inner());
        seen.as_ref()
            .filter(|(g, _)| *g == generation)
            .and_then(|(_, map)| map.get(program).copied())
    };
    let (libass, vidstab, zscale) = cached.unwrap_or_else(|| {
        let probed = crate::hwencode::feature_filters_at(program);
        let mut seen = SEEN.lock().unwrap_or_else(|p| p.into_inner());
        if seen.as_ref().is_none_or(|(g, _)| *g != generation) {
            *seen = Some((generation, HashMap::new()));
        }
        if let Some((_, map)) = seen.as_mut() {
            map.insert(program.to_os_string(), probed);
        }
        probed
    });
    features.iter().all(|f| match f {
        FfmpegFeature::Libass => libass,
        FfmpegFeature::Vidstab => vidstab,
        FfmpegFeature::Zscale => zscale,
    })
}

/// The `features` the ffmpeg Cut would use for them cannot provide (each is
/// resolved through [`ffmpeg_for`] as a render would). Empty when every one is
/// available. Callers check that ffmpeg exists at all first.
pub fn missing_features(features: &[FfmpegFeature]) -> Vec<FfmpegFeature> {
    features
        .iter()
        .copied()
        .filter(|f| !build_has(&ffmpeg_for(&[*f]), &[*f]))
        .collect()
}

/// Program path for ffprobe: always absolute (see the honesty contract above).
///
/// An explicit `SHELLX_CUT_FFPROBE` wins. Otherwise a deliberately chosen
/// ffmpeg (`SHELLX_CUT_FFMPEG` or the persisted manual choice) brings the
/// ffprobe beside it, so "choose your own ffmpeg" also supplies media
/// inspection; the ladder is the fallback.
pub fn ffprobe() -> OsString {
    if let Some(p) = explicit_program(ENV_FFPROBE, "ffprobe") {
        return p;
    }
    let chosen = explicit_program(ENV_FFMPEG, "ffmpeg").or_else(ffmpeg_override);
    if let Some(sibling) = chosen.and_then(|ffmpeg| sibling_tool(Path::new(&ffmpeg), "ffprobe")) {
        return sibling.into_os_string();
    }
    resolve_tool("ffprobe", ENV_FFPROBE)
}

/// The `stem` executable in the same folder as `tool`, when it exists.
pub fn sibling_tool(tool: &Path, stem: &str) -> Option<PathBuf> {
    let sibling = tool.parent()?.join(exe_name(stem));
    sibling.is_file().then_some(sibling)
}

/// Where `stem` ("ffmpeg" or "ffprobe") would run from, decided WITHOUT
/// starting any process: the resolved absolute file, the same path the launch
/// uses. None means the tool is not installed where Cut looks, so every step
/// that needs it must refuse up front.
pub fn locate_media_tool(stem: &str) -> Option<PathBuf> {
    locate_media_tool_uncached(stem).or_else(|| {
        // Automatic hardware selection may use a build found only by the wider
        // candidate scan (common system folders outside PATH).
        (stem == "ffmpeg" && auto_enabled())
            .then(|| ffmpeg_candidates().into_iter().next())
            .flatten()
    })
}

/// [`locate_media_tool`] without the automatic-selection fallback and without
/// touching the hardware-probe caches (used by the tools fingerprint).
fn locate_media_tool_uncached(stem: &str) -> Option<PathBuf> {
    let program = match stem {
        "ffmpeg" => {
            let has_env = std::env::var_os(ENV_FFMPEG).is_some_and(|p| !p.is_empty());
            let manual = (!has_env)
                .then(|| {
                    read_override_setting()
                        .filter(|s| Path::new(s).is_absolute() && Path::new(s).is_file())
                })
                .flatten();
            manual
                .map(OsString::from)
                .unwrap_or_else(|| resolve_tool("ffmpeg", ENV_FFMPEG))
        }
        _ => {
            let explicit = explicit_program(ENV_FFPROBE, "ffprobe");
            let chosen = explicit_program(ENV_FFMPEG, "ffmpeg").or_else(|| {
                read_override_setting()
                    .filter(|s| Path::new(s).is_absolute())
                    .map(OsString::from)
            });
            explicit
                .or_else(|| {
                    chosen
                        .and_then(|f| sibling_tool(Path::new(&f), "ffprobe"))
                        .map(PathBuf::into_os_string)
                })
                .unwrap_or_else(|| resolve_tool("ffprobe", ENV_FFPROBE))
        }
    };
    // Resolution is always an absolute path now (found, or the not-installed
    // place): the gate admits exactly the file the launch will start.
    let path = PathBuf::from(&program);
    (path.is_absolute() && path.is_file()).then_some(path)
}

/// The directory that holds the resolved ffmpeg, if it resolved to a real file
/// (any rung, including an absolute PATH entry). Used to set
/// SHELLX_CUT_FFMPEG_DIR for the python sidecar so its bare `ffmpeg` calls hit
/// the same binary. None when ffmpeg is not installed.
pub fn resolved_ffmpeg_dir() -> Option<PathBuf> {
    let p = PathBuf::from(ffmpeg());
    if p.is_file() {
        p.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// The deterministic startup directory for the Python sidecar, if the normal
/// resolution ladder found a real ffmpeg file. This intentionally does not
/// run the optional hardware-capability selector: that selector executes
/// several subprocess probes and is warmed by the background Doctor only after
/// the server has opened its loopback listener and given the UI time to paint.
///
/// Explicit executable and persisted manual choices retain their precedence;
/// only the opportunistic automatic hardware choice is deferred. The sidecar
/// still receives a usable, user-selected-or-ladder-resolved ffmpeg before its
/// first media job.
pub fn resolved_startup_ffmpeg_dir() -> Option<PathBuf> {
    let has_env = std::env::var_os(ENV_FFMPEG).is_some_and(|p| !p.is_empty());
    let program = if has_env {
        resolve_tool("ffmpeg", ENV_FFMPEG)
    } else if let Some(manual) = ffmpeg_override() {
        manual
    } else {
        resolve_tool("ffmpeg", ENV_FFMPEG)
    };
    let p = PathBuf::from(program);
    if p.is_file() {
        p.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// EVERY ffmpeg binary discoverable on this machine, RESOLVED-FIRST then deduped
/// by canonical path (existing files only). This is the "find any installed
/// ffmpeg" the doctor probes: the search-ladder binary, plus a full PATH walk,
/// plus common system locations (so a GUI .app with a Finder-stripped PATH still
/// finds /usr/bin, Homebrew, etc.). The doctor probes each for hardware
/// capability, reports the most capable, and suggests an official download when
/// none is accelerated. Order is significant: the resolved (default) binary is
/// first so the caller can tell "best" apart from "what we use today".
pub fn ffmpeg_candidates() -> Vec<PathBuf> {
    let want = exe_name("ffmpeg");
    // Collect in priority order, then dedup by canonical path below.
    let mut raw: Vec<PathBuf> = Vec::new();
    // The ladder-resolved binary first (NOT ffmpeg(): that is auto-aware and would
    // recurse back into this scan). The PATH walk below finds the others.
    let resolved = PathBuf::from(resolve_tool("ffmpeg", ENV_FFMPEG));
    if resolved.is_absolute() && resolved.is_file() {
        raw.push(resolved);
    }
    // The search-ladder dirs (env override, beside-exe, app-data, mac brew).
    for dir in ffmpeg_search_dirs() {
        raw.push(dir.join(&want));
    }
    // A FULL PATH walk (rung 4 resolves only the first; here we want them all).
    // Relative entries are skipped: these candidates are run to probe them,
    // and a relative entry would resolve against the current folder.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path).filter(|dir| dir.is_absolute()) {
            raw.push(dir.join(&want));
        }
    }
    // Common system locations (also the Finder-stripped-PATH safety net).
    for d in [
        "/usr/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/bin",
        "/snap/bin",
    ] {
        raw.push(PathBuf::from(d).join(&want));
    }
    // Existing files only, deduped by canonical path (symlinks/relative dups
    // collapse), preserving first-seen (resolved-first) order.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in raw {
        if !p.is_file() {
            continue;
        }
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            out.push(p);
        }
    }
    out
}

/// One tool's doctor status — what resolved and whether it is actually runnable.
#[derive(Debug, Clone)]
pub struct ToolStatus {
    /// Tool stem, e.g. "ffmpeg".
    pub name: String,
    /// The resolved program (full path or bare name).
    pub resolved: String,
    /// True when `resolved` is an existing file (rungs 1–3). False means it is a
    /// bare name relying on PATH — `on_path` then says whether PATH has it.
    pub bundled: bool,
    /// True when the tool can actually be spawned (`-version` exits 0). This is
    /// the honest "is it usable" bit the bootstrap state keys on.
    pub runnable: bool,
}

/// Probe one ffmpeg-family tool: resolve it, then try `<tool> -version` to prove
/// it is genuinely runnable (not just present). Best-effort; never panics.
fn doctor_ffmpeg_tool(stem: &str, prog: OsString) -> ToolStatus {
    let resolved = prog.to_string_lossy().into_owned();
    let bundled = Path::new(&prog).is_file();
    let mut command = std::process::Command::new(&prog);
    command.arg("-version");
    let runnable = crate::ffmpeg::run_bounded_command(&mut command, "probe media tool")
        .map(|output| output.status.success())
        .unwrap_or(false);
    ToolStatus {
        name: stem.to_string(),
        resolved,
        bundled,
        runnable,
    }
}

/// Resolve + runnability-probe ffmpeg and ffprobe. Cheap (two `-version`
/// spawns). Powers the `tools.doctor` verb and the desktop first-run bootstrap
/// state so a cold install is told exactly which media tool is missing.
pub fn doctor_media() -> Vec<ToolStatus> {
    vec![
        doctor_ffmpeg_tool("ffmpeg", ffmpeg()),
        doctor_ffmpeg_tool("ffprobe", ffprobe()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The candidate scan returns only EXISTING files, deduped by canonical path
    /// (so a symlink/relative duplicate of the same binary appears once). Portable:
    /// passes on a box with zero, one, or many ffmpegs.
    #[test]
    fn ffmpeg_candidates_are_existing_and_deduped() {
        let cands = ffmpeg_candidates();
        for p in &cands {
            assert!(p.is_file(), "candidate {p:?} must be an existing file");
        }
        let mut canon: Vec<_> = cands
            .iter()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
            .collect();
        let n = canon.len();
        canon.sort();
        canon.dedup();
        assert_eq!(
            canon.len(),
            n,
            "candidates must be deduped by canonical path"
        );
    }

    /// Candidates are run to probe them, so none may be a relative path that
    /// would resolve against the current folder.
    #[test]
    fn ffmpeg_candidates_are_absolute() {
        for p in ffmpeg_candidates() {
            assert!(p.is_absolute(), "candidate {p:?} must be absolute");
        }
    }

    #[test]
    fn exe_name_is_platform_correct() {
        if cfg!(windows) {
            assert_eq!(exe_name("ffmpeg"), "ffmpeg.exe");
        } else {
            assert_eq!(exe_name("ffmpeg"), "ffmpeg");
        }
    }

    #[test]
    fn isolated_shellx_home_owns_ffmpeg_preference() {
        let isolated = PathBuf::from("/tmp/shellx-cut-isolated");
        let shared = PathBuf::from("/opt/shellx-cut/tools");
        assert_eq!(
            override_file_with(Some(isolated.clone()), Some(shared.clone())),
            Some(isolated.join("preferences").join("ffmpeg-override")),
            "an isolated engine must not read or mutate the installed user's ffmpeg choice"
        );
        assert_eq!(
            override_file_with(None, Some(shared.clone())),
            Some(PathBuf::from("/opt/shellx-cut/ffmpeg-override")),
            "normal installed runs preserve the existing app-data preference location"
        );
    }

    #[test]
    fn explicit_env_full_path_wins() {
        // A bogus but non-empty path must be returned verbatim (rung 1a),
        // proving the override short-circuits all lower rungs.
        let key = "SHELLX_CUT_FFMPEG_TEST_ONLY";
        // Use resolve_tool directly with a private key to avoid clobbering the
        // real env for parallel tests.
        let custom = if cfg!(windows) {
            r"C:\nonexistent\custom\ffmpeg.exe"
        } else {
            "/nonexistent/custom/ffmpeg"
        };
        std::env::set_var(key, custom);
        let got = resolve_tool("ffmpeg", key);
        assert_eq!(got, OsString::from(custom));
        // A relative value is read against the current folder: never used.
        std::env::set_var(key, "./ffmpeg");
        let got = PathBuf::from(resolve_tool("ffmpeg", key));
        assert!(
            got.is_absolute() && got != Path::new("./ffmpeg"),
            "{}",
            got.display()
        );
        std::env::remove_var(key);
    }

    /// No required features ⇒ the feature-aware selector is identical to the normal
    /// resolver (non-caption renders keep auto-best-HW unchanged). Portable: holds
    /// regardless of which ffmpegs exist on the box.
    #[test]
    fn ffmpeg_for_empty_features_matches_default() {
        assert_eq!(ffmpeg_for(&[]), ffmpeg());
    }

    /// `caps_has` maps each FfmpegFeature to the right capability bit — the key the
    /// selector filters candidates by (a libass build is picked for captions even
    /// when a faster libass-less build outranks it on HW).
    #[test]
    fn caps_has_maps_features() {
        let caps = crate::hwencode::FfmpegCaps {
            path: "x".into(),
            version: Some("8.1".into()),
            hw: crate::hwencode::HwCaps::default(),
            cuda_filters: false,
            libass: true,
            vidstab: false,
            zscale: true,
            diagnosis: Default::default(),
        };
        assert!(caps_has(&caps, FfmpegFeature::Libass));
        assert!(!caps_has(&caps, FfmpegFeature::Vidstab));
        // zscale maps to its own bit — a color-managed render is routed to a
        // libzimg-capable build even when a faster build dropped zscale.
        assert!(caps_has(&caps, FfmpegFeature::Zscale));
    }

    /// The feature selector prefers a zscale-CAPABLE build for a color-managed render
    /// even when a FASTER (higher-HW-rank) build dropped libzimg — the exact Homebrew
    /// 8.x macOS failure mode (a videotoolbox-but-no-zscale ffmpeg outranks a slower
    /// software build that HAS zscale). This mirrors `ffmpeg_for`'s body (filter by
    /// `caps_has` then `max_by_key(rank)`) against MOCKED caps, so it is deterministic
    /// regardless of which ffmpegs the test box actually has.
    #[test]
    fn zscale_selection_prefers_capable_build_over_faster_one() {
        use crate::hwencode::{FfmpegCaps, HwCaps};
        // Fast Homebrew-style build: hardware encode, but NO zscale (libzimg dropped).
        let fast_no_zscale = FfmpegCaps {
            path: "/opt/homebrew/bin/ffmpeg".into(),
            version: Some("8.1".into()),
            hw: HwCaps {
                h264: Some("h264_videotoolbox".into()),
                ..Default::default()
            },
            cuda_filters: false,
            libass: false,
            vidstab: false,
            zscale: false,
            diagnosis: Default::default(),
        };
        // Slower software build that HAS zscale (e.g. ffmpeg-full / a static build).
        let slow_zscale = FfmpegCaps {
            path: "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg".into(),
            version: Some("8.1".into()),
            hw: HwCaps::default(),
            cuda_filters: false,
            libass: true,
            vidstab: true,
            zscale: true,
            diagnosis: Default::default(),
        };
        // Sanity: the no-zscale build really is the FASTER one (so a HW-only ranker
        // would wrongly pick it for a color-managed render).
        assert!(fast_no_zscale.rank() > slow_zscale.rank());

        let cands = [fast_no_zscale, slow_zscale];
        // Replicate ffmpeg_for(&[Zscale])'s selection predicate exactly.
        let want = [FfmpegFeature::Zscale];
        let picked = cands
            .iter()
            .filter(|c| c.version.is_some() && want.iter().all(|f| caps_has(c, *f)))
            .max_by_key(|c| c.rank())
            .map(|c| c.path.clone());
        assert_eq!(
            picked.as_deref(),
            Some("/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg"),
            "a color-managed render must pick the zscale-capable build, not the faster no-zscale one"
        );
    }

    #[test]
    fn never_resolves_to_a_bare_name() {
        // With no override, resolution is a real file found in an absolute
        // folder, or the absolute not-installed place — never the bare name a
        // relative PATH entry (the current folder) could answer.
        let key = "SHELLX_CUT_FFMPEG_DEFINITELY_UNSET_XYZ";
        std::env::remove_var(key);
        for stem in ["ffmpeg", "ffprobe"] {
            let got = PathBuf::from(resolve_tool(stem, key));
            assert!(got.is_absolute(), "{}", got.display());
            assert_eq!(got.file_stem().unwrap(), stem);
        }
    }
}
