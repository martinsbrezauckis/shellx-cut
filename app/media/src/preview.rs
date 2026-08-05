//! preview.rs — incremental segment-render cache for draft preview.
//!
//! Role: a DRAFT preview of the WHOLE current timeline that re-renders only the
//! base-track segments whose inputs changed since the last preview, reuses
//! cached segment files for the rest, and stitches them with the concat demuxer.
//! This is the "edit one clip in a 10-clip timeline, only the touched segment
//! re-renders" path — the difference between a ~40 s full draft render and a
//! ~few-second incremental preview after a single edit.
//!
//! WHAT IT IS NOT: this is DERIVED state for fast feedback, never an op input
//! and never a receipt input — receipts stay render.final (public verb contract). The
//! preview is proxy-grade (960×540, draft encode, from the asset PROXY not the
//! 4K source) and is deliberately approximate: it composites only the BASE
//! video track + its audio (the editorial AV the human scrubs). Overlay/PiP
//! compositing and caption burn-in are NOT in the incremental preview — when a
//! frame-exact composed view is needed the agent uses render.frame (compose=1)
//! or render.final. Documented honestly so nobody mistakes the preview for a
//! proof.
//!
//! CACHE KEY (per base-track media segment): a content hash over everything
//! that changes the rendered bytes of THAT segment — asset content hash,
//! source in/out, fit, crop, fade, gain, and the proxy/output geometry+fps.
//! Anything the segment's pixels/samples depend on is in the key; a segment
//! whose inputs are unchanged hashes the same and its cached file is reused.
//!
//! Dependencies: ffmpeg.rs (run_ffmpeg), cut-core (Project/Edl). Primary
//! caller: server render.preview{draft:true} (dispatch.rs).

use crate::ffmpeg::{concat_demuxer_file_line, run_ffmpeg, DETERMINISM_FLAGS};
use crate::render::RenderPreset;
use cut_core::{error_codes, CutError, Edl, Project};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Format ms as fractional seconds for filter args ("1.500").
fn secs(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// Outcome of an incremental preview build — what dispatch returns and what the
/// cache-correctness evidence reads. `segments_rendered` + `segments_reused`
/// sum to the base-track media-segment count; the named lists let the proof
/// point at exactly which segment re-rendered after a single-clip edit.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewResult {
    /// The stitched preview file (proxy-grade mp4) in the preview cache dir.
    pub path: PathBuf,
    /// How many base-track segments were (re)rendered this build.
    pub segments_rendered: usize,
    /// How many base-track segments were reused from cache.
    pub segments_reused: usize,
    /// The cache filenames (seg_<hash>.mp4) that were freshly rendered.
    pub rendered: Vec<String>,
    /// The cache filenames that were reused from cache.
    pub reused: Vec<String>,
    /// Total composition duration (ms) the preview covers.
    pub duration_ms: u64,
}

/// One base-track segment resolved for incremental rendering: its ordinal on
/// the timeline, the source proxy to render from, the source range, the fit/
/// crop/fade/gain that shape it, and the cache file its content hashes to.
struct SegPlan {
    cache_name: String,
    cache_path: PathBuf,
    /// For VIDEO segments: the asset PROXY to fast-seek. For STILL-IMAGE
    /// segments: the source image itself (stills have no proxy — see the still-image preview contract).
    /// Empty path = a GAP segment (black + silence).
    proxy_path: PathBuf,
    /// True when this segment is a still image: render with `-loop 1` from the
    /// source image and synthesize silent audio (stills carry no audio and no
    /// proxy). This is what makes draft preview work on timelines with
    /// intro/outro cards (the still-image preview contract) — a still is a trivially-cacheable conform.
    image: bool,
    src_in_ms: u64,
    dur_ms: u64,
    crop: Option<cut_core::ClipCrop>,
    fade: Option<cut_core::ClipFade>,
    gain_db: f64,
}

