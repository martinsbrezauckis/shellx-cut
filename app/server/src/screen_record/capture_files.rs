//! Plain-file validation for optional artifacts beside a screen capture.

use std::path::{Path, PathBuf};

use crate::output_paths::fenced_existing_file_under_dir;
use cut_core::{error_codes, CutError};

/// Resolve one existing capture artifact without following a final link/reparse
/// point. A missing leaf remains optional; any present unsafe leaf fails closed.
pub(crate) fn optional_plain_file_in_dir(
    base_dir: &Path,
    file_name: &str,
    label: &str,
    suggested_action: &str,
) -> Result<Option<PathBuf>, CutError> {
    if file_name.is_empty() || file_name.contains(['/', '\\', ':']) {
        return Err(unsafe_capture_leaf(
            &base_dir.join(file_name),
            label,
            suggested_action,
        ));
    }
    let path = base_dir.join(file_name);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(capture_leaf_error(&path, label, suggested_action, error)),
        Ok(_) => plain_existing_file_under_dir(base_dir, &path, label, suggested_action).map(Some),
    }
}

/// Validate a required capture artifact before returning a fenced canonical path.
pub(crate) fn plain_existing_file_under_dir(
    base_dir: &Path,
    path: &Path,
    label: &str,
    suggested_action: &str,
) -> Result<PathBuf, CutError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return fenced_existing_file_under_dir(base_dir, path, label, suggested_action)
        }
        Err(error) => return Err(capture_leaf_error(path, label, suggested_action, error)),
        Ok(_) => {}
    }
    if !record_recovery::is_plain_regular_file(path).map_err(|error| {
        capture_leaf_error(path, label, suggested_action, std::io::Error::other(error))
    })? {
        return Err(unsafe_capture_leaf(path, label, suggested_action));
    }
    fenced_existing_file_under_dir(base_dir, path, label, suggested_action)
}

/// Resolve a project-relative or absolute request only after rejecting a final
/// link/reparse point. This is for capture inputs that are addressed by a verb
/// argument rather than a fixed capture-leaf name.
pub(crate) fn plain_existing_file_under_project(
    project_dir: &Path,
    requested: &str,
    label: &str,
    suggested_action: &str,
) -> Result<PathBuf, CutError> {
    if requested.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} path is empty"),
            "a project-local file path is required",
        )
        .with_suggested_action(suggested_action));
    }
    let raw = Path::new(requested);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        project_dir.join(raw)
    };
    plain_existing_file_under_dir(project_dir, &candidate, label, suggested_action)
}

fn unsafe_capture_leaf(path: &Path, label: &str, suggested_action: &str) -> CutError {
    CutError::new(
        error_codes::IO,
        format!(
            "{label} is not a local regular capture file: {}",
            path.display()
        ),
        "capture artifact leaves cannot be links, reparse points, or non-regular files",
    )
    .with_suggested_action(suggested_action)
}

fn capture_leaf_error(
    path: &Path,
    label: &str,
    suggested_action: &str,
    error: std::io::Error,
) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not inspect {label} at {}: {error}", path.display()),
        "capture artifacts must remain local regular files",
    )
    .with_suggested_action(suggested_action)
}
