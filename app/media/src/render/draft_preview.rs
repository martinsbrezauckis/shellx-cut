//! The draft preview (`render.preview draft:true`) shows the export's frames.
//!
//! It used to render each base-track clip on its own from the asset proxy
//! (`-ss in -i proxy -t dur ... fps=`) and join the files: each clip came out
//! as a whole number of its own frames with its first frame doubled, speed was
//! ignored and a reversed clip played forward, so the draft was longer than
//! the timeline and showed other frames than the export (measured: 277 of 308
//! frames differ on 20 same-rate cuts).
//!
//! Now each piece of the draft is a window of the export's segmented render:
//! `Edl::window` cut at the base track's clip boundaries, rendered by the
//! export's own window chain (`render_window_video`: the frame lock, window
//! parts, speed, reverse, captions and overlays) at draft size, joined with the
//! concat demuxer, and the sound is the export's single audio pass. Every
//! output frame shows the export's source frame at the same index.
//!
//! Incremental reuse: a piece is cached under a hash of everything its
//! pixels depend on (see [`piece_key`]); a changed clip remakes only the
//! pieces it is in, and a ripple that moves later clips by whole frames keeps
//! their pieces. Pieces of the old draft (`seg_*.mp4`) are never read again.

use super::{
    build_graph, plain_internal_file_exists, plan_seg_resources, render_window_video,
    require_plain_internal_dir, reverse_memory, segment_trust, RenderOptions, RenderPreset,
    Resolution,
};
use crate::ffmpeg::{owned_concat_file_lines, DETERMINISM_FLAGS};
use crate::preview::PreviewResult;
use cut_core::{error_codes, CutError, Edl, Project};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::Path;

/// The draft's frame box (the proxy size). The project's aspect is kept.
const DRAFT_W: u32 = crate::proxy::PROXY_WIDTH;
const DRAFT_H: u32 = crate::proxy::PROXY_HEIGHT;

/// Cache file prefix of an exact draft piece. Changing the piece recipe
/// changes the prefix, so no older piece is ever reused.
const PIECE_PREFIX: &str = "draft5_";

/// The draft size of a project frame: scaled into DRAFT_W x DRAFT_H (never up),
/// even-sized for yuv420.
pub(super) fn draft_geometry(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (width.max(2) as f64, height.max(2) as f64);
    let s = (DRAFT_W as f64 / w).min(DRAFT_H as f64 / h).min(1.0);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    (even(w * s), even(h * s))
}

