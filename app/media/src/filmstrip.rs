//! filmstrip.rs — horizontal thumbnail strip per asset for the timeline.
//!
//! Role: from an asset's PROXY (the 960×540 editing copy), render a single tiled
//! JPEG of N evenly-spaced frames over `[0, duration]`. The timeline clip body
//! uses it as a CSS background sliced to the clip's `[src_in, src_out]` sub-range,
//! so each video clip shows its own frames directly in the timeline.
//!
//! The UI maps source time → strip x by the LINEAR fraction `t / duration`, so no
//! per-frame metadata is needed — only the strip path. A ±1-thumbnail slop at the
//! tail (from fps rounding) is invisible at timeline scale.
//!
//! Callers: the server import chain + the `media.filmstrip` verb. Deps: ffmpeg.

use cut_core::CutError;
use std::path::{Path, PathBuf};

/// Thumbnail height in px (16:9 source → ~128 px wide). Small: decorative.
const THUMB_H: u32 = 72;
/// Frame-count bounds. Sampled at ~2 frames/second so zooming IN reveals more
/// real frames instead of stretching a sparse set (reported: "zooming stretches
/// the thumbnails, doesn't split into frames"). Capped so a long clip's strip
/// stays a sane pixel width (160 × ~128 ≈ 20k px, JPEG-compressed).
const MIN_FRAMES: u32 = 12;
const MAX_FRAMES: u32 = 160;
const FRAMES_PER_SEC: u32 = 2;

/// Frame count for a clip of `duration_ms` (≈2/s, clamped to [MIN, MAX]).
fn frame_count(duration_ms: u64) -> u32 {
    ((duration_ms * FRAMES_PER_SEC as u64 / 1000) as u32).clamp(MIN_FRAMES, MAX_FRAMES)
}

/// Render `<out_dir>/<asset_id>.jpg`: N frames from `proxy`, tiled left→right,
/// evenly spaced over the clip duration. Idempotent — an existing strip is
/// returned as-is (callers cache by asset hash, so a present strip is valid).
pub fn make_filmstrip(
    proxy: &Path,
    out_dir: &Path,
    asset_id: &str,
    duration_ms: u64,
) -> Result<PathBuf, CutError> {
    let out = out_dir.join(format!("{asset_id}.jpg"));
    if out.exists() {
        if crate::image_cache::existing_image_cache_is_complete(&out) {
            return Ok(out);
        }
        let _ = std::fs::remove_file(&out);
    }
    require_live_output_dir(out_dir)?;

    let dur_s = (duration_ms as f64 / 1000.0).max(0.1);
    let n = frame_count(duration_ms);
    // Sample N frames evenly across the duration, scale to THUMB_H (width auto,
    // kept even for yuv420), then pack into one N×1 row. A tiny oversample
    // (n + 0.5) guarantees the tile fills even when fps rounding would otherwise
    // yield N-1 frames; the extra frame spills into a second (discarded) tile.
    let fps = (n as f64 + 0.5) / dur_s;
    let vf = format!("fps={fps:.6},scale=-2:{THUMB_H},tile={n}x1");
    let args: Vec<String> = vec![
        "-i".into(),
        proxy.display().to_string(),
        "-vf".into(),
        vf,
        "-frames:v".into(),
        "1".into(),
        "-q:v".into(),
        "5".into(), // mid JPEG quality — small file, fine for thumbnails
        out.display().to_string(),
    ];
    crate::ffmpeg::run_ffmpeg_atomic_output(&args, &out)?;
    Ok(out)
}

/// Cache filename for a windowed thumbnail tile — encodes the full request so
/// the on-disk cache is keyed by (asset, window, count, height). Pure (no IO) so
/// it is unit-testable; `make_window_thumbs` and any cache pre-check agree on it.
fn window_thumb_name(asset_id: &str, start_ms: u64, end_ms: u64, count: u32, h: u32) -> String {
    format!("{asset_id}_w{start_ms}-{end_ms}_{count}x{h}.jpg")
}

