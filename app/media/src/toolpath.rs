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
//!   4. system PATH              — bare "ffmpeg"/"ffprobe" (dev + power users).
//!
//! WHY THIS SEAM (the load-bearing design point)
//!   All three packaging futures cost ~nothing because each just populates a
//!   different rung: download-to-appdata (rung 3), bundle-in-installer (rung 2),
//!   dev-PATH (rung 4). We are NOT architected against bundling — flipping to a
//!   bundled ffmpeg later is one installer line, no code change here.
//!
//! HONESTY CONTRACT
//!   Resolution NEVER fabricates a path: if a tool is not found at rungs 1–3 we
//!   fall back to the bare name (rung 4) so the OS PATH lookup runs. When even
//!   that is absent the spawn fails with the engine's existing actionable
//!   ffmpeg/sidecar CutError. `doctor()` lets callers report the resolved path
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
use std::sync::OnceLock;

/// Env var: explicit full path to the ffmpeg executable (highest precedence).
pub const ENV_FFMPEG: &str = "SHELLX_CUT_FFMPEG";
/// Env var: explicit full path to the ffprobe executable.
pub const ENV_FFPROBE: &str = "SHELLX_CUT_FFPROBE";
/// Env var: a directory containing ffmpeg[.exe] + ffprobe[.exe]. Used both as a
/// resolution rung AND as the value the perception sidecar prepends to PATH so
/// `instruments.py`'s bare `ffmpeg` calls hit the SAME binary as Rust.
pub const ENV_FFMPEG_DIR: &str = "SHELLX_CUT_FFMPEG_DIR";
/// Env var: when truthy AND no explicit `SHELLX_CUT_FFMPEG` override is set, the
/// engine AUTO-SELECTS the best HARDWARE-capable installed ffmpeg (so GPU "just
/// works" with no user action). Opt-in — the desktop shell sets it; headless/CLI/
/// tests stay on the deterministic ladder unless they opt in. An explicit
/// override always wins (the user's manual choice); software-only machines stay
/// on the bundled build (auto only ever switches UP to a hardware build).
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
/// skipped — resolution falls through to PATH).
pub fn appdata_tools_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("ShellX Cut").join("tools"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("ShellX Cut")
                .join("tools")
        })
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|base| base.join("shellx-cut").join("tools"))
    }
}

// ── Manual ffmpeg override (the UI "Change ffmpeg" control) ────────────────────
//
// The user can pick a specific ffmpeg; it is persisted to a tiny file beside the
// app-data tools dir (survives app-bundle reinstalls) and read ONCE at engine
// startup. Reading it once (not live) keeps the HW-capability caches
// (hwencode::hw_caps / gpu_filters_available) CONSISTENT with the binary in use —
// so changing it takes effect on the next engine start (system.set_ffmpeg reports
// restart_required). An explicit `SHELLX_CUT_FFMPEG` env still wins over this.

