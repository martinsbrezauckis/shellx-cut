//! The engine's private working folder (item 98 and its security review).
//!
//! Intermediate files the person never keeps (the app's playback audio, a
//! segmented render's joined video and list) live here, not in the project:
//! a project folder can come from someone else, and any create or delete
//! under it could be sent through a planted link or junction to a folder
//! outside the project. This folder is:
//!
//! - under the engine's own app-data directory (`SHELLX_CUT_HOME/work` when
//!   that is set, else `<app data>/ShellX Cut/work` beside the tools folder),
//!   never derived from a project path;
//! - one folder per engine process, created exclusively with a random name
//!   (`s-XXXXXX`, mode 0700 on Unix) by `tempfile`, so nothing can be waiting
//!   at that name;
//! - removed when the engine shuts down; the first use in a later process
//!   removes session folders older than 12 hours that a crashed engine left
//!   (a live engine's folder is younger: its files are rewritten on use).
//!
//! `remove_dir_all` does not follow links inside the tree on current Rust, and
//! the paths it is given here are built only from the app-data root and names
//! this module created.

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const SESSION_PREFIX: &str = "s-";
const CRASH_AGE: Duration = Duration::from_secs(12 * 3600);

static SESSION: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The root that holds every engine's session folder.
pub fn work_root() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("SHELLX_CUT_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home).join("work"));
    }
    crate::toolpath::appdata_tools_dir().and_then(|tools| tools.parent().map(|p| p.join("work")))
}

/// This engine's private folder, created on first use.
pub fn session_dir() -> Result<PathBuf, CutError> {
    let mut slot = SESSION.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(dir) = slot.as_ref().filter(|d| d.is_dir()) {
        return Ok(dir.clone());
    }
    let root = work_root().ok_or_else(|| {
        CutError::new(
            error_codes::IO,
            "no private working folder for this engine",
            "neither SHELLX_CUT_HOME nor the app-data folder is known",
        )
        .with_suggested_action("set SHELLX_CUT_HOME to a folder the engine may write")
    })?;
    std::fs::create_dir_all(&root).map_err(|e| {
        CutError::new(
            error_codes::IO,
            format!(
                "could not create the private working folder {}",
                root.display()
            ),
            e.to_string(),
        )
    })?;
    sweep_crashed(&root, CRASH_AGE);
    let dir = tempfile::Builder::new()
        .prefix(SESSION_PREFIX)
        .tempdir_in(&root)?
        .keep();
    // Owner-only regardless of the umask: ffmpeg output staging refuses a
    // folder others can write (they could replace a file between steps).
    owner_only(&dir)?;
    *slot = Some(dir.clone());
    Ok(dir)
}

#[cfg(unix)]
fn owner_only(dir: &Path) -> Result<(), CutError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn owner_only(_dir: &Path) -> Result<(), CutError> {
    Ok(())
}

/// A named subfolder of the session folder (created when missing).
pub fn session_subdir(name: &str) -> Result<PathBuf, CutError> {
    let dir = session_dir()?.join(name);
    match std::fs::create_dir(&dir) {
        Ok(()) => owner_only(&dir).map(|()| dir),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(dir),
        Err(e) => Err(e.into()),
    }
}

/// Remove this engine's session folder (engine shutdown).
pub fn remove_session_dir() {
    let mut slot = SESSION.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(dir) = slot.take() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Remove session folders older than `age` that a crashed engine left.
pub(crate) fn sweep_crashed(root: &Path, age: Duration) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(SESSION_PREFIX)
        {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|d| d >= age);
        if !stale {
            continue;
        }
        // A link at a session name is removed as a name, never followed.
        if meta.is_dir() && !meta.file_type().is_symlink() {
            let _ = std::fs::remove_dir_all(entry.path());
        } else {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_sweep_takes_old_sessions_only() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("s-old");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("joined.mp4"), b"x").unwrap();
        std::fs::write(root.path().join("not-ours.txt"), b"keep").unwrap();
        sweep_crashed(root.path(), Duration::ZERO);
        assert!(!old.exists());
        assert!(root.path().join("not-ours.txt").exists());
        let fresh = root.path().join("s-fresh");
        std::fs::create_dir(&fresh).unwrap();
        sweep_crashed(root.path(), Duration::from_secs(3600));
        assert!(
            fresh.exists(),
            "a live engine's folder is younger than the age"
        );
    }

    /// The session folder is owner-only whatever the umask (ffmpeg output
    /// staging refuses a group-writable folder).
    #[cfg(unix)]
    #[test]
    fn session_folder_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = session_subdir("perm-check").unwrap();
        for d in [dir.parent().unwrap(), dir.as_path()] {
            let mode = std::fs::metadata(d).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", d.display());
        }
        std::fs::remove_dir(&dir).unwrap();
    }

    /// A link at a session name is removed as a name; its target survives.
    #[cfg(unix)]
    #[test]
    fn crash_sweep_never_follows_a_link() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("s-planted")).unwrap();
        sweep_crashed(root.path(), Duration::ZERO);
        assert!(outside.path().join("sentinel").exists());
        assert!(std::fs::symlink_metadata(root.path().join("s-planted")).is_err());
    }
}
