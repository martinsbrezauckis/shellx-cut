//! Graph input resolution and cache-backed alpha stream collection.
//!
//! This owns the ordered FFmpeg `-i` list shared by software and GPU graph
//! builders: project media, replace-matte backgrounds, and cached alpha inputs.

use super::{input_paths::strip_verbatim_prefix, mask_uses_geq, matte, source_dims};
use cut_core::{error_codes, CutError, Edl, EdlSegment, Project, TrackKind};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One `-i` input of the graph. Stills (kind=image probes) are fed with
/// `-loop 1` so the single frame becomes an infinite stream the segment
/// chain can trim to the clip duration (the renderer "loops the still").
pub(super) struct GraphInput {
    pub(super) path: PathBuf,
    /// True when the asset probed as a still image (probe.kind == "image").
    pub(super) image: bool,
    /// GPU fast-track: NVDEC-decode THIS input to CUDA frames (graph_args emits
    /// `-hwaccel cuda` for it). Per-INPUT (not whole-graph) because the GPU path
    /// keeps the expensive BASE track on the GPU (NVDEC→scale_cuda→nvenc) while
    /// OVERLAY inputs decode on the CPU — the small overlay needs CPU-only filters
    /// (pad/colorchannelmixer/transparent-filler) before a single hwupload into
    /// `overlay_cuda` (2b-ii). Software graph: always false (system-memory frames).
    pub(super) gpu_decode: bool,
}

/// Absolute path to a clip's baked matte alpha under the project cache. The
/// renderer (reader) and the server bake step (writer) both route through
/// `ClipMatte::cache_filename` so they always agree.
pub(super) fn matte_alpha_path(
    project_dir: &Path,
    asset_hash: &str,
    m: &cut_core::ClipMatte,
) -> Result<PathBuf, CutError> {
    cut_core::matte_cache::alpha_path(project_dir, asset_hash, m)
}

/// Graph-input map key for a matte alpha file (distinct from asset-id keys so a
/// matte and a same-named asset never collide).
pub(super) fn matte_input_key(alpha_path: &Path) -> String {
    format!("matte::{}", alpha_path.display())
}

/// Absolute path to a clip's baked mask alpha PNG under the project cache. Content-
/// addressed by the mask geometry + frame size (`ClipMask::cache_tag`), so identical
/// masks share one baked file and a render reuses it.
pub(super) fn mask_alpha_path(
    project_dir: &Path,
    mask: &cut_core::ClipMask,
    w: u32,
    h: u32,
) -> PathBuf {
    project_dir
        .join("cache")
        .join("mask")
        .join(format!("{}.png", mask.cache_tag(w, h)))
}

/// Graph-input map key for a mask alpha file (distinct from asset/matte keys).
pub(super) fn mask_input_key(alpha_path: &Path) -> String {
    format!("mask::{}", alpha_path.display())
}

/// Absolute path to a power-window's baked shape-alpha PNG (edit.grade_window). The
/// window's geometry is lowered to an ephemeral [`cut_core::ClipMask`]
/// (`WindowShape::to_mask`), so the alpha bake + content-address reuse the proven mask
/// path verbatim — identical window shapes (across windows or clips) share one baked file.
/// Stored under a distinct `gwindow/` cache dir so a window alpha and a same-shaped mask
/// alpha never collide.
pub(super) fn window_alpha_path(
    project_dir: &Path,
    win: &cut_core::WindowShape,
    w: u32,
    h: u32,
) -> PathBuf {
    project_dir
        .join("cache")
        .join("gwindow")
        .join(format!("{}.png", win.to_mask().cache_tag(w, h)))
}

/// Graph-input map key for a power-window alpha file (distinct from asset/matte/mask keys).
pub(super) fn window_input_key(alpha_path: &Path) -> String {
    format!("gwindow::{}", alpha_path.display())
}

pub(super) fn segment_video_track_visible(project: &Project, seg: &EdlSegment) -> bool {
    if seg.track_kind != TrackKind::Video {
        return true;
    }
    match project.track(&seg.track) {
        Some(track) => track.visible,
        // EDL segments normally reference live project tracks. If an imported or
        // legacy EDL lacks that track, preserve the historical render behavior.
        None => true,
    }
}

