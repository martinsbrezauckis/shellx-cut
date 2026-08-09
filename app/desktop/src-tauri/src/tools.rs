// ─────────────────────────────────────────────────────────────────────────────
// tools.rs — desktop-side external-tool detection + first-run bootstrap surface
// for installed-app tool discovery.
//
// ROLE
//   The desktop shell is the FIRST thing that runs on a cold Windows notebook.
//   Before/while it spawns cutd, it must (a) tell the engine WHERE the bundled
//   or app-data ffmpeg / python sidecar live (via env vars the engine's
//   toolpath resolver reads), and (b) give the user an HONEST, actionable
//   bootstrap state when a heavy tool is missing — never a silent failure.
//
//   This module is intentionally SELF-CONTAINED (std only, no cut-media dep):
//   pulling the engine workspace into the Tauri shell crate would blow up the
//   shell build time and couple two deliberately-separate cargo trees. It is a
//   small, well-commented mirror of the resolution RUNGS in
//   app/media/src/toolpath.rs + app/perception/src/sidecar.rs — the contract
//   (env var names, dir layout) is the shared truth; keep them in sync.
//
//   See docs/public/BUILDING.md for the licensing and packaging design
//   this implements (ffmpeg: download-on-first-run, user-consented; python:
//   resolution fixed now + guided env fetch later).
// ─────────────────────────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};

/// Env var the engine's ffmpeg/ffprobe resolver reads (cut-media toolpath).
pub const ENV_FFMPEG_DIR: &str = "SHELLX_CUT_FFMPEG_DIR";

/// Env var: explicit full path to the ffmpeg executable — the engine's
/// highest-precedence rung (cut-media toolpath ENV_FFMPEG). The shell only
/// VALIDATES it for honest reporting; the spawned engine reads it itself.
pub const ENV_FFMPEG_EXE: &str = "SHELLX_CUT_FFMPEG";
/// Env var the perception sidecar reads to locate instruments.py + the venv.
pub const ENV_SIDECAR_DIR: &str = "SHELLX_CUT_SIDECAR_DIR";
/// Env var that turns on the engine's auto-selection of the best HARDWARE-capable
/// installed ffmpeg (so GPU "just works" — no user step). The desktop shell always
/// enables it; a user's explicit `SHELLX_CUT_FFMPEG` override still wins.
pub const ENV_FFMPEG_AUTO: &str = "SHELLX_CUT_FFMPEG_AUTO";

/// Platform exe name for a tool stem.
fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// %LOCALAPPDATA%\ShellX Cut\tools (and platform equivalents) — where the
/// first-run bootstrap downloads ffmpeg. Mirrors cut-media::toolpath.
fn appdata_tools_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("ShellX Cut").join("tools"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/ShellX Cut/tools"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|b| b.join("shellx-cut/tools"))
    }
}

/// %LOCALAPPDATA%\ShellX Cut\perception — where the bootstrap installs the
/// sidecar venv. Mirrors cut-perception::sidecar::appdata_sidecar_dir.
fn appdata_sidecar_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("ShellX Cut").join("perception"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/ShellX Cut/perception"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|b| b.join("shellx-cut/perception"))
    }
}