/// The pieces: the base track's clip boundaries over the whole timeline (a
/// window holding a reversed clip too big for memory is split further, as the
/// export does).
fn pieces(project: &Project, edl: &Edl) -> Vec<(u64, u64)> {
    let dur = edl.duration_ms;
    let mut cuts: Vec<u64> = vec![0, dur];
    if let Some(base) = edl.base_video_track() {
        for s in edl.track_segments(base) {
            cuts.push(s.timeline_in_ms.min(dur));
            cuts.push(s.timeline_out_ms.min(dur));
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    // Every piece holds at least one frame: a cut that adds no frame (a clip
    // shorter than half a frame between two grid points) joins its neighbour.
    let fps = project.settings.fps;
    let mut kept: Vec<u64> = vec![0];
    for &c in &cuts[1..] {
        if super::frame_budget::frame_at(c, fps)
            > super::frame_budget::frame_at(*kept.last().unwrap(), fps)
        {
            kept.push(c);
        } else if c == dur && kept.len() > 1 {
            *kept.last_mut().unwrap() = dur;
        }
    }
    if kept.len() < 2 {
        kept = vec![0, dur];
    }
    let windows: Vec<(u64, u64)> = kept.windows(2).map(|w| (w[0], w[1])).collect();
    reverse_memory::split_windows(project, edl, windows, fps)
}

/// The shortest whole number of milliseconds that moves the frame grid by a
/// whole number of frames (30 fps: 100 ms; 60: 50; 30000/1001: 1001), when the
/// rate is a fraction with a denominator up to 1001.
fn grid_period_ms(fps: f64) -> Option<u64> {
    (1..=1001u64).find_map(|q| {
        let p = (fps * q as f64).round();
        if p < 1.0 || (p / q as f64 - fps).abs() > 1e-9 {
            return None;
        }
        let (a, b) = (1000 * q, p as u64);
        let g = gcd(a, b);
        Some(a / g)
    })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The window EDL with its absolute positions made relative to the window, and
/// the window's place on the timeline kept only as its phase on the frame grid
/// (`origin mod period`). Moving a piece by a whole number of frames leaves
/// every frame of it unchanged: every frame index the chain computes,
/// `round((origin + t) * fps / 1000)`, moves by the same whole number (a
/// rounding tie is exact in f64 at whole rates and impossible at /1001 rates).
fn normalized_window(wedl: &Edl, fps: f64) -> Value {
    let mut v = serde_json::to_value(wedl).unwrap_or(Value::Null);
    let period = grid_period_ms(fps);
    if let Some(segs) = v.get_mut("segments").and_then(Value::as_array_mut) {
        for seg in segs {
            let Some(place) = seg.get_mut("window").and_then(Value::as_object_mut) else {
                continue;
            };
            let origin = place.get("origin_ms").and_then(Value::as_u64).unwrap_or(0);
            let Some(period) = period else {
                continue; // an unusual rate keeps its absolute place
            };
            for key in ["clip_timeline_in_ms", "clip_timeline_out_ms"] {
                if let Some(t) = place.get(key).and_then(Value::as_u64) {
                    place.insert(key.into(), Value::from(t as i64 - origin as i64));
                }
            }
            place.insert("origin_ms".into(), Value::from(origin % period));
        }
    }
    v
}

/// The cache key of one piece: everything its pixels depend on. The window EDL
/// without its audio-track clips
/// (normalized, above), the project settings, caption styles, brand kit and
/// nests, every track's own properties (without its clips), the full record of
/// each clip in the piece (a clip's record holds no timeline position), the
/// content hash of each asset it reads, the draft size and the encoder args.
fn piece_key(project: &Project, wedl: &Edl, gw: u32, gh: u32, preset: &RenderPreset) -> String {
    // A piece is picture only (its sound is the single audio pass): clips on
    // audio tracks do not change it, so an edit of the sound keeps it.
    let picture = Edl {
        segments: wedl
            .segments
            .iter()
            .filter(|s| s.track_kind != cut_core::TrackKind::Audio)
            .cloned()
            .collect(),
        ..wedl.clone()
    };
    let wedl = &picture;
    let mut h = Sha256::new();
    h.update(PIECE_PREFIX.as_bytes());
    let fps = project.settings.fps;
    h.update(serde_json::to_vec(&normalized_window(wedl, fps)).unwrap_or_default());
    h.update(serde_json::to_vec(&project.settings).unwrap_or_default());
    h.update(serde_json::to_vec(&project.caption_styles).unwrap_or_default());
    h.update(serde_json::to_vec(&project.brand).unwrap_or_default());
    h.update(serde_json::to_vec(&project.nests).unwrap_or_default());
    for t in &project.tracks {
        let mut tv = serde_json::to_value(t).unwrap_or(Value::Null);
        if let Some(o) = tv.as_object_mut() {
            o.remove("clips");
        }
        h.update(serde_json::to_vec(&tv).unwrap_or_default());
    }
    let mut clips: Vec<&str> = wedl
        .segments
        .iter()
        .filter_map(|s| s.clip_id.as_deref())
        .collect();
    clips.sort_unstable();
    clips.dedup();
    for id in clips {
        if let Some((tid, i)) = project.find_clip(id) {
            if let Some(track) = project.track(tid) {
                h.update(serde_json::to_vec(&track.clips[i]).unwrap_or_default());
            }
        }
    }
    let mut assets: Vec<&str> = wedl
        .segments
        .iter()
        .filter_map(|s| s.asset.as_deref())
        .collect();
    assets.sort_unstable();
    assets.dedup();
    for id in assets {
        if let Some(a) = project.assets.get(id) {
            h.update(id.as_bytes());
            h.update(a.hash.as_bytes());
        }
    }
    h.update(format!("{gw}x{gh}@{fps}").as_bytes());
    h.update(format!("{:?}", preset.video_args).as_bytes());
    format!("{PIECE_PREFIX}{}", &format!("{:x}", h.finalize())[..20])
}

/// Render the draft preview of the whole timeline to `<cache_dir>/preview.mp4`,
/// remaking only the pieces whose key is not cached.
pub fn render_draft(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    cache_dir: &Path,
    preset: &RenderPreset,
) -> Result<PreviewResult, CutError> {
    super::keyframe_limit::require_bounded_keyframes(project, edl)?;
    let measured = super::source_facts::measured_for_reverse(project, project_dir); // reversed clips
    let project = measured.as_ref().unwrap_or(project);
    let (gw, gh) = draft_geometry(project.settings.width, project.settings.height);
    let opts = RenderOptions {
        resolution: Resolution::Explicit {
            width: gw,
            height: gh,
        },
        ..RenderOptions::default()
    };
    require_plain_internal_dir(cache_dir)?;
    let windows = pieces(project, edl);
    let longest = windows.iter().map(|(a, b)| b - a).max().unwrap_or(0);
    let (_, caps) =
        plan_seg_resources(longest.max(1), gw, gh, super::overlay_track_count(project))?;
    let mut files = Vec::with_capacity(windows.len());
    let (mut rendered, mut reused) = (Vec::new(), Vec::new());
    let mut list = String::new();
    for &(w0, w1) in &windows {
        let wedl = edl.window(w0, w1);
        let name = format!("{}.mp4", piece_key(project, &wedl, gw, gh, preset));
        let path = cache_dir.join(&name);
        writeln!(list, "{}", owned_concat_file_lines(&path)?).unwrap();
        files.push((wedl, path, name, w0));
    }
    for (wedl, path, name, w0) in &files {
        if plain_internal_file_exists(path)? && segment_trust::trusted(path) {
            if !reused.contains(name) {
                reused.push(name.clone());
            }
            continue;
        }
        render_window_video(
            project,
            wedl,
            project_dir,
            path,
            preset,
            opts,
            caps,
            *w0,
            None,
            &|_| {},
        )?;
        segment_trust::remember(path);
        rendered.push(name.clone());
    }

    // Join the pieces (stream copy) in the engine's private folder.
    let private = crate::private_work::session_dir()?;
    let list_path = crate::internal_files::private_list_file(".cut-draft-", ".txt", &list)?;
    let video_only = tempfile::Builder::new()
        .prefix(".cut-draft-")
        .suffix(".mp4")
        .tempfile_in(&private)?
        .into_temp_path();
    let mut cargs: Vec<String> = ["-f", "concat", "-safe", "0", "-i"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    cargs.push(list_path.display().to_string());
    cargs.extend(["-c".to_string(), "copy".to_string()]);
    cargs.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    cargs.push(video_only.display().to_string());
    crate::ffmpeg::run_owned_concat_atomic_output(&cargs, &video_only)?;

    // The export's single audio pass, muxed onto the joined picture.
    let out = cache_dir.join("preview.mp4");
    plain_internal_file_exists(&out)?;
    let agraph = build_graph(project, edl, project_dir, false, true, false, opts, None)?;
    let mut margs: Vec<String> = Vec::new();
    if let Some(aout) = agraph.audio_out.clone() {
        for input in &agraph.inputs {
            if input.image {
                margs.extend(["-loop".to_string(), "1".to_string()]);
            }
            margs.extend(["-i".to_string(), input.path.display().to_string()]);
        }
        let vid = agraph.inputs.len();
        margs.extend(["-i".to_string(), video_only.display().to_string()]);
        margs.push("-filter_complex".into());
        margs.push(agraph.filter.trim_end_matches(['\n', ';']).to_string());
        margs.extend(["-map".to_string(), format!("[{aout}]")]);
        margs.extend(["-map".to_string(), format!("{vid}:v")]);
        margs.extend(["-c:v".to_string(), "copy".to_string()]);
        margs.extend(preset.audio_args.iter().cloned());
    } else {
        margs.extend(["-i".to_string(), video_only.display().to_string()]);
        margs.extend(["-c".to_string(), "copy".to_string()]);
    }
    margs.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    margs.push(out.display().to_string());
    crate::ffmpeg::run_ffmpeg_atomic_output(&margs, &out)?;
    if !out.is_file() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "the draft preview was not written",
            "ffmpeg finished without the joined preview file",
        )
        .with_suggested_action(
            "run render.preview draft:true again; if it repeats, use render.final",
        ));
    }
    Ok(PreviewResult {
        path: out,
        segments_rendered: rendered.len(),
        segments_reused: reused.len(),
        rendered,
        reused,
        duration_ms: edl.duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_draft_keeps_the_project_aspect_inside_the_proxy_box() {
        assert_eq!(draft_geometry(1920, 1080), (960, 540));
        assert_eq!(draft_geometry(3840, 2160), (960, 540));
        assert_eq!(draft_geometry(1080, 1920), (304, 540));
        assert_eq!(draft_geometry(640, 360), (640, 360));
        assert_eq!(draft_geometry(1080, 1080), (540, 540));
    }

    #[test]
    fn the_grid_period_moves_the_frame_grid_by_whole_frames() {
        assert_eq!(grid_period_ms(30.0), Some(100));
        assert_eq!(grid_period_ms(60.0), Some(50));
        assert_eq!(grid_period_ms(25.0), Some(40));
        assert_eq!(grid_period_ms(24.0), Some(125));
        assert_eq!(grid_period_ms(30000.0 / 1001.0), Some(1001));
        assert_eq!(grid_period_ms(24000.0 / 1001.0), Some(1001));
        // Every shift by the period moves every frame index by the same amount.
        for fps in [
            24.0,
            25.0,
            30.0,
            50.0,
            60.0,
            30000.0 / 1001.0,
            24000.0 / 1001.0,
        ] {
            let p = grid_period_ms(fps).unwrap();
            let k = (p as f64 * fps / 1000.0).round() as i64;
            for t in 0..5000u64 {
                let f = |t: u64| (t as f64 * fps / 1000.0).round() as i64;
                assert_eq!(f(t + 7 * p) - f(t), 7 * k, "fps {fps} t {t}");
            }
        }
    }
}