pub(super) fn bake_mask_png_atomic(
    mask: &cut_core::ClipMask,
    w: u32,
    h: u32,
    alpha_path: &Path,
) -> Result<(), CutError> {
    if let Some(parent) = alpha_path.parent() {
        let cache = parent.parent().ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "mask cache has no parent",
                parent.display().to_string(),
            )
        })?;
        let project = cache.parent().ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "mask cache has no project",
                cache.display().to_string(),
            )
        })?;
        super::ensure_plain_internal_child(project, cache)?;
        super::ensure_plain_internal_child(cache, parent)?;
    }
    if super::plain_internal_file_exists(alpha_path)? {
        return Ok(());
    }
    let parent = alpha_path
        .parent()
        .expect("mask cache parent checked above");
    let reserved = tempfile::Builder::new()
        .prefix(".cut-mask-")
        .suffix(".png")
        .tempfile_in(parent)?;
    let tmp = reserved.into_temp_path();
    crate::mask::bake_mask_png(mask, w, h, &tmp)?;
    super::plain_internal_file_exists(alpha_path)?;
    match std::fs::rename(&tmp, alpha_path) {
        Ok(()) => Ok(()),
        Err(e) if super::plain_internal_file_exists(alpha_path)? => {
            drop(e);
            Ok(())
        }
        Err(e) => Err(CutError::new(
            error_codes::FFMPEG,
            "mask PNG publish failed",
            e.to_string(),
        )),
    }
}

/// Preview output geometry: scale `(w,h)` to height `ph`, preserving aspect and
/// even yuv420 dimensions. No-op when `ph >= h`; previews never upscale.
pub(super) fn preview_geometry(w: u32, h: u32, ph: u32) -> (u32, u32) {
    if h == 0 || ph == 0 || ph >= h {
        return (w & !1, h & !1);
    }
    let pw = ((w as u64 * ph as u64 + (h as u64) / 2) / h as u64) as u32;
    ((pw.max(2)) & !1, (ph.max(2)) & !1)
}

/// Whether an asset's proxy can safely replace its raw source in a preview.
/// Requires an existing clean downscale with matching aspect and no source-pixel
/// geometry such as crop or stabilization. Coordinate-free effects remain valid
/// at proxy resolution; ineligible assets fall back to the raw source.
fn asset_proxy_ok(project: &Project, edl: &Edl, asset_id: &str, asset: &cut_core::Asset) -> bool {
    let _ = project;
    if asset.proxy.is_none() {
        return false;
    }
    let Some((sw, sh)) = source_dims(asset) else {
        return false;
    };
    let src_ar = sw as f64 / sh as f64;
    let proxy_ar = crate::proxy::PROXY_WIDTH as f64 / crate::proxy::PROXY_HEIGHT as f64;
    if (src_ar - proxy_ar).abs() > 0.01 {
        return false; // letterboxed proxy — bars would composite as content
    }
    !edl.segments.iter().any(|s| {
        s.asset.as_deref() == Some(asset_id) && (s.crop.is_some() || s.stabilize.is_some())
    })
}

