//! Path-free identity for the currently open project.

use cut_core::{error_codes, CutError, ProjectStore};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PROJECT_ID_SCHEMA: &str = "shellx-cut/project-identity/1";

/// Stable identity for receipts and state projections. The canonical origin
/// remains server-local; callers receive only its SHA-256 and safe display name.
pub(crate) fn project_identity(store: &ProjectStore) -> Result<Value, CutError> {
    let canonical = store.dir.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::IO,
            "open project origin cannot be resolved",
            error.to_string(),
        )
        .with_suggested_action("reopen the project from an available canonical .cutproj origin")
    })?;
    Ok(json!({
        "schema": PROJECT_ID_SCHEMA,
        "origin_path_sha256": format!("sha256:{:x}", Sha256::digest(canonical.to_string_lossy().as_bytes())),
        "project_name": store.project.name,
    }))
}
