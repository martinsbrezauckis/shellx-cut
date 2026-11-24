//! Private per-user staging and parent-pinned atomic publication shared by
//! media renders and exports.

use cut_core::{error_codes, CutError};
use std::fs;
use std::io::Write;
use std::path::Path;

mod pinned_dir;
#[cfg(unix)]
mod pinned_dir_unix;
#[cfg(windows)]
mod pinned_dir_windows;
mod publish;
mod reserved;
mod staging;
#[cfg(windows)]
mod windows_acl;

pub use pinned_dir::PinnedExportDir;
pub use staging::{export_staging_root, sweep_stale_export_staging, PrivateStageDir};

/// True only when `path` is a plain file (not a link) owned by the current
/// user. Where the owner cannot be read the answer is false, so a caller that
/// deletes only owned files keeps the file.
pub fn is_plain_file_owned_by_current_user(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.uid() == unsafe { libc::geteuid() })
    }
    #[cfg(windows)]
    {
        windows_acl::file_is_owned_by_current_user(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        false
    }
}

/// True when `pid` still refers to a running process (or cannot be proven gone).
/// On doubt the answer is true, so callers never delete another live Cut
/// engine's in-flight staging file or folder.
pub fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let Ok(raw_pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 probes process existence without delivering a signal;
        // `raw_pid > 0` addresses one process rather than a group.
        let rc = unsafe { libc::kill(raw_pid, 0) };
        if rc == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, STILL_ACTIVE,
        };
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: `process` is closed before returning; `exit_code` is a local.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                // `ERROR_INVALID_PARAMETER` means no such process; `ERROR_ACCESS_DENIED`
                // or any other refusal keeps the entry.
                return GetLastError() != ERROR_INVALID_PARAMETER;
            }
            let mut exit_code = 0u32;
            let ok = GetExitCodeProcess(process, &mut exit_code);
            CloseHandle(process);
            ok == 0 || exit_code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

#[cfg(all(test, unix))]
use std::sync::atomic::AtomicU64;

/// Retained only to let unit tests construct the predictable legacy sibling path.
#[cfg(all(test, unix))]
static OUTPUT_SEQ: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(crate) fn private_test_tempdir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create private media test directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))
            .expect("protect media test directory from the caller's umask");
    }
    dir
}

/// The rule every FFmpeg export directory must meet, read-only (nothing is
/// tightened): verifies the folder and its parent chain without following
/// user/project symlinks, and refuses everyone-writable non-sticky directories.
pub fn check_output_dir_policy(path: &Path) -> Result<(), CutError> {
    let _ = PinnedExportDir::open_with_options(path, false)?;
    Ok(())
}

/// Split `out` into `(parent_dir, target_leaf_name)` requiring a normal final
/// component (never `.`, `..`, empty, or ending with a separator).
fn split_target(out: &Path) -> Result<(&Path, &std::ffi::OsStr), CutError> {
    let target_name = out.file_name().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("output path has no file name: {}", out.display()),
            "an export target must name a file inside a directory",
        )
    })?;
    let parent = out.parent().unwrap_or_else(|| Path::new(""));
    Ok((parent, target_name))
}

/// Remove only a stale zero-byte output via a pinned no-follow directory handle;
/// a prior valid render or symlinked destination is left intact.
pub(crate) fn clear_stale_empty_output(out: &Path) -> Result<(), CutError> {
    let (parent, target_name) = split_target(out)?;
    let pinned = PinnedExportDir::open_with_options(parent, true)?;
    pinned.clear_stale_empty_child(target_name)
}