/// The persisted-override file (one line: the chosen ffmpeg's absolute path).
/// `<app-data>/ShellX Cut/ffmpeg-override`. None when there is no app-data dir.
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
/// UI shows the current setting; resolution uses the cached [`ffmpeg_override`].
pub fn read_override_setting() -> Option<String> {
    let s = std::fs::read_to_string(override_file()?).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Persist (`Some`) or clear (`None`/empty) the manual ffmpeg override. Takes
/// effect on the next engine start (resolution caches the startup value). The
/// caller (system.set_ffmpeg) validates that the path is a runnable ffmpeg first.
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

/// The manual override in effect FOR THIS PROCESS — read once at startup, cached.
/// A stale path that no longer points at a file is IGNORED (so a deleted ffmpeg
/// can't break every op — resolution falls through to auto/ladder).
fn ffmpeg_override() -> Option<OsString> {
    static OVERRIDE: OnceLock<Option<OsString>> = OnceLock::new();
    OVERRIDE
        .get_or_init(|| {
            let s = read_override_setting()?;
            PathBuf::from(&s).is_file().then(|| OsString::from(s))
        })
        .clone()
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
    // brew-installed binary is found without the user fiddling with PATH. (Until
    // mac ffmpeg auto-fetch lands, this is how a Mac install gets ffmpeg.)
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

/// Resolve a tool ("ffmpeg" or "ffprobe") to a runnable program path.
/// `explicit_env` is the per-exe full-path override (rung 1a). Returns a full
/// path when found at rungs 1–3, else the BARE name (rung 4 = PATH lookup).
fn resolve_tool(stem: &str, explicit_env: &str) -> OsString {
    // rung 1a: explicit full path to THIS exe.
    if let Some(p) = std::env::var_os(explicit_env) {
        if !p.is_empty() {
            return p;
        }
    }
    let want = exe_name(stem);
    for dir in ffmpeg_search_dirs() {
        let cand = dir.join(&want);
        if cand.is_file() {
            return cand.into_os_string();
        }
    }
    // rung 4: PATH — bare name, the OS resolves it (or the spawn fails with the
    // engine's existing actionable error).
    OsString::from(stem)
}

/// Program path for ffmpeg (full path when resolvable, else bare "ffmpeg").
/// `Command::new` takes `AsRef<OsStr>`, so callers pass this straight through.
///
/// Resolution precedence:
///   1. explicit `SHELLX_CUT_FFMPEG` env override — always wins (power users / CI);
///   2. the persisted MANUAL choice (the UI "Change ffmpeg" control, system.set_ffmpeg);
///   3. AUTO (when `SHELLX_CUT_FFMPEG_AUTO` is on) — the best HARDWARE-capable
///      installed ffmpeg, so GPU "just works" with no user action;
///   4. the resolution ladder (bundled / beside-exe / app-data / PATH).
/// Auto never disturbs a software-only machine (it only switches UP to a hardware
/// build) and neither auto nor a manual choice overrides the env.
pub fn ffmpeg() -> OsString {
    let has_env = std::env::var_os(ENV_FFMPEG).is_some_and(|p| !p.is_empty());
    if !has_env {
        // 2. the user's persisted manual choice wins over auto + the ladder.
        if let Some(p) = ffmpeg_override() {
            return p;
        }
        // 3. auto best-hardware.
        if auto_enabled() {
            if let Some(best) = auto_best_ffmpeg() {
                return best;
            }
        }
    }
    resolve_tool("ffmpeg", ENV_FFMPEG)
}

/// True when auto-selection is enabled (`SHELLX_CUT_FFMPEG_AUTO` truthy).
fn auto_enabled() -> bool {
    matches!(
        std::env::var(ENV_FFMPEG_AUTO).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "on" | "On")
    )
}

/// The best HARDWARE-capable installed ffmpeg, or None when no installed ffmpeg
/// has hardware acceleration (then we stay on the deterministic ladder — software
/// renders are unchanged). Probed ONCE and cached for the process lifetime.
///
/// Recursion-safe: `ffmpeg_candidates()` resolves via the LADDER (not this
/// auto-aware path) and `probe_ffmpeg_caps` runs each binary by EXPLICIT path
/// (never `ffmpeg()`), so initialising this cache cannot re-enter `ffmpeg()`.
fn auto_best_ffmpeg() -> Option<OsString> {
    static BEST: OnceLock<Option<OsString>> = OnceLock::new();
    BEST.get_or_init(|| {
        let best = all_candidate_caps().iter().max_by_key(|c| c.rank())?;
        // Only auto-switch to a HARDWARE-capable build; software-only stays on the
        // ladder default so deterministic software renders never silently change.
        best.hw.any().then(|| OsString::from(best.path.clone()))
    })
    .clone()
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

/// Every discoverable ffmpeg's capabilities, PROBED ONCE and cached for the process
/// lifetime (the probe runs test encodes — expensive). Shared by auto-best-HW and
/// the feature-aware selector so each binary is probed at most once per process.
fn all_candidate_caps() -> &'static [crate::hwencode::FfmpegCaps] {
    static CAPS: OnceLock<Vec<crate::hwencode::FfmpegCaps>> = OnceLock::new();
    CAPS.get_or_init(|| {
        ffmpeg_candidates()
            .iter()
            .map(|p| crate::hwencode::probe_ffmpeg_caps(p))
            .collect()
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
/// stabilize ⇒ Vidstab). Among discoverable binaries it picks the highest-
/// acceleration build that provides ALL required features — so captions/stabilize
/// never silently fail just because the FASTEST build dropped a library, while
/// still preferring a hardware build WHEN it also has the feature. Precedence
/// mirrors [`ffmpeg`]: an explicit `SHELLX_CUT_FFMPEG` env or the persisted manual
/// choice always wins (the user's deliberate pick). Falls back to [`ffmpeg`] when
/// NO discoverable binary has the features (the render then degrades exactly as
/// before, and `doctor()` already flags the missing capability).
///
/// The auto-selector must not rank purely by hardware support: a
/// videotoolbox-but-libass-less build cannot burn captions. Caption/stabilize
/// renders now route here and get a feature-capable build.
pub fn ffmpeg_for(features: &[FfmpegFeature]) -> OsString {
    // Explicit overrides win — identical precedence to ffmpeg().
    if std::env::var_os(ENV_FFMPEG).is_some_and(|p| !p.is_empty()) {
        return resolve_tool("ffmpeg", ENV_FFMPEG);
    }
    if let Some(p) = ffmpeg_override() {
        return p;
    }
    if features.is_empty() {
        return ffmpeg();
    }
    let best = all_candidate_caps()
        .iter()
        .filter(|c| c.version.is_some() && features.iter().all(|f| caps_has(c, *f)))
        .max_by_key(|c| c.rank());
    match best {
        Some(c) => OsString::from(c.path.clone()),
        // No feature-capable build anywhere — degrade to the normal pick (doctor flags it).
        None => ffmpeg(),
    }
}

/// Program path for ffprobe (full path when resolvable, else bare "ffprobe").
pub fn ffprobe() -> OsString {
    resolve_tool("ffprobe", ENV_FFPROBE)
}

/// The directory that holds the resolved ffmpeg, if it resolved to a real file
/// (rungs 1–3). Used to set SHELLX_CUT_FFMPEG_DIR for the python sidecar so its
/// bare `ffmpeg` calls hit the same binary. None when ffmpeg came from PATH.
pub fn resolved_ffmpeg_dir() -> Option<PathBuf> {
    let p = PathBuf::from(ffmpeg());
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
    // recurse back into this scan). None when ffmpeg only lives on PATH as a bare
    // name — the PATH walk below still finds it.
    let resolved = PathBuf::from(resolve_tool("ffmpeg", ENV_FFMPEG));
    if resolved.is_file() {
        raw.push(resolved);
    }
    // The search-ladder dirs (env override, beside-exe, app-data, mac brew).
    for dir in ffmpeg_search_dirs() {
        raw.push(dir.join(&want));
    }
    // A FULL PATH walk (rung 4 resolves only the first; here we want them all).
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
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
        std::env::set_var(key, "/nonexistent/custom/ffmpeg");
        let got = resolve_tool("ffmpeg", key);
        assert_eq!(got, OsString::from("/nonexistent/custom/ffmpeg"));
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
    fn falls_back_to_bare_name_when_unresolved() {
        // With no overrides and (assumed) no beside-exe/app-data ffmpeg in the
        // test sandbox, resolution must yield the bare name so PATH lookup runs.
        let key = "SHELLX_CUT_FFMPEG_DEFINITELY_UNSET_XYZ";
        std::env::remove_var(key);
        let got = resolve_tool("ffmpeg", key);
        // Either a real beside-exe/app-data hit (full path) OR the bare name —
        // never empty, never a fabricated nonexistent path.
        assert!(!got.is_empty());
        if !Path::new(&got).is_file() {
            assert_eq!(got, OsString::from("ffmpeg"));
        }
    }
}
