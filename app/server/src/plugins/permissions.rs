//! Durable, fail-closed persistence for the plugin permission fence.
//!
//! This module owns only the on-disk state. It accepts a missing state file as
//! the first-run default, but treats every other read/parse/shape failure as a
//! blocked permission decision. Writes use a same-directory temp file followed
//! by a platform-native replacement, so readers observe either the prior
//! complete state or the new one.

use super::{find, PluginAccess, BUILTIN_PLUGINS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};

static PERSISTENCE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Why a plugin gateway call is blocked before its own enabled flag is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStateProblem {
    Corrupt,
    Unavailable,
}

/// Result of an explicit persisted permission update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetEnabled {
    pub recovered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct PersistedPermissions {
    disabled: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PermissionState {
    Default,
    Ready(PersistedPermissions),
    Blocked(PermissionStateProblem),
}

impl PermissionState {
    pub(super) fn access(&self, name: &str) -> PluginAccess {
        match self {
            Self::Default => PluginAccess::Enabled,
            Self::Ready(state) if state.disabled.iter().any(|disabled| disabled == name) => {
                PluginAccess::Disabled
            }
            Self::Ready(_) => PluginAccess::Enabled,
            Self::Blocked(problem) => PluginAccess::Blocked(*problem),
        }
    }

    pub(super) fn status_json(&self) -> Value {
        match self {
            Self::Default | Self::Ready(_) => json!({ "status": "ready" }),
            Self::Blocked(PermissionStateProblem::Corrupt) => json!({
                "status": "corrupt",
                "recovery": "Run plugins.enable with one exact plugin name and enabled:true. This atomically repairs the saved state and enables only that plugin; approve other plugins separately.",
            }),
            Self::Blocked(PermissionStateProblem::Unavailable) => json!({
                "status": "unavailable",
                "recovery": "Restore access to the ShellX Cut app-data directory, then run plugins.enable with the exact plugin name and enabled:true.",
            }),
        }
    }
}

pub(super) fn current_state() -> PermissionState {
    match state_path() {
        Some(path) => read_state_at(&path),
        None => PermissionState::Blocked(PermissionStateProblem::Unavailable),
    }
}

pub(super) fn set_enabled(name: &str, enabled: bool) -> io::Result<SetEnabled> {
    let path = state_path()
        .ok_or_else(|| io::Error::other("cannot resolve ShellX Cut app-data directory"))?;
    set_enabled_at(&path, name, enabled)
}

fn state_path() -> Option<PathBuf> {
    // Beside the perception app-data dir (one ShellX Cut app-data root).
    cut_perception::appdata_sidecar_dir().and_then(|p| p.parent().map(|d| d.join("plugins.json")))
}

pub(super) fn read_state_at(path: &Path) -> PermissionState {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return PermissionState::Default,
        Err(_) => return PermissionState::Blocked(PermissionStateProblem::Unavailable),
    };
    if !metadata.file_type().is_file() {
        return PermissionState::Blocked(PermissionStateProblem::Corrupt);
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return PermissionState::Blocked(PermissionStateProblem::Unavailable),
    };
    let state = match serde_json::from_str::<PersistedPermissions>(&text) {
        Ok(state) => state,
        Err(_) => return PermissionState::Blocked(PermissionStateProblem::Corrupt),
    };
    if state_is_valid(&state) {
        PermissionState::Ready(state)
    } else {
        PermissionState::Blocked(PermissionStateProblem::Corrupt)
    }
}

fn state_is_valid(state: &PersistedPermissions) -> bool {
    let mut seen = std::collections::HashSet::new();
    state
        .disabled
        .iter()
        .all(|name| find(name).is_some() && seen.insert(name))
}

pub(super) fn set_enabled_at(path: &Path, name: &str, enabled: bool) -> io::Result<SetEnabled> {
    let lock = PERSISTENCE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = read_state_at(path);
    let recovered = matches!(
        previous,
        PermissionState::Blocked(PermissionStateProblem::Corrupt)
    );
    let mut disabled = match previous {
        PermissionState::Default => Vec::new(),
        PermissionState::Ready(state) => state.disabled,
        // Recovery begins from the secure baseline: no plugin gets a grant merely
        // because a malformed file could no longer be parsed.
        PermissionState::Blocked(PermissionStateProblem::Corrupt) => BUILTIN_PLUGINS
            .iter()
            .map(|plugin| plugin.name.to_string())
            .collect(),
        PermissionState::Blocked(PermissionStateProblem::Unavailable) => {
            return Err(io::Error::other(
                "plugin permission state is unavailable; restore app-data access before changing permissions",
            ));
        }
    };
    if enabled {
        disabled.retain(|disabled_name| disabled_name != name);
    } else if !disabled.iter().any(|disabled_name| disabled_name == name) {
        disabled.push(name.to_string());
    }
    write_state_atomically(path, &PersistedPermissions { disabled })?;
    Ok(SetEnabled { recovered })
}

fn write_state_atomically(path: &Path, state: &PersistedPermissions) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("plugin state path has no parent"))?;
    fs::create_dir_all(parent)?;
    let body = serde_json::to_vec_pretty(state)
        .map_err(|error| io::Error::other(format!("could not serialize plugin state: {error}")))?;
    let temp = temporary_path(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&body)?;
        file.sync_all()?;
        drop(file);
        replace_state_file(&temp, path)?;
        sync_parent_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(not(windows))]
fn replace_state_file(temp: &Path, path: &Path) -> io::Result<()> {
    // POSIX rename replaces an existing destination atomically when both paths
    // are on the same filesystem (the temp lives beside the state file).
    fs::rename(temp, path)
}

#[cfg(windows)]
fn replace_state_file(temp: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };

    let temp_wide = temp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if path.exists() {
        // SAFETY: both paths are nul-terminated UTF-16 buffers that live until
        // ReplaceFileW returns; no optional backup/exclude/preserve pointers are
        // supplied. ReplaceFileW atomically swaps an existing destination.
        if unsafe {
            ReplaceFileW(
                path_wide.as_ptr(),
                temp_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null(),
                std::ptr::null(),
            )
        } != 0
        {
            return Ok(());
        }
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both paths are nul-terminated UTF-16 buffers that live until
    // MoveFileExW returns. The replacement flag closes the race where another
    // process creates the state file between the existence check and this call.
    if unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            path_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn temporary_path(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("plugin state path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("plugin state path has no file name"))?;
    for _ in 0..64 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a temporary plugin state path",
    ))
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}
