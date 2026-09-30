//! No-follow fingerprint/copy operations for admitted retry inputs.
use crate::jobs::JobInputFingerprint;
use cut_core::{error_codes, CutError};
use record_recovery::{is_plain_regular_file, PrivateStaging};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
pub(super) fn copy_input(
    project_dir: &Path,
    stage_parent: &Path,
    role: &str,
    source: &Path,
    stage_leaf: String,
) -> Result<(PathBuf, JobInputFingerprint, PrivateStaging), CutError> {
    if !is_plain_regular_file(source).map_err(|error| retry_io_error(source, error))? {
        return Err(retry_conflict(
            "retry inputs must remain local regular files",
        ));
    }
    let logical_path = input_path(project_dir, role, source)?;
    let stage = PrivateStaging::create(stage_parent, "recorder-retry-input", &stage_leaf)
        .map_err(|error| retry_io_error(stage_parent, error))?;
    let mut input = open_regular_nofollow(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.path())
        .map_err(|error| retry_io_error(stage.path(), error))?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| retry_io_error(source, error))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|error| retry_io_error(stage.path(), error))?;
        bytes = bytes
            .checked_add(u64::try_from(count).expect("read count fits u64"))
            .ok_or_else(|| retry_conflict("retry input byte count overflowed"))?;
    }
    output
        .sync_all()
        .map_err(|error| retry_io_error(stage.path(), error))?;
    Ok((
        stage.path().to_path_buf(),
        JobInputFingerprint {
            role: role.to_string(),
            path: logical_path,
            bytes,
            sha256: format!("{:x}", hash.finalize()),
        },
        stage,
    ))
}

pub(super) fn fingerprint_input(
    dir: &Path,
    role: &str,
    path: &Path,
) -> Result<JobInputFingerprint, CutError> {
    // First export retains its established renderer-path behavior. Only the
    // retry admission path below binds inputs to private no-follow snapshots.
    let mut file = open_regular_nofollow(path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| retry_io_error(path, error))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes = bytes
            .checked_add(u64::try_from(count).expect("read count fits u64"))
            .ok_or_else(|| retry_conflict("retry input byte count overflowed"))?;
    }
    Ok(JobInputFingerprint {
        role: role.to_string(),
        path: input_path(dir, role, path)?,
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

pub(super) fn stage_leaf(role: &str, path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .unwrap_or("media");
    format!("{role}.{extension}")
}

/// Validate the literal input before opening it, then prove the opened handle
/// is still a local regular file. Unix uses O_NOFOLLOW; Windows asks for the
/// reparse point itself and rejects it from the opened metadata.
fn open_regular_nofollow(path: &Path) -> Result<File, CutError> {
    if !is_plain_regular_file(path).map_err(|error| retry_io_error(path, error))? {
        return Err(retry_conflict(
            "retry inputs must remain local regular files",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| retry_io_error(path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| retry_io_error(path, error))?;
    if is_open_plain_regular(&metadata) {
        Ok(file)
    } else {
        Err(retry_conflict(
            "retry input changed while opening without following links",
        ))
    }
}

fn is_open_plain_regular(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

pub(super) fn project_relative_path(dir: &Path, path: &Path) -> Result<String, CutError> {
    let dir = dir
        .canonicalize()
        .map_err(|error| retry_io_error(dir, error))?;
    let path = path
        .canonicalize()
        .map_err(|error| retry_io_error(path, error))?;
    path.strip_prefix(dir)
        .map_err(|_| retry_conflict("retry inputs must remain inside the active project"))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn retry_io_error(path: &Path, error: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not stage retry input {}: {error}", path.display()),
        "restore the recorder export input or start a new export",
    )
}
pub(super) fn retry_staging_join_error(error: tokio::task::JoinError) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        if error.is_panic() {
            "recorder retry input staging worker panicked"
        } else {
            "recorder retry input staging worker was cancelled"
        },
        error.to_string(),
    )
    .with_suggested_action("retry the export after project storage is available")
}

fn retry_conflict(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "screen-record export retry is not safe to admit",
        cause.into(),
    )
}

fn input_path(dir: &Path, role: &str, path: &Path) -> Result<String, CutError> {
    if role == "plan_webcam" {
        // Only a camera admitted from the initiating snapshot may be external.
        let root = dir
            .canonicalize()
            .map_err(|error| retry_io_error(dir, error))?;
        let path = path
            .canonicalize()
            .map_err(|error| retry_io_error(path, error))?;
        return Ok(path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/"));
    }
    project_relative_path(dir, path)
}