/// Invoke `writer` with an exclusively reserved empty file in a private
/// `0700` / user-ACL staging directory, verify the finished file, and publish it
/// atomically into the pinned export folder without following target links.
pub fn stage_and_publish<T>(
    out: &Path,
    allow_empty: bool,
    writer: impl FnOnce(&Path) -> Result<T, CutError>,
) -> Result<T, CutError> {
    let (parent, target_name) = split_target(out)?;
    let dest_dir = PinnedExportDir::open_with_options(parent, true)?;
    if !allow_empty {
        dest_dir.clear_stale_empty_child(target_name)?;
    }

    let stage_dir = PrivateStageDir::new()?;
    let reserved = stage_dir.reserve_file_for(out)?;
    let value = match writer(&reserved.path) {
        Ok(value) => value,
        Err(err) => {
            publish::remove_reserved_temporary_output(&reserved.path);
            return Err(err);
        }
    };

    let (staged_file, staged_meta) =
        publish::verify_staged_file(&reserved.path, Some(&reserved.identity), allow_empty)?;
    publish::publish_verified_stage(
        Some(&stage_dir),
        Some(std::ffi::OsStr::new(&reserved.leaf_name)),
        &reserved.path,
        staged_file,
        staged_meta.len(),
        &dest_dir,
        target_name,
        publish::PublishMode::Normal,
    )?;
    Ok(value)
}

/// Stage `bytes` into a private user-only staging file, flush to disk (`sync_all`),
/// verify, and publish atomically into `out`'s pinned directory.
pub fn write_bytes_privately_and_publish(out: &Path, bytes: &[u8]) -> Result<(), CutError> {
    stage_and_publish(out, true, |staged_path| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(staged_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })
}

/// Verify an already-written staged file and move it atomically into `out`'s
/// pinned directory without following links at the target or its ancestors.
pub fn publish_staged_file(
    staged_path: &Path,
    out: &Path,
    allow_empty: bool,
) -> Result<(), CutError> {
    let (parent, target_name) = split_target(out)?;
    let dest_dir = PinnedExportDir::open_with_options(parent, false)?;
    let (staged_file, staged_meta) =
        match publish::verify_staged_file(staged_path, None, allow_empty) {
            Ok(verified) => verified,
            Err(err) => {
                publish::remove_reserved_temporary_output(staged_path);
                return Err(err);
            }
        };
    if let Err(err) = publish::publish_verified_stage(
        None,
        None,
        staged_path,
        staged_file,
        staged_meta.len(),
        &dest_dir,
        target_name,
        publish::PublishMode::Normal,
    ) {
        publish::remove_reserved_temporary_output(staged_path);
        return Err(err);
    }
    Ok(())
}

/// Invoke an output writer whose last CLI argument is the output path, staging
/// privately outside the export folder and publishing only after success.
pub(crate) fn run_with_atomic_output<T>(
    args: &[String],
    out: &Path,
    run: impl FnOnce(&[String]) -> Result<T, CutError>,
) -> Result<T, CutError> {
    run_with_validated_atomic_output(args, out, run, |_| Ok(()))
}

/// Variant of [`run_with_atomic_output`] that validates the completed staged
/// file before it becomes visible at the final path.
pub(crate) fn run_with_validated_atomic_output<T>(
    args: &[String],
    out: &Path,
    run: impl FnOnce(&[String]) -> Result<T, CutError>,
    validate: impl FnOnce(&Path) -> Result<(), CutError>,
) -> Result<T, CutError> {
    let (parent, target_name) = split_target(out)?;
    let dest_dir = PinnedExportDir::open_with_options(parent, true)?;
    dest_dir.clear_stale_empty_child(target_name)?;

    let stage_dir = PrivateStageDir::new()?;
    let reserved = stage_dir.reserve_file_for(out)?;
    let mut tmp_args = args.to_vec();
    if let Some(last) = tmp_args.last_mut() {
        *last = reserved.path.display().to_string();
    }

    let value = match run(&tmp_args) {
        Ok(value) => value,
        Err(error) => {
            publish::remove_reserved_temporary_output(&reserved.path);
            return Err(error);
        }
    };
    let (staged_file, staged_meta) =
        publish::verify_staged_file(&reserved.path, Some(&reserved.identity), false)?;
    if let Err(error) = validate(&reserved.path) {
        publish::remove_reserved_temporary_output(&reserved.path);
        return Err(error);
    }
    publish::publish_verified_stage(
        Some(&stage_dir),
        Some(std::ffi::OsStr::new(&reserved.leaf_name)),
        &reserved.path,
        staged_file,
        staged_meta.len(),
        &dest_dir,
        target_name,
        publish::PublishMode::Normal,
    )?;
    Ok(value)
}

#[cfg(test)]
mod tests;
