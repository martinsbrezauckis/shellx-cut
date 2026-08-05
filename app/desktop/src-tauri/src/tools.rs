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
    if cfg!(target_os = "macos") {
        "ffmpeg is not installed. Core editing + render needs it.\n\
         One-time fix (no admin):\n\
         - With Homebrew:  brew install ffmpeg-full\n\
         \x20  Cut detects its keg-only path automatically after restart.\n\
         - Or download a static macOS build and put ffmpeg + ffprobe in:\n\
         \x20   ~/Library/Application Support/ShellX Cut/tools/ffmpeg/bin\n\
         Then restart ShellX Cut."
            .to_string()
    } else if cfg!(windows) {
        "ffmpeg is not installed. Core editing + render needs it.\n\
         One-time fix (downloads ~120 MB into your app data, no admin):\n\
         1. Download ffmpeg-master-latest-win64-gpl.zip from\n\
         \x20  https://github.com/BtbN/FFmpeg-Builds/releases\n\
         \x20  This is a separate GPL-licensed runtime, not part of Cut.\n\
         2. Extract it to:  %LOCALAPPDATA%\\ShellX Cut\\tools\\ffmpeg\n\
         \x20  (so ffmpeg.exe is at ...\\tools\\ffmpeg\\bin\\ffmpeg.exe)\n\
         3. Restart ShellX Cut."
            .to_string()
    } else {
        "ffmpeg is not installed. Core editing + render needs it.\n\
         One-time fix: install it with your package manager so ffmpeg + ffprobe\n\
         are on PATH (e.g. `apt install ffmpeg` or `dnf install ffmpeg`), or put\n\
         them in:  ~/.local/share/shellx-cut/tools/ffmpeg/bin\n\
         Then restart ShellX Cut."
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
    /// True when ffmpeg is usable (bundled dir OR present on PATH).
    pub ffmpeg_ok: bool,
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
        // ── ffmpeg: beside-exe `ffmpeg/` → app-data tools/ffmpeg → PATH ──────
        let mut ff_dirs = vec![exe_dir.join("ffmpeg")];
        if resource_dir != exe_dir {
            ff_dirs.push(resource_dir.join("ffmpeg"));
        }
        if let Some(t) = appdata_tools_dir() {
            ff_dirs.push(t.join("ffmpeg"));
        }
        let ffmpeg_dir = find_tool_dir("ffmpeg", &ff_dirs);
        let ffmpeg_ok = ffmpeg_dir.is_some() || runnable_on_path("ffmpeg", "-version");

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
                "source": if self.ffmpeg_dir.is_some() { "bundled-or-appdata" }
                          else if self.ffmpeg_ok { "path" } else { "missing" },
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
