//! Crash-durability barriers for an already verified camera publication.

use std::fs::File;

use record_core::Result;

use crate::camera_finalization_anchored::AnchoredDirectory;
use crate::camera_finalization_error::finalization_error;

/// Persist final media bytes after its read-only mode, measured facts, and hash
/// have all been verified through the anchored published descriptor.
pub(crate) fn sync_published_file(file: &File) -> Result<()> {
    file.sync_all().map_err(|error| {
        finalization_error(
            "camera published media cannot be synchronized",
            &error.to_string(),
        )
    })
}

/// Persist the no-replace directory entry through the exact anchored parent.
pub(crate) fn sync_publication_directory(parent: &AnchoredDirectory) -> Result<()> {
    parent.sync_directory()
}
