//! Before a final render, bake again every matte alpha the render will use
//! that this engine did not bake (item 155).
//!
//! The renderer uses a matte alpha only when this engine baked it
//! (`cut_media::cache_trust`); an alpha that came with the project (planted
//! under the predictable cache name, baked on another computer, or a legacy
//! name from an older version) would otherwise stop the render with "not made
//! on this computer". render.final therefore bakes those again first, through
//! the same `ensure_baked` as edit.matte: the person's first render of a copied
//! project is slower; nothing fails unless the matting tool is missing, and
//! then the bake's own error says how to install it.

use cut_core::{CutError, Edl, Project, TrackKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Returns how many alphas were baked again.
pub fn rebake_untrusted(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
) -> Result<usize, CutError> {
    let mut seen = HashSet::new();
    let mut baked = 0;
    for seg in &edl.segments {
        let (Some(m), Some(asset_id)) = (seg.matte.as_ref(), seg.asset.as_deref()) else {
            continue;
        };
        if seg.track_kind != TrackKind::Video
            || project.track(&seg.track).is_some_and(|t| !t.visible)
        {
            continue;
        }
        let Some(asset) = project.assets.get(asset_id) else {
            continue; // the renderer reports a missing asset itself
        };
        let alpha = cut_core::matte_cache::alpha_path(project_dir, &asset.hash, m)?;
        if !seen.insert(alpha.clone()) || !alpha.exists() || cut_media::cache_trust::trusted(&alpha)
        {
            continue;
        }
        let mut source = PathBuf::from(&asset.path);
        if source.is_relative() {
            source = project_dir.join(source);
        }
        super::ensure_baked(project_dir, &source, &asset.hash, m)?;
        baked += 1;
    }
    Ok(baked)
}
