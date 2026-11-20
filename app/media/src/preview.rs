//! preview.rs — the draft preview of the whole timeline (`render.preview draft:true`).
//!
//! Role: a quick, small picture of the WHOLE current timeline that shows the
//! export's frames (the same source frame at every output frame, the same
//! length, speed, reversed clips, captions and overlays) at draft size, and
//! after an edit remakes only the pieces the edit changed. The pieces are
//! windows of the export's segmented render cut at the base track's clip
//! boundaries (`crate::render::render_draft`, which also documents the reuse
//! key); the sound is the export's single audio pass.
//!
//! WHAT IT IS NOT: derived state for fast feedback, never an op input and
//! never a receipt input (receipts stay render.final). Its size is the
//! project's aspect inside 960x540 and its encode is the draft tier.
//!
//! Dependencies: crate::render (window chain), cut-core (Project/Edl). Primary
//! caller: server render.preview{draft:true} (dispatch/rendering.rs).

use crate::render::RenderPreset;
use cut_core::{error_codes, CutError, Edl, Project};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Outcome of a draft preview build — what dispatch returns and what the
/// cache-correctness evidence reads. `segments_rendered` + `segments_reused`
/// count the draft's pieces (one per base-track clip or gap, a piece of no
/// frame joined to its neighbour); the named lists let the proof point at
/// exactly which piece was remade after a single-clip edit.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewResult {
    /// The joined preview file (draft size and encode) in the preview cache dir.
    pub path: PathBuf,
    /// How many pieces were (re)made this build.
    pub segments_rendered: usize,
    /// How many pieces were reused from the cache.
    pub segments_reused: usize,
    /// The cache filenames (draft2_<key>.mp4) that were freshly made.
    pub rendered: Vec<String>,
    /// The cache filenames that were reused from cache.
    pub reused: Vec<String>,
    /// Total composition duration (ms) the preview covers.
    pub duration_ms: u64,
}

/// Render the DRAFT preview of the whole timeline to `<cache_dir>/preview.mp4`:
/// the export's frames at draft size, remaking only the pieces whose content
/// changed (see the module doc). `preset` is the draft tier by convention.
pub fn render_preview_incremental(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    cache_dir: &Path,
    preset: &RenderPreset,
) -> Result<PreviewResult, CutError> {
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to preview",
            "EDL duration is 0 ms",
        )
        .with_suggested_action("insert at least one clip before render.preview draft"));
    }
    let cache_parent = cache_dir.parent().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "preview cache has no parent",
            cache_dir.display().to_string(),
        )
    })?;
    crate::render::ensure_plain_internal_child(cache_parent, cache_dir)?;
    edl.base_video_track().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "no base video track to preview",
            "the timeline has no video segments",
        )
    })?;

    crate::render::render_draft(project, edl, project_dir, cache_dir, preset)
}