/// Add one imported media asset to a graph, once.  Matte replacement can refer
/// to an asset that has no timeline segment of its own, so the normal timeline
/// input collection and `MatteBg::Asset` share this resolver.
fn add_graph_asset_input(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    use_proxy: bool,
    asset_id: &str,
    clip_id: Option<&str>,
    inputs: &mut Vec<GraphInput>,
    input_idx: &mut BTreeMap<String, usize>,
) -> Result<(), CutError> {
    if input_idx.contains_key(asset_id) {
        return Ok(());
    }
    let asset = project.assets.get(asset_id).ok_or_else(|| {
        let err = CutError::new(
            error_codes::NOT_FOUND,
            format!("asset {asset_id} referenced by the timeline does not exist"),
            "EDL references an asset id missing from project.assets",
        );
        match clip_id {
            Some(clip) => err.with_clip(clip),
            None => err,
        }
    })?;
    // Decode the proxy when it is coordinate-safe. `export.frame` passes
    // `use_proxy=false` for full-resolution source; a missing proxy falls back.
    let proxy_rel = if use_proxy && asset_proxy_ok(project, edl, asset_id, asset) {
        asset.proxy.clone()
    } else {
        None
    };
    let mut path = PathBuf::from(proxy_rel.as_deref().unwrap_or(asset.path.as_str()));
    if path.is_relative() {
        path = project_dir.join(path); // Asset (and proxy) paths may be project-relative.
    }
    if proxy_rel.is_some() && !path.exists() {
        // Proxy vanished — fall back to the raw source so the preview still renders.
        path = PathBuf::from(&asset.path);
        if path.is_relative() {
            path = project_dir.join(path);
        }
    }
    // Rust canonicalization stamps Windows paths with `\\?\`, but the shipped
    // FFmpeg build cannot open that extended form. Keep canonical paths for
    // ownership checks and persistence, then hand external media tools the
    // equivalent plain drive/UNC path.
    path = strip_verbatim_prefix(&path);
    if !path.exists() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!(
                "media file for asset {asset_id} not found: {}",
                path.display()
            ),
            "source file moved or deleted since import",
        )
        .with_suggested_action(
            "repoint the asset at the file's new location via media.relink {asset, path} \
             (media.check lists offline assets), or restore the file",
        ));
    }
    // Stills are looped at the input (-loop 1) so the trim chain can cut
    // the clip's duration out of an infinite single-frame stream.
    let image = asset
        .probe
        .as_ref()
        .and_then(|p| p.get("kind"))
        .and_then(|k| k.as_str())
        == Some("image");
    input_idx.insert(asset_id.to_string(), inputs.len());
    // gpu_decode defaults false (CPU decode); build_graph_gpu flips it on for
    // BASE-track inputs only (NVDEC). The software path leaves it false.
    inputs.push(GraphInput {
        path,
        image,
        gpu_decode: false,
    });
    Ok(())
}