/// Cache filename for a MEDIA segment: a content hash over everything that
/// changes the segment's rendered bytes — asset content hash, source in/out,
/// output geometry+fps, gain, and crop/fade (folded in via their JSON, so None
/// vs Some and any field change both move the hash). Two segments with
/// identical inputs hash identically → the second reuses the first's cache file;
/// any input change (e.g. trimming src_out) yields a NEW filename, so the old
/// cached file is left untouched and the changed segment re-renders. This is the
/// whole basis of "edit one clip, only that segment re-renders".
fn media_segment_cache_name(
    asset_hash: &str,
    src_in: u64,
    src_out: u64,
    geom: &str,
    fps: f64,
    gain_db: f64,
    crop: Option<&cut_core::ClipCrop>,
    fade: Option<&cut_core::ClipFade>,
    // For STILL-IMAGE segments (the still-image preview contract): the clip duration the still is looped
    // to. A still's src_in/src_out are timeline-positioning artifacts, not a
    // source byte range, so duration is what actually changes the rendered
    // bytes — fold it in so two cards of different lengths from the SAME image
    // get distinct cache files. `None` for video (src_in/out already capture
    // the byte range there).
    still_dur_ms: Option<u64>,
) -> String {
    let mut h = Sha256::new();
    h.update(b"segv1");
    h.update(asset_hash.as_bytes());
    h.update(src_in.to_le_bytes());
    h.update(src_out.to_le_bytes());
    h.update(geom.as_bytes());
    h.update(fps.to_le_bytes());
    h.update(gain_db.to_le_bytes());
    h.update(serde_json::to_string(&crop).unwrap_or_default().as_bytes());
    h.update(serde_json::to_string(&fade).unwrap_or_default().as_bytes());
    // Distinct domain tag + value so a still NEVER collides with a video
    // segment that happens to share the other inputs.
    h.update(b"still");
    h.update(
        serde_json::to_string(&still_dur_ms)
            .unwrap_or_default()
            .as_bytes(),
    );
    let hash = format!("{:x}", h.finalize());
    format!("seg_{}.mp4", &hash[..16])
}

/// Cache filename for a GAP segment (black + silence): hashed by duration +
/// geometry + fps. Identical gaps share a cache file.
fn gap_segment_cache_name(dur_ms: u64, geom: &str, fps: f64) -> String {
    let mut h = Sha256::new();
    h.update(b"gapv1");
    h.update(dur_ms.to_le_bytes());
    h.update(geom.as_bytes());
    h.update(fps.to_le_bytes());
    let hash = format!("{:x}", h.finalize());
    format!("seg_{}.mp4", &hash[..16])
}

