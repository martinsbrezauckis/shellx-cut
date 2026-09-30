//! Admission for generated project directories.

use crate::error::{codes, CutError};
use std::path::Path;

pub(super) fn validate_internal_output_roots(dir: &Path) -> Result<(), CutError> {
    // These names belong to Cut's generated project state, not the user's
    // selected media source paths. Check parents too, before create_dir_all can
    // follow a linked parent while opening an untrusted project.
    const ROOTS: &[&str] = &[
        "receipts",
        "dub",
        "request-receipts",
        "proxies",
        "proxies/preview-cache",
        "filmstrip",
        "frames",
        "embeddings",
        "indexes",
        "previews",
        ".cache",
        ".cache/segrender",
        "cache",
        "cache/matte",
        "cache/mask",
        "cache/gwindow",
        "stab",
        "assets",
        "assets/generated",
        "assets/placeholders",
    ];
    validate_roots(dir, ROOTS)?;
    validate_job_output_roots(dir)
}

/// Preflight both job directories before any recovery side effect. Missing
/// directories are permitted; existing entries must be plain directories.
pub fn validate_job_output_roots(dir: &Path) -> Result<(), CutError> {
    validate_roots(dir, &["jobs", "jobs/quarantine"])
}

/// Request response roots may be absent; any existing entry must be a plain
/// directory before a request lookup or publication uses it as authority.
pub fn validate_request_receipt_output_root(dir: &Path) -> Result<(), CutError> {
    validate_roots(dir, &["request-receipts"])
}

fn validate_roots(dir: &Path, roots: &[&str]) -> Result<(), CutError> {
    for relative in roots {
        let path = dir.join(relative);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let reparse = false;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() || reparse {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "project output directory is not a plain directory",
                format!(
                    "refusing linked or non-directory project output {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}