pub(super) fn collect_graph_inputs(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    use_proxy: bool,
    with_video: bool,
    output_w: u32,
    output_h: u32,
) -> Result<(Vec<GraphInput>, BTreeMap<String, usize>), CutError> {
    let mut input_idx: BTreeMap<String, usize> = BTreeMap::new();
    let mut inputs: Vec<GraphInput> = Vec::new();
    for seg in edl.segments.iter().filter(|s| s.asset.is_some()) {
        if !segment_video_track_visible(project, seg) {
            continue;
        }
        let asset_id = seg.asset.as_deref().unwrap();
        add_graph_asset_input(
            project,
            edl,
            project_dir,
            use_proxy,
            asset_id,
            seg.clip_id.as_deref(),
            &mut inputs,
            &mut input_idx,
        )?;
    }
    // Matte alpha mattes (edit.matte): each matted segment needs its baked alpha
    // as a PARALLEL input so the overlay chain can alphamerge it onto the clip.
    // Keyed by the cache path so clips sharing one baked matte share one `-i`.
    // The alpha is baked by edit.matte (content-addressed); a missing file is a
    // clear error, never a silent un-matted render.
    if with_video {
        // A replacement may use an imported image/video that is not otherwise
        // placed on the timeline. It is still a normal project asset and follows
        // the same path/proxy/loop rules as every other graph input.
        for seg in edl.segments.iter().filter(|s| {
            s.matte.is_some() && s.asset.is_some() && segment_video_track_visible(project, s)
        }) {
            if let Some(matte::ReplacementBackground::Asset(asset_id)) =
                matte::replacement_background(seg.matte.as_ref())?
            {
                add_graph_asset_input(
                    project,
                    edl,
                    project_dir,
                    use_proxy,
                    asset_id,
                    seg.clip_id.as_deref(),
                    &mut inputs,
                    &mut input_idx,
                )?;
            }
        }
        for seg in edl.segments.iter().filter(|s| {
            s.matte.is_some() && s.asset.is_some() && segment_video_track_visible(project, s)
        }) {
            let m = seg.matte.as_ref().unwrap();
            let asset_id = seg.asset.as_deref().unwrap();
            let asset = project.assets.get(asset_id).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("asset {asset_id} referenced by a matte segment does not exist"),
                    "EDL references an asset id missing from project.assets",
                )
            })?;
            let alpha_path = matte_alpha_path(project_dir, &asset.hash, m)?;
            let key = matte_input_key(&alpha_path);
            if input_idx.contains_key(&key) {
                continue;
            }
            if !super::plain_internal_file_exists(&alpha_path)? {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!(
                        "baked matte alpha missing for clip {}: {}",
                        seg.clip_id.as_deref().unwrap_or(asset_id),
                        alpha_path.display()
                    ),
                    "the matte alpha is baked by edit.matte (content-addressed); it is absent",
                )
                .with_suggested_action(
                    "re-apply edit.matte on the clip to bake its alpha (the matting sidecar must be reachable), then render",
                ));
            }
            let matte_dir = alpha_path.parent().expect("matte alpha has a parent");
            super::require_plain_internal_dir(
                matte_dir.parent().expect("matte cache has a parent"),
            )?;
            super::require_plain_internal_dir(matte_dir)?;
            input_idx.insert(key, inputs.len());
            inputs.push(GraphInput {
                path: alpha_path,
                image: false,
                gpu_decode: false,
            });
        }
        // Vector/freeform masks (edit.add_mask): each masked segment needs its baked
        // GRAY alpha PNG as a PARALLEL input so the chain can maskedmerge the region
        // effect. Unlike mattes (baked by a slow sidecar), a mask is rasterized HERE
        // (resvg, ~ms) and cached content-addressed — so it bakes on first render and
        // reuses thereafter. Keyed by the cache path → clips sharing one mask share `-i`.
        let (mw, mh) = (output_w, output_h);
        for seg in edl.segments.iter().filter(|s| {
            // A TRACKED rect/ellipse mask paints its alpha procedurally (geq) — no PNG
            // to bake. Only STATIC (or polygon) masks need a baked shape input here.
            s.mask.as_ref().is_some_and(|m| !mask_uses_geq(m))
                && s.asset.is_some()
                && segment_video_track_visible(project, s)
        }) {
            let m = seg.mask.as_ref().unwrap();
            let alpha_path = mask_alpha_path(project_dir, m, mw, mh);
            let key = mask_input_key(&alpha_path);
            if input_idx.contains_key(&key) {
                continue;
            }
            bake_mask_png_atomic(m, mw, mh, &alpha_path)?;
            input_idx.insert(key, inputs.len());
            inputs.push(GraphInput {
                path: alpha_path,
                image: true, // a single still PNG (looped at the input like other stills)
                gpu_decode: false,
            });
        }
        // Power windows (edit.grade_window): each window needs its baked shape-alpha PNG as a
        // PARALLEL input so the chain can alphamerge the GRADED copy into the region. The
        // alpha is rasterized exactly like a mask (resvg, ~ms) and cached content-addressed by
        // the window geometry — identical window shapes (within a clip, across clips) share one
        // `-i`. Windows are STATIC (no geq path), so every window bakes a PNG here.
        for seg in edl.segments.iter().filter(|s| {
            !s.grade_windows.is_empty()
                && s.asset.is_some()
                && segment_video_track_visible(project, s)
        }) {
            for gw in &seg.grade_windows {
                let alpha_path = window_alpha_path(project_dir, &gw.window, mw, mh);
                let key = window_input_key(&alpha_path);
                if input_idx.contains_key(&key) {
                    continue;
                }
                bake_mask_png_atomic(&gw.window.to_mask(), mw, mh, &alpha_path)?;
                input_idx.insert(key, inputs.len());
                inputs.push(GraphInput {
                    path: alpha_path,
                    image: true, // a single still PNG (looped at the input like other stills)
                    gpu_decode: false,
                });
            }
        }
    }
    Ok((inputs, input_idx))
}
