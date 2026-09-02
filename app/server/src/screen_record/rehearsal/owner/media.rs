//! Validation for the single server-owned rehearsal playback file.

use cut_core::{error_codes, CutError};
use std::path::{Component, Path, PathBuf};

/// Validate recorder output against the owned root. The only accepted result is
/// the native backend's fixed regular `source.mp4`; neither user input nor a
/// recorder-returned arbitrary output name can become a playback target.
pub(crate) fn media_relative_in_root(root: &Path, reported: &Path) -> Result<PathBuf, CutError> {
    let canonical_root = root.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::IO,
            "the disposable rehearsal workspace is unavailable",
            error.to_string(),
        )
    })?;
    if !reported.is_absolute() {
        return Err(CutError::new(
            error_codes::IO,
            "the native rehearsal did not return an owned media file",
            "the recorder returned a relative media path",
        ));
    }
    let metadata = std::fs::symlink_metadata(reported).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "the native rehearsal did not produce playable media",
            error.to_string(),
        )
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(CutError::new(
            error_codes::IO,
            "the native rehearsal did not produce playable media",
            "the recorder output was empty, non-regular, or a symbolic link",
        ));
    }
    let canonical_media = reported.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::IO,
            "the native rehearsal media could not be verified",
            error.to_string(),
        )
    })?;
    let relative = canonical_media.strip_prefix(&canonical_root).map_err(|_| {
        CutError::new(
            error_codes::IO,
            "the native rehearsal returned media outside its disposable workspace",
            "the recorder output escaped the server-owned rehearsal root",
        )
    })?;
    if relative != Path::new("source.mp4")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CutError::new(
            error_codes::IO,
            "the native rehearsal returned an invalid playback file",
            "only the regular source.mp4 in the owned rehearsal root is playable",
        ));
    }
    Ok(relative.to_path_buf())
}