/// Render a WINDOWED thumbnail tile (the ZOOM path): `count` frames evenly
/// sampled across the proxy sub-range `[start_ms, end_ms)`, scaled to height `h`,
/// tiled into one `count`×1 JPEG.
///
/// Why this exists: the whole-asset `make_filmstrip` strip is fixed-density, so
/// zooming in just stretches it. Instead the UI requests exactly the frames
/// VISIBLE for a clip at the current zoom (≈1 thumb per fixed pixel width). As you
/// zoom in the visible window SHRINKS, so the same ~`count` frames now cover less
/// time → higher effective density, approaching per-frame at sub-second zoom —
/// Timeline-editor behavior. Cost is BOUNDED because you only ever sample the visible
/// window, keeping work bounded across zoom bands.
///
/// Input-seek (`-ss` before `-i`) keeps it on ffmpeg's fast path. The filename
/// encodes the request, so identical windows (pan/zoom jitter, re-renders) hit
/// the disk cache. Idempotent. Ephemeral — NOT stored on the asset (unlike the
/// base strip); the UI caches the URL in memory and these tiles live in the same
/// `filmstrip/` dir, served by the existing `/filmstrip/:file` route.
#[allow(clippy::too_many_arguments)]
pub fn make_window_thumbs(
    proxy: &Path,
    out_dir: &Path,
    asset_id: &str,
    start_ms: u64,
    end_ms: u64,
    count: u32,
    h: u32,
) -> Result<PathBuf, CutError> {
    let count = count.clamp(MIN_FRAMES, MAX_FRAMES);
    let h = h.clamp(24, 240);
    let out = out_dir.join(window_thumb_name(asset_id, start_ms, end_ms, count, h));
    if out.exists() {
        if crate::image_cache::existing_image_cache_is_complete(&out) {
            return Ok(out);
        }
        let _ = std::fs::remove_file(&out);
    }
    require_live_output_dir(out_dir)?;

    let start_s = start_ms as f64 / 1000.0;
    // Guard a degenerate/inverted window to a small positive span so fps is finite.
    let win_s = ((end_ms.saturating_sub(start_ms)) as f64 / 1000.0).max(0.05);
    // Same +0.5 oversample trick as make_filmstrip so fps rounding can't yield a
    // short (N-1) row; the spare frame spills into a discarded second tile. When
    // `count` exceeds the real frames in the window (deep surgical zoom), the fps
    // filter duplicates frames to fill — i.e. you see genuine per-frame detail.
    let fps = (count as f64 + 0.5) / win_s;
    let vf = format!("fps={fps:.6},scale=-2:{h},tile={count}x1");
    let args: Vec<String> = vec![
        "-ss".into(),
        format!("{start_s:.3}"),
        "-t".into(),
        format!("{win_s:.3}"),
        "-i".into(),
        proxy.display().to_string(),
        "-vf".into(),
        vf,
        "-frames:v".into(),
        "1".into(),
        "-q:v".into(),
        "5".into(),
        out.display().to_string(),
    ];
    crate::ffmpeg::run_ffmpeg_atomic_output(&args, &out)?;
    Ok(out)
}

/// Render `<out_dir>/<asset_id>.jpg`: a single THUMB_H-tall thumbnail of a STILL
/// IMAGE asset (which has no proxy/duration, so make_filmstrip can't run). The
/// timeline tiles it across the image clip so the picture is visible, not just a
/// photo icon ("don't see images in the timeline"). Stored in the same field +
/// dir as the video filmstrip. Idempotent.
pub fn make_image_thumb(src: &Path, out_dir: &Path, asset_id: &str) -> Result<PathBuf, CutError> {
    let out = out_dir.join(format!("{asset_id}.jpg"));
    if out.exists() {
        if crate::image_cache::existing_image_cache_is_complete(&out) {
            return Ok(out);
        }
        let _ = std::fs::remove_file(&out);
    }
    require_live_output_dir(out_dir)?;
    let args: Vec<String> = vec![
        "-i".into(),
        src.display().to_string(),
        "-vf".into(),
        format!("scale=-2:{THUMB_H}"),
        "-frames:v".into(),
        "1".into(),
        "-q:v".into(),
        "5".into(),
        out.display().to_string(),
    ];
    crate::ffmpeg::run_ffmpeg_atomic_output(&args, &out)?;
    Ok(out)
}

fn require_live_output_dir(dir: &Path) -> Result<(), CutError> {
    if dir.is_dir() {
        return Ok(());
    }
    Err(CutError::new(
        cut_core::error::codes::IO,
        "could not generate thumbnails because the project is no longer open",
        format!(
            "the project filmstrip directory no longer exists: {}",
            dir.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_count_clamps() {
        assert_eq!(frame_count(0), MIN_FRAMES); // sub-second → floor
        assert_eq!(frame_count(3_000), MIN_FRAMES); // 3s × 2 = 6, clamped up to 12
        assert_eq!(frame_count(20_000), 40); // 20s × 2 = 40
        assert_eq!(frame_count(600_000), MAX_FRAMES); // 10min × 2 way over → cap 160
    }

    #[test]
    fn window_thumb_name_encodes_request() {
        // The cache key is the filename — two distinct windows must not collide,
        // and an identical request must reproduce the same name (cache hit).
        let a = window_thumb_name("a1", 0, 10_000, 12, 80);
        let b = window_thumb_name("a1", 5_000, 5_500, 12, 80);
        assert_eq!(a, "a1_w0-10000_12x80.jpg");
        assert_ne!(a, b); // different window → different cache file
        assert_eq!(a, window_thumb_name("a1", 0, 10_000, 12, 80)); // stable
    }

    #[test]
    fn incomplete_existing_filmstrip_is_not_a_cache_hit() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("asset1.jpg");
        std::fs::write(&out, [0xff, 0xd8]).unwrap();
        let err = make_filmstrip(
            Path::new("/no/such/source.mp4"),
            dir.path(),
            "asset1",
            1_000,
        )
        .unwrap_err();
        assert!(
            err.message.contains("ffmpeg") || err.message.contains("No such"),
            "stale partial jpeg should be rejected and rerender attempted: {err:?}"
        );
    }

    #[test]
    fn late_filmstrip_never_recreates_a_deleted_project() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("deleted.cutproj");
        let filmstrip = project.join("filmstrip");

        let error = make_image_thumb(Path::new("source.png"), &filmstrip, "a1")
            .expect_err("a late thumbnail must be rejected after project deletion");

        assert_eq!(error.code, cut_core::error::codes::IO);
        assert!(!project.exists(), "the deleted project must stay deleted");
    }
}
