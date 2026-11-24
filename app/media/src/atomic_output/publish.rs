//! Verification and atomic same-volume / cross-volume publication of staged
//! export files into a [`PinnedExportDir`].

use super::pinned_dir::PinnedExportDir;
use super::reserved::ReservedFile;
use super::staging::PrivateStageDir;
use cut_core::{error_codes, CutError};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static CROSS_VOL_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub(crate) enum PublishMode {
    Normal,
    #[cfg(test)]
    ForceCrossVolume,
}

struct TempChildGuard<'a> {
    dir: &'a PinnedExportDir,
    name: OsString,
    active: bool,
}

impl<'a> TempChildGuard<'a> {
    fn new(dir: &'a PinnedExportDir, name: OsString) -> Self {
        Self {
            dir,
            name,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TempChildGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.dir.unlink_child_best_effort(&self.name);
        }
    }
}

pub(crate) fn is_plain_regular_file(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file()
        && !metadata.file_type().is_symlink()
        && !cut_core::reparse::names_another_entry(metadata)
}

/// Remove a confirmed plain regular temporary file inside private staging.
pub(crate) fn remove_reserved_temporary_output(path: &Path) {
    if fs::symlink_metadata(path)
        .map(|metadata| is_plain_regular_file(&metadata))
        .unwrap_or(false)
    {
        let _ = fs::remove_file(path);
    }
}

/// Verify a finished staged file before moving it into an export folder:
/// regular single-link file owned by the current user, matching any prior
/// exclusive reservation, non-empty unless `allow_empty` is set.
pub(crate) fn verify_staged_file(
    staged_path: &Path,
    reserved: Option<&ReservedFile>,
    allow_empty: bool,
) -> Result<(File, fs::Metadata), CutError> {
    let file = open_staged_readonly_nofollow(staged_path)?;
    let meta = file.metadata()?;
    if let Some(expected) = reserved {
        let symlink_meta = fs::symlink_metadata(staged_path)?;
        if !expected.still_ours(&symlink_meta) || !is_plain_regular_file(&symlink_meta) {
            return Err(CutError::new(
                error_codes::FFMPEG,
                "the staged output was replaced while the render ran",
                "another user changed the file this render was writing, so it was not published",
            ));
        }
    }
    if !is_plain_regular_file(&meta) {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "ffmpeg output was not a regular local file",
            "the staged output became a symlink or reparse point",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let euid = unsafe { libc::geteuid() };
        if meta.uid() != euid || meta.nlink() != 1 {
            return Err(CutError::new(
                error_codes::FFMPEG,
                "the staged output was replaced while the render ran",
                "staged file ownership or hard-link count did not match",
            ));
        }
    }
    #[cfg(windows)]
    {
        if cut_core::reparse::handle_reparse_tag(&file)?
            .is_some_and(cut_core::reparse::is_name_surrogate_tag)
        {
            return Err(CutError::new(
                error_codes::FFMPEG,
                "ffmpeg output was not a regular local file",
                "the staged output is a Windows symlink or junction",
            ));
        }
        let (_, _, nlink) = cut_core::reparse::handle_identity(&file)?;
        if nlink != 1 {
            return Err(CutError::new(
                error_codes::FFMPEG,
                "the staged output was replaced while the render ran",
                "staged file has unexpected hard links",
            ));
        }
    }
    if !allow_empty && meta.len() == 0 {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "ffmpeg completed without output bytes",
            "the temporary final-render file was zero bytes",
        ));
    }
    Ok((file, meta))
}

fn open_staged_readonly_nofollow(path: &Path) -> Result<File, CutError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        Ok(file)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE,
            FILE_SHARE_READ, OPEN_EXISTING,
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_READ,
                // The verified handle stays open while the file is moved out of
                // the private folder, so the move needs delete sharing; nobody
                // else can open that folder.
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_handle(raw.cast()) })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(File::open(path)?)
    }
}

fn is_cross_device_error(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::CrossesDevices {
        return true;
    }
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::EXDEV) {
        return true;
    }
    #[cfg(windows)]
    if err.raw_os_error() == Some(17) {
        return true;
    }
    false
}

/// Publish a verified staged file into `dest_dir` under `target_name`:
/// - Re-verify `dest_dir` still matches its initial handle & ancestor path.
/// - Same-volume: atomic `renameat` (Unix) / `MoveFileExW` without
///   `MOVEFILE_COPY_ALLOWED` (Windows), neither following links at `target_name`.
/// - Cross-volume: exclusive temp file inside `dest_dir` (`O_EXCL`/`CREATE_NEW`),
///   copy + `sync_all`, re-verify `dest_dir`, atomic same-dir rename over
///   `target_name` (without following links), dir sync, remove staged file.
pub(crate) fn publish_verified_stage(
    stage_dir: Option<&PrivateStageDir>,
    stage_leaf: Option<&OsStr>,
    staged_path: &Path,
    mut staged_file: File,
    staged_len: u64,
    dest_dir: &PinnedExportDir,
    target_name: &OsStr,
    mode: PublishMode,
) -> Result<(), CutError> {
    dest_dir.verify_still_at_path()?;

    let skip_same_volume = match mode {
        PublishMode::Normal => false,
        #[cfg(test)]
        PublishMode::ForceCrossVolume => true,
    };

    if !skip_same_volume {
        #[cfg(unix)]
        let rename_res = match (stage_dir.and_then(PrivateStageDir::raw_fd), stage_leaf) {
            (Some(sfd), Some(leaf)) => dest_dir.rename_from_stage(Some(sfd), leaf, target_name),
            _ => dest_dir.rename_from_stage(None, staged_path.as_os_str(), target_name),
        };
        #[cfg(windows)]
        let rename_res = {
            let _ = (stage_dir, stage_leaf);
            let target_path = dest_dir.verified_child_path(target_name)?;
            PinnedExportDir::move_file_replace_no_copy(staged_path, &target_path)
        };
        match rename_res {
            Ok(()) => {
                dest_dir.fsync_best_effort();
                return Ok(());
            }
            Err(e) if is_cross_device_error(&e) => {}
            Err(e) => return Err(e.into()),
        }
    }

    let seq = CROSS_VOL_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = OsString::from(format!(".cut-pub-{}.{seq}.tmp", std::process::id()));
    let mut guard = TempChildGuard::new(dest_dir, tmp_name.clone());
    let mut dest_tmp = dest_dir.create_exclusive_temp(&tmp_name)?;
    let copied = std::io::copy(&mut staged_file, &mut dest_tmp)?;
    if copied != staged_len {
        return Err(CutError::new(
            error_codes::IO,
            "cross-volume export copy ended short of staged byte count",
            format!("copied {copied} of {staged_len} bytes"),
        ));
    }
    dest_tmp.flush()?;
    dest_tmp.sync_all()?;
    drop(dest_tmp);
    drop(staged_file);

    dest_dir.verify_still_at_path()?;

    #[cfg(unix)]
    dest_dir.rename_from_stage(Some(dest_dir.raw_fd()), &tmp_name, target_name)?;
    #[cfg(windows)]
    {
        let tmp_path = dest_dir.verified_child_path(&tmp_name)?;
        let target_path = dest_dir.verified_child_path(target_name)?;
        PinnedExportDir::move_file_replace_no_copy(&tmp_path, &target_path)?;
    }

    guard.disarm();
    dest_dir.fsync_best_effort();
    remove_reserved_temporary_output(staged_path);
    Ok(())
}
