//! Scope image siblings need their own final-leaf admission before rendering.

use cut_core::CutError;
use cut_media::scopes::ScopeKind;
use std::path::{Path, PathBuf};

pub(super) fn image_paths(
    project_dir: &Path,
    at_ms: u64,
    want_images: bool,
    kinds: &[ScopeKind],
) -> Result<Vec<(ScopeKind, PathBuf)>, CutError> {
    if !want_images || kinds.is_empty() {
        return Ok(Vec::new());
    }
    let directory = crate::output_paths::ensure_plain_project_relative_dir(
        project_dir,
        Path::new("exports/scopes"),
    )?;
    let fence = cut_media::PathFence::new(project_dir)?;
    kinds
        .iter()
        .map(|kind| {
            let path = directory.join(format!("scope_{at_ms}ms_{}.png", kind.key()));
            Ok((*kind, fence.fence_output_path(&path)?))
        })
        .collect()
}

#[cfg(test)]
mod tests;
