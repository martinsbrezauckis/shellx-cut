//! Cross-platform pinned export destination directory handle.
//!
//! Pinned export directories walk every component without following user/project
//! symlinks or Windows junctions/symlinks, hold an open handle/fd on the destination
//! directory from preflight until atomic publication, and re-verify that every
//! ancestor and the final directory identity remained intact at commit time.

use cut_core::{error_codes, CutError};
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use super::pinned_dir_unix::UnixPinnedDir;
#[cfg(windows)]
use super::pinned_dir_windows::WindowsPinnedDir;

/// An open, no-follow-verified handle to an export destination directory.
pub struct PinnedExportDir {
    normalized_path: PathBuf,
    #[cfg(unix)]
    inner: UnixPinnedDir,
    #[cfg(windows)]
    inner: WindowsPinnedDir,
}

fn normalize_lexical_abs(dir: &Path) -> Result<PathBuf, CutError> {
    if dir.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("output directory contains '..': {}", dir.display()),
            "parent-directory traversal is refused when pinning an export directory",
        ));
    }
    let abs = if dir.as_os_str().is_empty() {
        std::env::current_dir()?
    } else if dir.is_relative() {
        std::env::current_dir()?.join(dir)
    } else {
        dir.to_path_buf()
    };
    let mut clean = PathBuf::new();
    for c in abs.components() {
        if !matches!(c, Component::CurDir) {
            clean.push(c.as_os_str());
        }
    }
    Ok(clean)
}

impl PinnedExportDir {
    /// Open and pin `dir` without traversing user symlinks or Windows junctions.
    pub fn open(dir: &Path) -> Result<Self, CutError> {
        Self::open_with_options(dir, false)
    }

    pub(crate) fn open_with_options(dir: &Path, harden_project: bool) -> Result<Self, CutError> {
        let normalized_path = normalize_lexical_abs(dir)?;
        #[cfg(unix)]
        let inner = UnixPinnedDir::open(&normalized_path, harden_project)?;
        #[cfg(windows)]
        let inner = WindowsPinnedDir::open(&normalized_path, harden_project)?;
        Ok(Self {
            normalized_path,
            inner,
        })
    }

    /// Re-verify that the directory handle and all ancestor components along its
    /// path still resolve to the same physical directories opened earlier.
    pub fn verify_still_at_path(&self) -> Result<(), CutError> {
        self.inner.verify_still_at_path(&self.normalized_path)
    }

    /// Normalized path used when this directory was opened and verified.
    pub fn path(&self) -> &Path {
        &self.normalized_path
    }

    pub(crate) fn clear_stale_empty_child(&self, name: &OsStr) -> Result<(), CutError> {
        self.inner.clear_stale_empty_child(name)
    }

    #[cfg(unix)]
    pub(crate) fn raw_fd(&self) -> std::os::fd::RawFd {
        self.inner.raw_fd()
    }

    #[cfg(unix)]
    pub(crate) fn rename_from_stage(
        &self,
        stage_fd: Option<std::os::fd::RawFd>,
        stage_leaf_or_path: &OsStr,
        target_name: &OsStr,
    ) -> std::io::Result<()> {
        self.inner
            .rename_from_stage(stage_fd, stage_leaf_or_path, target_name)
    }

    #[cfg(windows)]
    pub(crate) fn verified_child_path(&self, name: &OsStr) -> Result<PathBuf, CutError> {
        self.inner.verified_child_path(name)
    }

    #[cfg(windows)]
    pub(crate) fn move_file_replace_no_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
        WindowsPinnedDir::move_file_replace_no_copy(src, dst)
    }

    pub(crate) fn create_exclusive_temp(&self, tmp_name: &OsStr) -> Result<File, CutError> {
        self.inner.create_exclusive_temp(tmp_name)
    }

    pub(crate) fn unlink_child_best_effort(&self, name: &OsStr) {
        self.inner.unlink_child_best_effort(name)
    }

    pub(crate) fn fsync_best_effort(&self) {
        #[cfg(unix)]
        self.inner.fsync_best_effort();
    }
}