/// Render an incremental DRAFT preview of the whole timeline.
///
/// Walks the BASE video track's media segments, content-hashes each, renders
/// only the segments whose cache file is absent (changed/new inputs), reuses
/// the rest, then concat-demuxes the ordered segment files into one preview
/// mp4 at `<cache_dir>/preview.mp4`. Gaps on the base track render as black
/// segments (also cached by their hash). `preset` is the draft tier by
/// convention (caller passes RenderPreset::named("draft")).
///
/// AUDIO: each segment carries its own audio (from the proxy, gain/fade
/// applied) so the concat preview has continuous sound. Only the base track's
/// audio is in the preview — separate audio tracks (music beds, etc.) are a
/// final-render concern, not a scrub-feedback one (documented approximation).
///
/// Returns the PreviewResult naming exactly which segments rendered vs reused.
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
    std::fs::create_dir_all(cache_dir)?;
    let base = edl.base_video_track().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "no base video track to preview",
            "the timeline has no video segments",
        )
    })?;

    // Resolve every base-track segment to a render plan + its cache key.
    let geom = format!(
        "{}x{}",
        crate::proxy::PROXY_WIDTH,
        crate::proxy::PROXY_HEIGHT
    );
    let fps = project.settings.fps;
    let mut plans: Vec<SegPlan> = Vec::new();
    for seg in edl.track_segments(base) {
        let dur = seg.timeline_out_ms - seg.timeline_in_ms;
        match (&seg.asset, seg.src_in_ms, seg.src_out_ms) {
            (Some(asset_id), Some(src_in), Some(src_out)) => {
                let asset = project.assets.get(asset_id).ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        format!("asset {asset_id} referenced by the timeline does not exist"),
                        "EDL references an asset id missing from project.assets",
                    )
                })?;
                // Still images never get a proxy (their import chain stops after
                // probe by design), so a timeline with an intro/outro card used
                // to refuse the WHOLE draft preview (the still-image preview contract). A still is instead a
                // trivially-cacheable conform: render it directly from the source
                // image with `-loop 1` for the clip duration — no proxy needed.
                let is_image = asset
                    .probe
                    .as_ref()
                    .and_then(|p| p.get("kind"))
                    .and_then(|k| k.as_str())
                    == Some("image");
                let src_path = if is_image {
                    // Render from the source image (stills have no proxy).
                    let mut p = PathBuf::from(&asset.path);
                    if p.is_relative() {
                        p = project_dir.join(p);
                    }
                    p
                } else {
                    // VIDEO: render from the PROXY (fast). No proxy yet → this
                    // path can't be fast; surface an actionable error so the
                    // caller falls back to the full render.preview rather than
                    // silently rendering the 4K source.
                    let proxy_rel = asset.proxy.as_ref().ok_or_else(|| {
                        CutError::new(
                            error_codes::INVALID_ARGS,
                            format!("asset {asset_id} has no proxy yet — incremental preview needs proxies"),
                            "the import proxy step has not completed for this asset",
                        )
                        .with_suggested_action("wait for the import job chain, or use render.preview without draft")
                    })?;
                    let mut proxy_path = PathBuf::from(proxy_rel);
                    if proxy_path.is_relative() {
                        proxy_path = project_dir.join(proxy_path);
                    }
                    proxy_path
                };
                // Content hash over everything that changes this segment's bytes.
                // For a still the cache key also folds in the duration (the still
                // is looped to dur_ms) so two cards of different lengths from the
                // same image get distinct cache files.
                let cache_name = media_segment_cache_name(
                    &asset.hash,
                    src_in,
                    src_out,
                    &geom,
                    fps,
                    seg.gain_db,
                    seg.crop.as_ref(),
                    seg.fade.as_ref(),
                    if is_image { Some(dur) } else { None },
                );
                let cache_path = cache_dir.join(&cache_name);
                plans.push(SegPlan {
                    cache_name,
                    cache_path,
                    proxy_path: src_path,
                    image: is_image,
                    src_in_ms: src_in,
                    dur_ms: dur,
                    crop: seg.crop.clone(),
                    fade: seg.fade.clone(),
                    gain_db: seg.gain_db,
                });
            }
            _ => {
                // Gap on the base track → black + silence, cached by duration.
                let cache_name = gap_segment_cache_name(dur, &geom, fps);
                let cache_path = cache_dir.join(&cache_name);
                plans.push(SegPlan {
                    cache_name,
                    cache_path,
                    proxy_path: PathBuf::new(), // empty = gap (black) segment
                    image: false,
                    src_in_ms: 0,
                    dur_ms: dur,
                    crop: None,
                    fade: None,
                    gain_db: 0.0,
                });
            }
        }
    }

    // Render the segments whose cache file is absent; reuse the rest.
    let mut rendered = Vec::new();
    let mut reused = Vec::new();
    for p in &plans {
        if p.cache_path.exists() {
            reused.push(p.cache_name.clone());
            continue;
        }
        render_one_segment(p, fps, preset)?;
        rendered.push(p.cache_name.clone());
    }

    // Stitch with the concat demuxer (stream-copy — no re-encode of the cached
    // segments, so stitching is near-instant and the cached bytes are reused
    // verbatim). All segments share codec/geometry/fps by construction.
    let list_path = cache_dir.join("concat.txt");
    let mut list = String::new();
    for p in &plans {
        // concat demuxer needs absolute paths, single-quoted.
        writeln!(list, "{}", concat_demuxer_file_line(&p.cache_path)).unwrap();
    }
    std::fs::write(&list_path, &list)?;
    let out = cache_dir.join("preview.mp4");
    let mut args: Vec<String> = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_path.display().to_string(),
        "-c".into(),
        "copy".into(),
    ];
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out.display().to_string());
    run_ffmpeg(&args)?;

    Ok(PreviewResult {
        path: out,
        segments_rendered: rendered.len(),
        segments_reused: reused.len(),
        rendered,
        reused,
        duration_ms: edl.duration_ms,
    })
}