/// The engine's persisted manual ffmpeg choice (the UI "Change ffmpeg"
/// control, system.set_ffmpeg), when it points at an existing file. Mirrors
/// cut-media::toolpath::override_file_with EXACTLY: an isolated
/// `SHELLX_CUT_HOME` keeps its override under `<home>/preferences/`, otherwise
/// the file lives beside the app-data tools dir. The engine reads this file
/// itself at startup — the shell only consults it so `ffmpeg_ok` reports the
/// truth about what the engine will resolve.
fn manual_override_ffmpeg() -> Option<PathBuf> {
    let file = std::env::var_os("SHELLX_CUT_HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join("preferences").join("ffmpeg-override"))
        .or_else(|| {
            appdata_tools_dir()
                .and_then(|tools| tools.parent().map(|root| root.join("ffmpeg-override")))
        })?;
    let s = std::fs::read_to_string(file).ok()?;
    let p = PathBuf::from(s.trim());
    p.is_file().then_some(p)
}

/// macOS standard install locations the engine also checks (cut-media
/// toolpath rung 3b): a GUI .app — and equally an SSH non-login shell on a QA
/// host — gets a STRIPPED PATH without Homebrew's bin dirs, so a
/// `brew install ffmpeg` would be reported missing while the engine finds it.
#[cfg(target_os = "macos")]
fn macos_system_tool_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// Find a directory (among `dirs`, and each one's `bin/` subdir) that contains
/// the platform exe for `stem`. Returns the DIRECTORY (not the file) so callers
/// can hand it to the engine as SHELLX_CUT_FFMPEG_DIR.
fn find_tool_dir(stem: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let want = exe_name(stem);
    for d in dirs {
        for cand in [d.clone(), d.join("bin")] {
            if cand.join(&want).is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn sidecar_venv_python(base: &Path) -> PathBuf {
    if cfg!(windows) {
        base.join(".venv").join("Scripts").join("python.exe")
    } else {
        base.join(".venv").join("bin").join("python")
    }
}

fn sidecar_venv_python_usable_for_platform(python: &Path, macos: bool) -> bool {
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
        || resolved.parent() == Some(Path::new("/usr/bin"))
            && resolved
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("python3."))
}

/// True when `stem` (a bare tool name) is runnable on the current PATH —
/// `<stem> -version` exits successfully. Best-effort; never panics.
fn runnable_on_path(stem: &str, version_flag: &str) -> bool {
    std::process::Command::new(stem)
        .arg(version_flag)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The per-OS "ffmpeg is missing, here's the one-command fix" message.
/// Split out from `bootstrap_hint` so each platform gets an HONEST, actionable
/// instruction: a Mac user must never be told to download a
/// `win64` zip into `%LOCALAPPDATA%`. The target dir each branch names matches
/// `appdata_tools_dir()` for that OS exactly.
fn ffmpeg_missing_hint() -> String {
    // Each branch ends by saying what was searched. An unexplained
    // ffmpeg_ok=false on a box that has ffmpeg sends
    // the reader hunting; naming the searched rungs makes the report
    // falsifiable at a glance). The lists mirror detect_with_resources.
    if cfg!(target_os = "macos") {
        "ffmpeg is not installed. Core editing + render needs it.\n\
         One-time fix (no admin):\n\
         - With Homebrew:  brew install ffmpeg-full\n\
         \x20  Cut detects its keg-only path automatically after restart.\n\
         - Or download a static macOS build and put ffmpeg + ffprobe in:\n\
         \x20   ~/Library/Application Support/ShellX Cut/tools/ffmpeg/bin\n\
         Then restart ShellX Cut.\n\
         (Searched: SHELLX_CUT_FFMPEG / SHELLX_CUT_FFMPEG_DIR, the in-app\n\
         \x20ffmpeg choice, beside the app, the app-data tools dir above,\n\
         \x20/opt/homebrew/bin, /usr/local/bin, and PATH.)"
            .to_string()
    } else if cfg!(windows) {
        "ffmpeg is not installed. Core editing + render needs it.\n\
         One-time fix (downloads ~120 MB into your app data, no admin):\n\
         1. Download ffmpeg-master-latest-win64-gpl.zip from\n\
         \x20  https://github.com/BtbN/FFmpeg-Builds/releases\n\
         \x20  This is a separate GPL-licensed runtime, not part of Cut.\n\
         2. Extract it to:  %LOCALAPPDATA%\\ShellX Cut\\tools\\ffmpeg\n\
         \x20  (so ffmpeg.exe is at ...\\tools\\ffmpeg\\bin\\ffmpeg.exe)\n\
         3. Restart ShellX Cut.\n\
         (Searched: SHELLX_CUT_FFMPEG / SHELLX_CUT_FFMPEG_DIR, the in-app\n\
         \x20ffmpeg choice, beside the app, the app-data tools dir above, and PATH.)"
            .to_string()
    } else {
        "ffmpeg is not installed. Core editing + render needs it.\n\
         One-time fix: install it with your package manager so ffmpeg + ffprobe\n\
         are on PATH (e.g. `apt install ffmpeg` or `dnf install ffmpeg`), or put\n\
         them in:  ~/.local/share/shellx-cut/tools/ffmpeg/bin\n\
         Then restart ShellX Cut.\n\
         (Searched: SHELLX_CUT_FFMPEG / SHELLX_CUT_FFMPEG_DIR, the in-app\n\
         \x20ffmpeg choice, beside the app, the app-data tools dir above, and PATH.)"
            .to_string()
    }
}

/// Resolution outcome for the two heavy deps, computed once at startup.
/// `ffmpeg_dir` / `sidecar_dir`, when Some, are exported to the spawned engine.
#[derive(Debug, Clone, Default)]
pub struct ToolResolution {
    /// Directory holding ffmpeg[.exe]+ffprobe[.exe] (bundled or app-data), or
    /// None when we rely on PATH.
    pub ffmpeg_dir: Option<PathBuf>,
    /// True when ffmpeg is usable through ANY rung of the engine's resolution
    /// ladder (env override, manual choice, bundled/app-data dir, macOS system
    /// dir, or PATH) — the shell mirrors cut-media::toolpath so this boolean
    /// states the truth about what the engine will actually resolve.
    pub ffmpeg_ok: bool,
    /// Which ladder rung satisfied ffmpeg detection:
    /// "env" | "manual-override" | "bundled-or-appdata" | "system-dir" |
    /// "path" | "missing". Reported in logs + tools-doctor so a QA log line
    /// like ffmpeg_ok=true is auditable to its source.
    pub ffmpeg_source: &'static str,
    /// Directory holding the perception sidecar payload (instruments.py + venv).
    pub sidecar_dir: Option<PathBuf>,
    /// True when a python venv for the sidecar exists (bundled or app-data).
    pub sidecar_ok: bool,
}

impl ToolResolution {
    /// Resolve both deps against the installed layout. `exe_dir` is the
    /// directory of the running shell exe (cutd.exe sits beside it).
    #[cfg(test)]
    pub fn detect(exe_dir: &Path) -> Self {
        Self::detect_with_resources(exe_dir, exe_dir)
    }

    /// Resolve both deps against the installed layout plus Tauri's resource dir.
    /// On macOS, bundle resources live under `Contents/Resources`, while the
    /// executables live under `Contents/MacOS`; checking only `exe_dir` makes the
    /// packaged `perception/` payload invisible on fresh installs.
    pub fn detect_with_resources(exe_dir: &Path, resource_dir: &Path) -> Self {
        // ── ffmpeg — MIRRORS the engine ladder (cut-media::toolpath), same
        // precedence, so ffmpeg_ok never contradicts what the engine resolves:
        //   1a. SHELLX_CUT_FFMPEG (full path)  1b. SHELLX_CUT_FFMPEG_DIR
        //   1c. persisted manual choice (system.set_ffmpeg override file)
        //   2/3. beside-exe / resources / app-data tools dir
        //   3b. macOS system dirs (/opt/homebrew/bin, /usr/local/bin)
        //   4. PATH
        // The env/manual rungs are validated but NOT re-exported as
        // ffmpeg_dir: the engine reads those sources itself.
        let env_exe_ok = std::env::var_os(ENV_FFMPEG_EXE)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .is_some_and(|p| p.is_file());
        let env_dir_hit = std::env::var_os(ENV_FFMPEG_DIR)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .and_then(|d| find_tool_dir("ffmpeg", std::slice::from_ref(&d)));

        let mut ff_dirs = vec![exe_dir.join("ffmpeg")];
        if resource_dir != exe_dir {
            ff_dirs.push(resource_dir.join("ffmpeg"));
        }
        if let Some(t) = appdata_tools_dir() {
            ff_dirs.push(t.join("ffmpeg"));
        }
        let bundled_hit = find_tool_dir("ffmpeg", &ff_dirs);

        #[cfg(target_os = "macos")]
        let system_hit = find_tool_dir("ffmpeg", &macos_system_tool_dirs());
        #[cfg(not(target_os = "macos"))]
        let system_hit: Option<PathBuf> = None;

        let (ffmpeg_ok, ffmpeg_dir, ffmpeg_source): (bool, Option<PathBuf>, &'static str) =
            if env_exe_ok {
                (true, None, "env")
            } else if let Some(d) = env_dir_hit {
                (true, Some(d), "env")
            } else if manual_override_ffmpeg().is_some() {
                (true, None, "manual-override")
            } else if let Some(d) = bundled_hit {
                (true, Some(d), "bundled-or-appdata")
            } else if let Some(d) = system_hit {
                (true, Some(d), "system-dir")
            } else if runnable_on_path("ffmpeg", "-version") {
                (true, None, "path")
            } else {
                (false, None, "missing")
            };

        // ── sidecar: beside-exe `perception/` → app-data perception/ ─────────
        // "ok" requires a venv python, since instruments.py alone can't run the
        // whisper/torch stack. The script itself ships beside the exe.
        let mut sc_dirs = vec![exe_dir.join("perception")];
        if resource_dir != exe_dir {
            sc_dirs.push(resource_dir.join("perception"));
        }
        if let Some(s) = appdata_sidecar_dir() {
            sc_dirs.push(s);
        }
        let sidecar_dir = sc_dirs
            .iter()
            .find(|d| d.join("instruments.py").is_file())
            .cloned();
        let sidecar_python_dir = sc_dirs
            .iter()
            .find(|d| {
                sidecar_venv_python_usable_for_platform(
                    &sidecar_venv_python(d),
                    cfg!(target_os = "macos"),
                )
            })
            .cloned();
        let sidecar_ok = sidecar_dir.is_some() && sidecar_python_dir.is_some();

        Self {
            ffmpeg_dir,
            ffmpeg_ok,
            ffmpeg_source,
            sidecar_dir,
            sidecar_ok,
        }
    }

    /// Machine-readable resolution outcome as JSON. Written to a file at startup
    /// (see `write_doctor_file`) so the cold-install verification can prove WHICH
    /// ffmpeg/python the installed app resolved WITHOUT needing Tauri IPC — the
    /// engine-served UI is a remote origin and Tauri 2 does not grant remote
    /// origins access to custom app commands, so a file is the honest,
    /// ACL-independent proof surface.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ffmpeg": {
                "ok": self.ffmpeg_ok,
                "dir": self.ffmpeg_dir.as_ref().map(|p| p.display().to_string()),
                "source": self.ffmpeg_source,
            },
            "sidecar": {
                "ok": self.sidecar_ok,
                "dir": self.sidecar_dir.as_ref().map(|p| p.display().to_string()),
                "source": if self.sidecar_ok { "bundled-or-appdata" } else { "missing" },
            },
            "hint": self.bootstrap_hint(),
        })
    }

    /// Write `to_json()` to `<LOCALAPPDATA or HOME equiv>\ShellX Cut\tools-doctor.json`
    /// (the app-data root, parent of tools/). Best-effort — a failure here never
    /// affects launch. Returns the path written, for logging.
    pub fn write_doctor_file(&self) -> Option<PathBuf> {
        // App-data root = parent of the tools dir.
        let root = appdata_tools_dir().and_then(|t| t.parent().map(Path::to_path_buf))?;
        let _ = std::fs::create_dir_all(&root);
        let path = root.join("tools-doctor.json");
        let body = serde_json::to_string_pretty(&self.to_json()).ok()?;
        std::fs::write(&path, body).ok()?;
        Some(path)
    }

    /// A user-facing bootstrap summary: which heavy deps are missing and the
    /// exact one-command fix for each. Empty string ⇒ everything is present.
    /// Surfaced verbatim in engine_status so the cold notebook is never left
    /// guessing (NO silent failures).
    pub fn bootstrap_hint(&self) -> String {
        let mut lines = Vec::new();
        if !self.ffmpeg_ok {
            // Platform-specific fix: the resolver looks beside the exe, then in
            // the app-data tools/ffmpeg dir, then PATH (tools.rs::detect). Only
            // Windows and Linux have an in-app auto-fetcher (fetch.rs BtbN
            // builds); macOS relies on a package manager / PATH, so each OS gets its OWN
            // honest one-command fix — never a win64 zip path on a Mac.
            lines.push(ffmpeg_missing_hint());
        }
        if !self.sidecar_ok {
            // ASCII-only: this string round-trips through PowerShell consoles +
            // log files on cold boxes; non-ASCII dashes/arrows mojibake there.
            lines.push(
                "Transcription + perception (AI cuts, captions, receipts) need\n\
                 the Python sidecar - optional; core editing works without it.\n\
                 Set it up later from Tools > Set up perception, or skip."
                    .to_string(),
            );
        }
        lines.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn exe_name_platform() {
        if cfg!(windows) {
            assert_eq!(exe_name("ffprobe"), "ffprobe.exe");
        } else {
            assert_eq!(exe_name("ffprobe"), "ffprobe");
        }
    }

    #[test]
    fn detect_in_empty_dir_reports_no_bundled_ffmpeg() {
        let _guard = ENV_LOCK.lock().unwrap();
        // A temp dir with nothing in it: ffmpeg_dir must be None (no beside-exe
        // ffmpeg); ffmpeg_ok then reflects whatever PATH has on the build box.
        let tmp = std::env::temp_dir().join(format!("scut-tools-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("XDG_DATA_HOME", tmp.join("data"));
        std::env::set_var("HOME", tmp.join("home"));
        let r = ToolResolution::detect(&tmp);
        assert!(
            r.ffmpeg_dir.is_none(),
            "no ffmpeg should be found beside an empty dir"
        );
        // sidecar venv absent in a clean temp dir.
        assert!(!r.sidecar_ok, "no venv beside an empty dir");
        match old_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Save-and-restore guard for the env vars the ffmpeg-ladder tests touch,
    /// so a failing assert can't leak state into sibling tests.
    struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl EnvRestore {
        fn capture(keys: &[&'static str]) -> Self {
            Self(keys.iter().map(|k| (*k, std::env::var_os(k))).collect())
        }
    }
    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    const LADDER_ENV_KEYS: &[&str] = &[
        ENV_FFMPEG_EXE,
        ENV_FFMPEG_DIR,
        "SHELLX_CUT_HOME",
        "XDG_DATA_HOME",
        "HOME",
        "LOCALAPPDATA",
    ];

    /// The shell must report ffmpeg through the same ladder the engine resolves
    /// with — here the SHELLX_CUT_FFMPEG_DIR
    /// env rung, which detection previously ignored entirely.
    #[test]
    fn detect_env_dir_override_reports_env_source() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture(LADDER_ENV_KEYS);
        let tmp = std::env::temp_dir().join(format!("scut-envdir-test-{}", std::process::id()));
        let ffdir = tmp.join("ff");
        std::fs::create_dir_all(&ffdir).unwrap();
        std::fs::write(ffdir.join(exe_name("ffmpeg")), "").unwrap();
        std::env::remove_var(ENV_FFMPEG_EXE);
        std::env::set_var(ENV_FFMPEG_DIR, &ffdir);
        std::env::set_var("SHELLX_CUT_HOME", tmp.join("home"));
        std::env::set_var("XDG_DATA_HOME", tmp.join("data"));
        std::env::set_var("HOME", tmp.join("home"));
        std::env::set_var("LOCALAPPDATA", tmp.join("data"));

        let r = ToolResolution::detect(&tmp.join("empty"));
        assert!(r.ffmpeg_ok, "env dir rung must satisfy detection");
        assert_eq!(r.ffmpeg_source, "env");
        assert_eq!(r.ffmpeg_dir.as_deref(), Some(ffdir.as_path()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The persisted in-app ffmpeg choice (system.set_ffmpeg) must count as
    /// present — the engine reads that file itself, so reporting it missing
    /// contradicts actual product behavior.
    #[test]
    fn detect_manual_override_file_reports_manual_source() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture(LADDER_ENV_KEYS);
        let tmp = std::env::temp_dir().join(format!("scut-override-test-{}", std::process::id()));
        let home = tmp.join("cut-home");
        let chosen = tmp.join("bin").join(exe_name("ffmpeg"));
        std::fs::create_dir_all(chosen.parent().unwrap()).unwrap();
        std::fs::write(&chosen, "").unwrap();
        std::fs::create_dir_all(home.join("preferences")).unwrap();
        std::fs::write(
            home.join("preferences").join("ffmpeg-override"),
            format!("{}\n", chosen.display()),
        )
        .unwrap();
        std::env::remove_var(ENV_FFMPEG_EXE);
        std::env::remove_var(ENV_FFMPEG_DIR);
        std::env::set_var("SHELLX_CUT_HOME", &home);
        std::env::set_var("XDG_DATA_HOME", tmp.join("data"));
        std::env::set_var("HOME", tmp.join("unix-home"));
        std::env::set_var("LOCALAPPDATA", tmp.join("data"));

        let r = ToolResolution::detect(&tmp.join("empty"));
        assert!(r.ffmpeg_ok, "persisted manual choice must satisfy detection");
        assert_eq!(r.ffmpeg_source, "manual-override");
        assert!(
            r.ffmpeg_dir.is_none(),
            "the engine reads its own override file — do not re-export a dir"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The missing-ffmpeg hint must SAY what was searched (falsifiable report).
    #[test]
    fn missing_hint_names_searched_locations() {
        let hint = ffmpeg_missing_hint();
        assert!(hint.contains("Searched:"), "hint must list searched rungs");
        assert!(hint.contains("SHELLX_CUT_FFMPEG"), "hint must name the env overrides");
        assert!(hint.contains("PATH"), "hint must name the PATH rung");
    }

    /// Drift guard: the shell's detection MIRRORS cut-media::toolpath (a
    /// separate workspace, so no shared code). If the engine's ladder changes,
    /// this test points at the contract that must be re-mirrored.
    #[test]
    fn shell_ladder_mirrors_engine_toolpath() {
        let engine = include_str!("../../../media/src/toolpath.rs");
        assert!(engine.contains(&format!("pub const ENV_FFMPEG: &str = \"{}\";", ENV_FFMPEG_EXE)));
        assert!(engine.contains(&format!("pub const ENV_FFMPEG_DIR: &str = \"{ENV_FFMPEG_DIR}\";")));
        // rung 3b system dirs (macOS) — mirrored by macos_system_tool_dirs().
        assert!(engine.contains("/opt/homebrew/bin"));
        assert!(engine.contains("/usr/local/bin"));
        // manual override file location — mirrored by manual_override_ffmpeg().
        assert!(engine.contains(".join(\"preferences\").join(\"ffmpeg-override\")"));
        assert!(engine.contains("root.join(\"ffmpeg-override\")"));
    }

    #[test]
    fn detect_finds_packaged_resource_sidecar_payload() {
        let tmp = std::env::temp_dir().join(format!("scut-resource-test-{}", std::process::id()));
        let exe_dir = tmp.join("Contents").join("MacOS");
        let resource_dir = tmp.join("Contents").join("Resources");
        let perception = resource_dir.join("perception");
        let _ = std::fs::create_dir_all(&perception);
        std::fs::write(perception.join("instruments.py"), "# test payload").unwrap();

        let r = ToolResolution::detect_with_resources(&exe_dir, &resource_dir);
        assert_eq!(r.sidecar_dir.as_deref(), Some(perception.as_path()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detect_does_not_report_venv_only_sidecar_ready() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("scut-venv-only-test-{}", std::process::id()));
        let exe_dir = tmp.join("app");
        let appdata = tmp.join("data");
        let perception = appdata.join("shellx-cut").join("perception");
        let py = sidecar_venv_python(&perception);
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, "").unwrap();

        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("XDG_DATA_HOME", &appdata);
        std::env::set_var("HOME", tmp.join("home"));

        let r = ToolResolution::detect(&exe_dir);
        assert!(
            !r.sidecar_ok,
            "a venv without instruments.py is not a runnable sidecar"
        );
        assert!(
            r.sidecar_dir.is_none(),
            "do not export a venv-only dir as a payload dir"
        );

        match old_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn macos_stub_python_venv_is_not_sidecar_ready() {
        let tmp =
            std::env::temp_dir().join(format!("scut-stub-python-test-{}", std::process::id()));
        let stale = tmp.join("stale");
        let fresh = tmp.join("fresh");
        let stale_py = sidecar_venv_python(&stale);
        let fresh_py = sidecar_venv_python(&fresh);
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

        assert!(
            !sidecar_venv_python_usable_for_platform(&stale_py, true),
            "macOS Apple-stub venvs must not be reported ready"
        );
        assert!(
            sidecar_venv_python_usable_for_platform(&fresh_py, true),
            "non-stub managed venvs remain valid"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
