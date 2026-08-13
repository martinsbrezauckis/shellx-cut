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

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

/// Proxy geometry — fixed by the current media contract (media-engine contract).
pub const PROXY_WIDTH: u32 = 960;
pub const PROXY_HEIGHT: u32 = 540;

/// Target proxy GOP length in FRAMES. At 30 fps this is
/// a ~1 s keyframe interval, so an input-side `-ss` seek decodes at most ~1 s
/// of frames before reaching the requested time. libx264's default keyint is
/// 250 frames (~4–8 s depending on source fps) — far too long for scrubbing.
///
/// DISK TRADEOFF: more keyframes imply a larger proxy, but proxies are
/// regenerable project-local cache files. The shorter interval favors
/// responsive scrubbing over minimizing this disposable cache.
pub const PROXY_GOP_FRAMES: u32 = 30;

/// Transcode `src` into `<proxies_dir>/<asset_id>.mp4`: 960×540 short-GOP h264
/// (preset fast, ~1 s keyframe interval) + passthrough-rate AAC audio. Returns
/// the proxy path. Reuses only an existing, ffprobe-valid 960×540 video proxy;
/// an interrupted or corrupt cache entry is removed and regenerated. Existing
/// long-GOP proxies are NOT regenerated (a valid existing file wins);
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
    make_proxy_with(src, proxies_dir, asset_id, |args, out| {
        crate::ffmpeg::run_ffmpeg_validated_atomic_output(args, out, validate_proxy_output)
    })
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
    if out.exists() && validate_proxy_output(&out).is_ok() {
        on_progress(1.0);
        return Ok(out);
    }
    make_proxy_with(src, proxies_dir, asset_id, |args, out| {
        if total_ms > 0 {
            crate::ffmpeg::run_ffmpeg_with_progress_validated_atomic_output(
                args,
                out,
                total_ms,
                on_progress,
                validate_proxy_output,
            )
        } else {
            crate::ffmpeg::run_ffmpeg_validated_atomic_output(args, out, validate_proxy_output)
        }
    })
}

/// The proxy cache is only reusable when it is a complete, regular 960×540
/// video. `ffprobe` rejects interrupted MP4s (including nonempty files missing
/// their `moov` atom), so they cannot silently become permanent cache hits.
fn validate_proxy_output(path: &Path) -> Result<(), CutError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("proxy output is not a regular file: {}", path.display()),
            "proxy cache entries must be completed regular files",
        ));
    }
    let probe = crate::probe(path)?;
    if probe.kind != crate::probe::kinds::VIDEO
        || probe.width != Some(PROXY_WIDTH)
        || probe.height != Some(PROXY_HEIGHT)
    {
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("proxy output has unexpected media shape: {}", path.display()),
            format!(
                "expected a {PROXY_WIDTH}x{PROXY_HEIGHT} video proxy; ffprobe reported kind={}, width={:?}, height={:?}",
                probe.kind, probe.width, probe.height
            ),
        ));
    }
    Ok(())
}

fn make_proxy_with(
    src: &Path,
    proxies_dir: &Path,
    asset_id: &str,
    encode: impl FnOnce(&[String], &Path) -> Result<(), CutError>,
) -> Result<PathBuf, CutError> {
    let out = proxies_dir.join(format!("{asset_id}.mp4"));
    if out.exists() && validate_proxy_output(&out).is_ok() {
        return Ok(out);
    }
    require_live_output_dir(proxies_dir)?;
    if out.exists() {
        // This is a project-local regenerable cache file. Do not let a failed
        // encode leave a known-invalid final path for later cache lookups.
        std::fs::remove_file(&out)?;
    }
    encode(&proxy_ffmpeg_args(src, &out), &out)?;
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
    use cut_core::error_codes;

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

    #[test]
    fn invalid_cached_proxy_is_not_reused() {
        let root = tempfile::tempdir().unwrap();
        let proxies = root.path().join("proxies");
        std::fs::create_dir(&proxies).unwrap();
        let out = proxies.join("a1.mp4");
        std::fs::write(&out, b"nonempty partial MP4 with no moov atom").unwrap();

        let returned = make_proxy_with(Path::new("source.mp4"), &proxies, "a1", |args, out| {
            crate::atomic_output::run_with_atomic_output(args, out, |tmp_args| {
                let tmp = Path::new(tmp_args.last().unwrap());
                assert_ne!(tmp, out, "ffmpeg must write a sibling temp file");
                assert!(
                    !out.exists(),
                    "the invalid cache entry must be removed before regeneration"
                );
                std::fs::write(tmp, b"replacement bytes").unwrap();
                Ok(())
            })
        })
        .expect("a corrupt cache must be regenerated, not reused");

        assert_eq!(returned, out);
        assert_eq!(std::fs::read(&out).unwrap(), b"replacement bytes");
        assert_eq!(std::fs::read_dir(&proxies).unwrap().count(), 1);
    }

    #[test]
    fn simulated_sigsegv_never_leaves_a_final_proxy() {
        let root = tempfile::tempdir().unwrap();
        let proxies = root.path().join("proxies");
        std::fs::create_dir(&proxies).unwrap();
        let out = proxies.join("a1.mp4");

        let error = make_proxy_with(Path::new("source.mp4"), &proxies, "a1", |args, out| {
            crate::atomic_output::run_with_atomic_output(args, out, |tmp_args| {
                std::fs::write(tmp_args.last().unwrap(), b"partial bytes").unwrap();
                Err(CutError::new(
                    error_codes::FFMPEG,
                    "ffmpeg exited with signal: 11 (SIGSEGV)",
                    "simulated child crash after writing a partial proxy",
                ))
            })
        })
        .expect_err("a crashed ffmpeg child must fail the proxy job");

        assert_eq!(error.code, error_codes::FFMPEG);
        assert!(
            !out.exists(),
            "a crashed proxy encode must leave source fallback with no final cache file"
        );
        assert_eq!(std::fs::read_dir(&proxies).unwrap().count(), 0);
    }
}