/// Render ONE base-track segment to its cache file (proxy-grade draft encode).
/// Media segments fast-seek the proxy at src_in and trim the duration; gaps
/// (empty proxy_path) emit black video + silence. Each segment is a fully
/// self-contained mp4 (one video + one audio stream, shared codec params) so
/// the concat demuxer can stream-copy them without re-encoding.
fn render_one_segment(p: &SegPlan, fps: f64, preset: &RenderPreset) -> Result<(), CutError> {
    if let Some(parent) = p.cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let fps_s = if fps.fract() == 0.0 {
        format!("{}", fps as u64)
    } else {
        format!("{fps:.3}")
    };
    // Proxy geometry as filter args. scale/pad take w:h (colon); the cache key
    // and lavfi color=s= use the WxH form — keep them distinct so we never feed
    // "960x540" into a filter expecting "960:540".
    let (w, h) = (crate::proxy::PROXY_WIDTH, crate::proxy::PROXY_HEIGHT);

    let mut args: Vec<String> = Vec::new();
    if p.proxy_path.as_os_str().is_empty() {
        // Gap: black video + silent audio of the gap duration.
        args.extend(
            [
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=black:s={w}x{h}:r={fps_s}:d={}", secs(p.dur_ms)),
                "-f",
                "lavfi",
                "-i",
                &format!("anullsrc=r=48000:cl=stereo:d={}", secs(p.dur_ms)),
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    } else {
        // Media (video proxy) OR still image (the still-image preview contract). Shared video filter:
        // scale+pad into the proxy frame, fps-normalise. Crop is ignored here
        // (source-space rect vs letterboxed proxy — same tradeoff as the scrub
        // path); preview is a feedback view, not a framing proof. fade/gain are
        // honored so the preview reflects them.
        let _ = &p.crop;
        let mut vf = format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps_s},format=yuv420p"
        );
        let mut af =
            String::from("aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo");
        if p.gain_db != 0.0 {
            write!(af, ",volume={:.2}dB", p.gain_db).unwrap();
        }
        if let Some(f) = &p.fade {
            // Segment-local fades, clamped to the segment duration.
            let in_ms = f.in_ms.min(p.dur_ms);
            let out_ms = f.out_ms.min(p.dur_ms);
            if matches!(f.kind, cut_core::FadeKind::Video | cut_core::FadeKind::Both) {
                if in_ms > 0 {
                    write!(vf, ",fade=t=in:st=0:d={}", secs(in_ms)).unwrap();
                }
                if out_ms > 0 {
                    write!(
                        vf,
                        ",fade=t=out:st={}:d={}",
                        secs(p.dur_ms - out_ms),
                        secs(out_ms)
                    )
                    .unwrap();
                }
            }
            if matches!(f.kind, cut_core::FadeKind::Audio | cut_core::FadeKind::Both) {
                if in_ms > 0 {
                    write!(af, ",afade=t=in:st=0:d={}", secs(in_ms)).unwrap();
                }
                if out_ms > 0 {
                    write!(
                        af,
                        ",afade=t=out:st={}:d={}",
                        secs(p.dur_ms - out_ms),
                        secs(out_ms)
                    )
                    .unwrap();
                }
            }
        }
        if p.image {
            // STILL: loop the single frame into an infinite stream (-loop 1,
            // mirrors render.rs's still input), trim to the clip duration, and
            // SYNTHESIZE silent audio (stills carry no audio track) so the
            // cached segment has the one A+V pair the concat demuxer needs. No
            // -ss seek: a still has no meaningful source timeline.
            args.extend(
                [
                    "-loop",
                    "1",
                    "-i",
                    &p.proxy_path.display().to_string(),
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("anullsrc=r=48000:cl=stereo:d={}", secs(p.dur_ms)),
                    "-t",
                    &secs(p.dur_ms),
                    "-vf",
                    &vf,
                    "-af",
                    &af,
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        } else {
            // VIDEO: input-side fast-seek the proxy, trim to the clip duration.
            args.extend(
                [
                    "-ss",
                    &secs(p.src_in_ms),
                    "-i",
                    &p.proxy_path.display().to_string(),
                    "-t",
                    &secs(p.dur_ms),
                    "-vf",
                    &vf,
                    "-af",
                    &af,
                ]
                .iter()
                .map(|s| s.to_string()),
            );
        }
    }
    // Draft-grade encode; a proxy with no audio still gets a silent track via
    // anullsrc on the gap path, and a media proxy always has the AAC track the
    // proxy step wrote — so every cached segment carries exactly one A+V pair.
    args.extend(preset.video_args.iter().cloned());
    args.extend(preset.audio_args.iter().cloned());
    // Stable timebase so concat-demuxer stream-copy never warns on timestamp
    // discontinuities between segments.
    args.extend(
        ["-video_track_timescale", "90000"]
            .iter()
            .map(|s| s.to_string()),
    );
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(p.cache_path.display().to_string());
    run_ffmpeg(&args)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The media cache key is content-addressed: identical inputs hash to the
    /// same filename (reuse), and ANY input change — trimming src_out, a gain
    /// change, adding a crop — yields a DIFFERENT filename (re-render). This is
    /// the mechanism behind "edit one clip in a 10-clip timeline, only the
    /// touched segment re-renders": the other nine segments keep their names
    /// and their cache files, only the edited segment gets a new name.
    #[test]
    fn media_segment_cache_key_is_content_addressed() {
        let g = "960x540";
        let base = media_segment_cache_name("sha256:a", 1000, 5000, g, 30.0, 0.0, None, None, None);
        // Same inputs → same key (reuse).
        assert_eq!(
            base,
            media_segment_cache_name("sha256:a", 1000, 5000, g, 30.0, 0.0, None, None, None),
            "identical inputs reuse the same cache file"
        );
        // Trimming src_out → new key (the edited segment re-renders).
        assert_ne!(
            base,
            media_segment_cache_name("sha256:a", 1000, 4000, g, 30.0, 0.0, None, None, None),
            "a trimmed source range re-renders"
        );
        // Different asset content → new key.
        assert_ne!(
            base,
            media_segment_cache_name("sha256:b", 1000, 5000, g, 30.0, 0.0, None, None, None)
        );
        // Gain change → new key (audio differs).
        assert_ne!(
            base,
            media_segment_cache_name("sha256:a", 1000, 5000, g, 30.0, -6.0, None, None, None)
        );
        // Adding a crop → new key (pixels differ).
        let crop = cut_core::ClipCrop {
            x: 0,
            y: 54,
            w: 3840,
            h: 2052,
        };
        assert_ne!(
            base,
            media_segment_cache_name(
                "sha256:a",
                1000,
                5000,
                g,
                30.0,
                0.0,
                Some(&crop),
                None,
                None
            )
        );
        // the still-image preview contract still-image segments: a still (still_dur_ms=Some) hashes apart
        // from the same-input video segment (still_dur_ms=None), and two cards
        // of DIFFERENT lengths from the same image get distinct cache files.
        let still_2s =
            media_segment_cache_name("sha256:a", 0, 0, g, 30.0, 0.0, None, None, Some(2000));
        let still_3s =
            media_segment_cache_name("sha256:a", 0, 0, g, 30.0, 0.0, None, None, Some(3000));
        assert_ne!(still_2s, still_3s, "different card lengths re-render");
        assert_ne!(
            still_2s,
            media_segment_cache_name("sha256:a", 0, 0, g, 30.0, 0.0, None, None, None),
            "a still segment never collides with a same-input video segment"
        );
        // Filename shape: seg_<16hex>.mp4 — concat-demuxer-friendly, collision-safe.
        assert!(
            base.starts_with("seg_") && base.ends_with(".mp4") && base.len() == 24,
            "{base}"
        );
    }

    /// Gap cache keys vary by duration but a same-duration gap reuses its file.
    #[test]
    fn gap_segment_cache_key_by_duration() {
        let g = "960x540";
        assert_eq!(
            gap_segment_cache_name(2000, g, 30.0),
            gap_segment_cache_name(2000, g, 30.0)
        );
        assert_ne!(
            gap_segment_cache_name(2000, g, 30.0),
            gap_segment_cache_name(3000, g, 30.0)
        );
    }
}
