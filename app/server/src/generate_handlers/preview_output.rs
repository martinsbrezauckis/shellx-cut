//! Admission for deterministic Generate PNGs served from the captured project.

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

/// Keep previews in the project's served frames tree, independent of the
/// session export folder. Repeated previews overwrite the same ordinary PNG,
/// but imported symlinks, reparse points and hardlinks cannot be write targets.
pub(super) fn project_preview_path(
    project_dir: &Path,
    file_name: &str,
) -> Result<PathBuf, CutError> {
    let frames =
        crate::output_paths::ensure_plain_project_relative_dir(project_dir, Path::new("frames"))?;
    cut_media::PathFence::new(project_dir)?.fence_output_path(&frames.join(file_name))
}

pub(super) fn copy_motion_preview(
    project_dir: &Path,
    source: &Path,
    file_name: &str,
) -> Result<PathBuf, CutError> {
    let target = project_preview_path(project_dir, file_name)?;
    // Admission is required even when Motion already returned the served path.
    if source != target {
        std::fs::copy(source, &target).map_err(|e| {
            CutError::new(
                error_codes::IO,
                "copy Motion preview into served frames dir",
                e.to_string(),
            )
        })?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests;
