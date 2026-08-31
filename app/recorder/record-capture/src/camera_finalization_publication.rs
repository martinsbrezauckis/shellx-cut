//! Anchored no-replace publication for one nameless camera stage.

use record_core::Result;

use crate::camera_finalization_anchored::{
    link_nameless_no_replace, open_regular_at, AnchoredDirectory,
};
use crate::camera_finalization_error::finalization_error;
use crate::camera_finalization_identity::{same_file_identity, VerifiedRegularFile};
use crate::camera_finalization_paths::CameraFinalizationPaths;

/// The exact file and directory handles that were re-opened through the
/// capture owner for one final publication verification.
pub(crate) struct VerifiedCameraPublication {
    file: VerifiedRegularFile,
    destination_parent: AnchoredDirectory,
}

impl VerifiedCameraPublication {
    pub(crate) fn file(&self) -> &VerifiedRegularFile {
        &self.file
    }

    pub(crate) fn destination_parent(&self) -> &AnchoredDirectory {
        &self.destination_parent
    }
}

/// Publish through the opened destination directory, then re-anchor and open
/// the declared final leaf to prove that its identity is the exact descriptor
/// that was linked. No failure cleanup unlinks a path: POSIX has no atomic
/// compare-and-unlink primitive, so retaining an unsealed collision is safer
/// than deleting a replacement raced into the final name.
pub(crate) fn link_no_replace(
    paths: &CameraFinalizationPaths<'_>,
    stage: &VerifiedRegularFile,
) -> Result<()> {
    let destination_parent = paths.destination_parent()?;
    link_nameless_no_replace(stage, &destination_parent, paths.destination_name())
}

pub(crate) fn verify_published(
    paths: &CameraFinalizationPaths<'_>,
    stage: &VerifiedRegularFile,
) -> Result<VerifiedCameraPublication> {
    let destination_parent = paths.destination_parent()?;
    let destination = open_regular_at(&destination_parent, paths.destination_name())?;
    if !same_file_identity(stage, &destination) {
        return Err(finalization_error(
            "camera publication identity changed",
            "the anchored destination no longer names the exact staged descriptor",
        ));
    }
    Ok(VerifiedCameraPublication {
        file: destination,
        destination_parent,
    })
}
