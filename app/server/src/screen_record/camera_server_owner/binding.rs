//! Exact project identity and revision checks for the private camera owner.

use std::path::Path;

use cut_core::{error_codes, CutError, ProjectStore};
use sha2::{Digest, Sha256};

pub(super) fn current_revision(store: &ProjectStore) -> Result<String, CutError> {
    store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "private camera requires a durable project revision",
            "save or reopen the project before reserving the private camera owner",
        )
    })
}

pub(super) fn project_identity(project_dir: &Path) -> Result<String, CutError> {
    let canonical = std::fs::canonicalize(project_dir).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not resolve private camera project identity",
            error.to_string(),
        )
    })?;
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical.as_os_str().as_encoded_bytes())
    ))
}

pub(super) fn missing_capture(capture_id: &str) -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        "private camera capture directory is unavailable",
        format!("capture '{capture_id}' is not reserved under the open project"),
    )
}
