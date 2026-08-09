//! Role: resolves the per-user ShellX Cut INTERNAL state root (`~/.shellx-cut`) and
//! its sub-paths — the projects INDEX (`projects.json`) and the global asset LIBRARY
//! (`library/library.json` + `library/blobs/`). This is distinct from
//! `cut_media::toolpath` (which owns the ffmpeg / python TOOLS dir): that is
//! machinery, this is user DATA. User PROJECTS themselves stay in the visible
//! `default_projects_dir()` (dispatch.rs) — only the index + library live under the
//! dotdir, so a project's physical location is irrelevant to reopening it.
//!
//! Cross-platform: `~` resolves via `USERPROFILE` on Windows, `HOME` elsewhere.
//! Every helper returns `None` when no home var is set; callers degrade gracefully
//! (the index / library features simply no-op rather than crash a headless run).
//!
//! Callers: projects_index.rs and library.rs. Dependencies: standard library only.

use std::path::PathBuf;

/// `~/.shellx-cut` — the internal app-state root (projects index + asset library).
/// `None` when no home directory env var is set.
///
/// `SHELLX_CUT_HOME` overrides the whole root: portable installs, CI, and test
/// isolation. Under `cfg(test)` the root additionally defaults to a per-process
/// temp dir so tests never append temporary projects to real user state.
pub fn shellx_cut_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SHELLX_CUT_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    #[cfg(test)]
    {
        Some(test_home())
    }
    #[cfg(not(test))]
    {
        let home = if cfg!(windows) {
            std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
        } else {
            std::env::var_os("HOME")
        };
        home.filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(".shellx-cut"))
    }
}

/// Process-global scratch app-state root for unit tests (crate-internal tests
/// compile with `cfg(test)`, so every `project.create` in the dispatch suite
/// lands here instead of the real registry). Keyed by pid so parallel test
/// binaries don't share state; left behind in the OS temp dir (temp cleaners
/// own it — deleting on drop would race the parallel test threads).
#[cfg(test)]
fn test_home() -> PathBuf {
    use std::sync::OnceLock;
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("shellx-cut-test-home-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    })
    .clone()
}

/// `~/.shellx-cut/projects.json` — the projects index file.
pub fn projects_index_path() -> Option<PathBuf> {
    shellx_cut_home().map(|h| h.join("projects.json"))
}

/// `~/.shellx-cut/library` — the global asset library dir.
pub fn library_dir() -> Option<PathBuf> {
    shellx_cut_home().map(|h| h.join("library"))
}

/// `~/.shellx-cut/library/library.json` — the asset manifest.
pub fn library_manifest_path() -> Option<PathBuf> {
    library_dir().map(|d| d.join("library.json"))
}

/// `~/.shellx-cut/library/blobs` — content-addressed copies of opt-in library
/// assets (the "Copy into library (portable)" path).
pub fn library_blobs_dir() -> Option<PathBuf> {
    library_dir().map(|d| d.join("blobs"))
}

/// `~/.shellx-cut/library/posters` — cached single-frame poster / waveform
/// thumbnails for library items (see `http::serve_library_poster`). Path+mtime
/// keyed, so it is a pure derived cache (safe to delete; rebuilt on demand).
pub fn library_posters_dir() -> Option<PathBuf> {
    library_dir().map(|d| d.join("posters"))
}
