//! Revalidation of the anchored parent immediately before a camera seal.

use record_core::Result;

use crate::camera_finalization_anchored::same_directory;
use crate::camera_finalization_error::finalization_error;
use crate::camera_finalization_paths::CameraFinalizationPaths;
use crate::camera_finalization_publication::VerifiedCameraPublication;

pub(super) fn revalidate_published_parent(
    paths: &CameraFinalizationPaths<'_>,
    published: &VerifiedCameraPublication,
) -> Result<()> {
    let current_parent = paths.destination_parent()?;
    if !same_directory(&current_parent, published.destination_parent().identity()) {
        return Err(finalization_error(
            "camera publication directory identity changed",
            "the published directory no longer matches the owner-anchored route",
        ));
    }
    Ok(())
}
