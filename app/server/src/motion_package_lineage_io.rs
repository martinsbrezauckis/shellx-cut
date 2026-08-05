//! Bounded, race-aware package-file reads for current Motion lineage checks.

use cut_core::{error_codes, CutError};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) struct HashedBytes {
    pub(crate) bytes: Vec<u8>,
    pub(crate) sha256: String,
}

pub(crate) async fn read_hashed_file(
    path: &Path,
    max_bytes: u64,
    capture_bytes: usize,
    label: &str,
) -> Result<HashedBytes, CutError> {
    let path = path.to_path_buf();
    let label = label.to_string();
    let task_label = label.clone();
    tokio::task::spawn_blocking(move || {
        read_hashed_file_sync(&path, max_bytes, capture_bytes, &task_label)
    })
    .await
    .map_err(|error| lineage_io_error(format!("verify {label}"), error.to_string()))?
}

fn read_hashed_file_sync(
    path: &Path,
    max_bytes: u64,
    capture_bytes: usize,
    label: &str,
) -> Result<HashedBytes, CutError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| lineage_io_error(format!("inspect {label}"), error.to_string()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > max_bytes
    {
        return Err(lineage_io_error(
            format!("{label} must be a bounded regular non-symlink file"),
            path.display().to_string(),
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| lineage_io_error(format!("open {label}"), error.to_string()))?;
    let opened = file
        .metadata()
        .map_err(|error| lineage_io_error(format!("inspect open {label}"), error.to_string()))?;
    if !same_file_identity(&metadata, &opened) {
        return Err(lineage_io_error(
            format!("{label} changed before verification"),
            path.display().to_string(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity((opened.len().min(max_bytes) as usize).min(capture_bytes));
    let mut buffer = [0_u8; 1024 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| lineage_io_error(format!("read {label}"), error.to_string()))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > max_bytes {
            return Err(lineage_io_error(
                format!("{label} exceeds its byte limit"),
                total.to_string(),
            ));
        }
        hasher.update(&buffer[..read]);
        let capture = capture_bytes.saturating_sub(bytes.len()).min(read);
        bytes.extend_from_slice(&buffer[..capture]);
    }
    let after = file
        .metadata()
        .map_err(|error| lineage_io_error(format!("reinspect {label}"), error.to_string()))?;
    let path_after = std::fs::symlink_metadata(path).map_err(|error| {
        lineage_io_error(format!("reinspect path for {label}"), error.to_string())
    })?;
    if total != opened.len()
        || !same_file_identity(&opened, &after)
        || !same_file_identity(&opened, &path_after)
    {
        return Err(lineage_io_error(
            format!("{label} changed during verification"),
            path.display().to_string(),
        ));
    }
    Ok(HashedBytes {
        bytes,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn same_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    if !left.file_type().is_file()
        || !right.file_type().is_file()
        || left.len() != right.len()
        || left.modified().ok() != right.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) async fn canonical_root_file(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<PathBuf, CutError> {
    let path = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|error| lineage_io_error(format!("canonicalize {label}"), error.to_string()))?;
    if !path.starts_with(root) {
        return Err(lineage_io_error(
            format!("{label} escapes the current Motion package"),
            relative.to_string(),
        ));
    }
    let canonical_relative = path
        .strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"));
    if canonical_relative.as_deref() != Some(relative) || !path.is_file() {
        return Err(lineage_io_error(
            format!("{label} path is not a canonical package file"),
            relative.to_string(),
        ));
    }
    Ok(path)
}

fn lineage_io_error(message: impl Into<String>, cause: impl Into<String>) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, cause)
        .with_suggested_action("Select the unchanged Motion package used for this render")
}
