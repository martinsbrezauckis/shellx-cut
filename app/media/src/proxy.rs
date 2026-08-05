//! proxy.rs — proxy generation (media-engine contract "proxy → 960×540 h264 + same audio").
//!
//! Role: build the lightweight edit proxy the UI <video> element plays AND the
//! scrub-frame source path serves. A long-GOP proxy is fine for
//! linear <video> playback but is the scrub-latency killer: a fast input-seek
//! (`-ss <pos> -i proxy`) must decode from the nearest keyframe forward, so a
//! 4 s GOP means up to 4 s of frames decoded per scrub. The proxy GOP is capped
//! at ~1 s (short-GOP, `-g 30 -keyint_min 30 -sc_threshold 0`) so a fast seek
//! lands within ≤1 s of decode — the difference between an ~8 s scrub and a
//! sub-100 ms one on supported validation media.
//!
//! Dependencies: ffmpeg.rs. Primary callers: server media.import job chain,
//! render::extract_scrub_frame (the fast scrub path).

use cut_core::CutError;
use std::path::{Path, PathBuf};

/// Proxy geometry — fixed by the current media contract (media-engine contract).
pub const PROXY_WIDTH: u32 = 960;
pub const PROXY_HEIGHT: u32 = 540;

/// Target proxy GOP length in FRAMES. At 30 fps this is
/// a ~1 s keyframe interval, so an input-side `-ss` seek decodes at most ~1 s
/// of frames before reaching the requested time. libx264's default keyint is
/// 250 frames (~4–8 s depending on source fps) — far too long for scrubbing.
///
/// DISK TRADEOFF: more keyframes imply a larger proxy.
/// Measured on the 4K canvas-demo-01 asset the short-GOP proxy is ~1.6× the
/// long-GOP proxy (still ≈1–2 MB for a 54 s clip — negligible next to the
/// 25 MB source). Proxies live in `<proj>/proxies/` (gitignored, regenerable),
/// so the cost is local scratch only and is repaid many times over by every
/// scrub the editor does. Scrub responsiveness >> a megabyte of scratch.
pub const PROXY_GOP_FRAMES: u32 = 30;

/// Transcode `src` into `<proxies_dir>/<asset_id>.mp4`: 960×540 short-GOP h264
/// (preset fast, ~1 s keyframe interval) + passthrough-rate AAC audio. Returns
/// the proxy path. Skips (returns existing path) when the proxy already exists
/// — callers cache by asset hash, so an existing file is always valid. OLD
/// Existing long-GOP proxies are NOT regenerated (an existing file wins);
/// they still scrub correctly, just with more decode per seek — graceful, never
/// a crash. Re-importing the asset into a fresh project gets the short-GOP proxy.
/// Build the ffmpeg args (without global flags) that transcode `src` → the
/// 960×540 editing proxy at `out`. Shared by [`make_proxy`] and
/// [`make_proxy_with_progress`] so the encode is byte-identical either way.
fn proxy_ffmpeg_args(src: &Path, out: &Path) -> Vec<String> {
    // Scale into a 960×540 box preserving aspect, pad to EXACTLY 960×540
    // (UI <video> relies on fixed proxy geometry); audio → AAC 128k.
    // Determinism flags applied so re-import on the same source is hashable.
    let g = PROXY_GOP_FRAMES.to_string();
    let mut args: Vec<String> = vec![
        "-i".into(),
        src.display().to_string(),
        "-vf".into(),
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1",
            w = PROXY_WIDTH,
            h = PROXY_HEIGHT
        ),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "fast".into(),
        "-crf".into(),
        "23".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        // Short GOP for scrub: a fixed 30-frame keyframe interval, no
        // scene-cut keyframes (sc_threshold=0) so the interval is uniform and
        // a fast seek's decode cost is bounded regardless of content.
        "-g".into(),
        g.clone(),
        "-keyint_min".into(),
        g,
        "-sc_threshold".into(),
        "0".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "128k".into(),
    ];
    args.extend(
        crate::ffmpeg::DETERMINISM_FLAGS
            .iter()
            .map(|s| s.to_string()),
    );
    args.push(out.display().to_string());
    args
}

pub fn make_proxy(src: &Path, proxies_dir: &Path, asset_id: &str) -> Result<PathBuf, CutError> {
    let out = proxies_dir.join(format!("{asset_id}.mp4"));
    if out.exists() {
        // Callers cache by asset hash → an existing proxy is always valid.
        return Ok(out);
    }
    require_live_output_dir(proxies_dir)?;
    crate::ffmpeg::run_ffmpeg(&proxy_ffmpeg_args(src, &out))?;
    Ok(out)
}

/// Progress-reporting [`make_proxy`]: maps the encode against `total_ms` (the
/// source duration) into `on_progress(0.0..=1.0)`. Used by the BACKGROUND proxy
/// job so the proxy transcode of a long file shows live progress instead
/// of a frozen number so encoding progress stays visible. With
/// `total_ms == 0` it runs without progress (equivalent to [`make_proxy`]).
pub fn make_proxy_with_progress(
    src: &Path,
    proxies_dir: &Path,
    asset_id: &str,
    total_ms: u64,
    on_progress: &dyn Fn(f32),
) -> Result<PathBuf, CutError> {
    let out = proxies_dir.join(format!("{asset_id}.mp4"));
    if out.exists() {
        on_progress(1.0);
        return Ok(out);
    }
    require_live_output_dir(proxies_dir)?;
    let args = proxy_ffmpeg_args(src, &out);
    if total_ms > 0 {
        crate::ffmpeg::run_ffmpeg_with_progress(&args, total_ms, on_progress)?;
    } else {
        crate::ffmpeg::run_ffmpeg(&args)?;
    }
    Ok(out)
}

fn require_live_output_dir(dir: &Path) -> Result<(), CutError> {
    if dir.is_dir() {
        return Ok(());
    }
    Err(CutError::new(
        cut_core::error::codes::IO,
        "could not generate a proxy because the project is no longer open",
        format!(
            "the project proxy directory no longer exists: {}",
            dir.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_proxy_never_recreates_a_deleted_project() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("deleted.cutproj");
        let proxies = project.join("proxies");

        let error = make_proxy(Path::new("source.mp4"), &proxies, "a1")
            .expect_err("a late proxy must be rejected after project deletion");

        assert_eq!(error.code, cut_core::error::codes::IO);
        assert!(!project.exists(), "the deleted project must stay deleted");
    }
}
