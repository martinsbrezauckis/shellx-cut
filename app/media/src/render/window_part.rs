//! A render window shows exactly the frames the whole-timeline render shows
//! (row 194).
//!
//! The whole render gives a clip the frames `F(start)..F(end)` of the timeline
//! (`frame_budget`): its own frame `j` is output frame `F(start) + j`. Inside
//! the clip, ffmpeg makes frame `j` from the source frames trimmed from the in
//! point: renumbered at the same rate and speed 1, otherwise placed by their
//! own timestamps and converted by `fps=` (each output frame shows the last
//! source frame whose timestamp rounds to it or before).
//!
//! A window cuts a clip at a millisecond, `Edl::window` advances its source
//! range by milliseconds, and the part then starts its own conversion from
//! there: at a rate conversion or a speed change the first frames differ from
//! the whole render's (seen: 9 to 185 frames per render at window seams). So a
//! part is built from the WHOLE clip instead (`EdlSegment::window`, filled by
//! `Edl::window`): its frames `j0..j0+n` of the clip, where
//! `j0 = F(part start) - F(clip start)`:
//! - same rate, speed 1: the trim starts exactly on source frame `first + j0`;
//! - otherwise: the trim starts a couple of source frames before the first
//!   one needed, `setpts` gives those frames the timestamps they have in the
//!   whole clip (`+ round(offset/TB)`, the same integer arithmetic), and
//!   `fps=...:start_time=j0/fps` starts the conversion at output frame `j0` of
//!   the clip, then `setpts=PTS-STARTPTS` makes the part start at 0;
//! - reversed: the same on the reversed order (the trim keeps the source frames
//!   the part shows, ending at `last - r0`).
//! The part's end is a few source frames after the last one it needs; the
//! frame budget lock then keeps exactly `n` frames. Verified with ffmpeg 8.0.1
//! on 30->60, 25->30, 23.976->30/24, 60->30, speed 0.5/1.5 and reversed
//! (`scripts/part_probe.py`: every part frame-identical to the whole clip).
//!
//! Not built this way (they keep `Edl::window`'s range): frozen clips (one held
//! frame), speed-ramp slices (their own budget), stabilized clips (their
//! motion file is made per window part) and variable-frame-rate sources.

use super::frame_budget::frame_at;
use cut_core::{EdlSegment, Project};

/// The trim and timing pieces of a window part's chain.
pub(super) struct PartChain {
    pub(super) trim_in: String,
    pub(super) trim_out: String,
    pub(super) setpts: String,
    /// Replaces the chain's `vfps` (rate conversion with a start).
    pub(super) vfps: String,
}

/// Source rate as an exact fraction when the probe states it (`avg_frame_rate`
/// or `r_frame_rate` of the first video stream), else the probe's float.
fn source_rate(asset: &cut_core::Asset) -> Option<f64> {
    let stream = asset
        .probe
        .as_ref()?
        .get("raw")?
        .get("streams")?
        .as_array()?
        .iter()
        .find(|s| s["codec_type"].as_str() == Some("video"))
        .cloned();
    let rational = |v: &serde_json::Value| -> Option<f64> {
        let (n, d) = v.as_str()?.split_once('/')?;
        let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
        (n > 0.0 && d > 0.0).then_some(n / d)
    };
    stream
        .as_ref()
        .and_then(|s| rational(&s["avg_frame_rate"]).or_else(|| rational(&s["r_frame_rate"])))
        .or_else(|| super::source_fps(asset))
}

fn secs9(s: f64) -> String {
    format!("{:.9}", s.max(0.0))
}

/// The chain pieces for `seg` when it is part of a clip cut by a render
/// window, `None` for a whole clip (or one of the kinds listed above).
pub(super) fn part_chain(
    project: &Project,
    seg: &EdlSegment,
    fps: f64,
    vfps: &str,
) -> Option<PartChain> {
    let place = seg.window?;
    if seg.freeze.is_some()
        || seg.stabilize.is_some()
        || !super::ramp_timing::video_suffix(project, seg).is_empty()
    {
        return None;
    }
    let asset = project.assets.get(seg.asset.as_deref()?)?;
    let variable = asset
        .probe
        .as_ref()
        .and_then(|p| p.get("variable_frame_rate"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if variable {
        return None;
    }
    let (src_in, src_out) = (place.clip_src_in_ms?, place.clip_src_out_ms?);
    let sf = source_rate(asset)?;
    let speed = if seg.speed.is_finite() && seg.speed > 0.0 {
        seg.speed
    } else {
        1.0
    };
    let g0 = place.origin_ms + seg.timeline_in_ms;
    let g1 = place.origin_ms + seg.timeline_out_ms;
    let clip0 = frame_at(place.clip_timeline_in_ms, fps);
    let clip1 = frame_at(place.clip_timeline_out_ms, fps);
    let j0 = frame_at(g0, fps).saturating_sub(clip0);
    let n = frame_at(g1, fps).saturating_sub(frame_at(g0, fps));
    // The whole clip (its source range untouched by the window): the ordinary
    // chain is exact. A clip cut a few milliseconds before the window still
    // needs the part chain even when it has no frame before it (j0 = 0): the
    // window advanced its source range by those milliseconds (seen at 300 cuts).
    let whole = seg.src_in_ms == Some(src_in) && seg.src_out_ms == Some(src_out);
    if whole && j0 == 0 && n >= clip1.saturating_sub(clip0) {
        return None;
    }
    let renumbered = vfps.starts_with("settb=");
    let (a, b) = (src_in as f64 / 1000.0, src_out as f64 / 1000.0);
    const EPS: f64 = 2e-6;
    // Source frames per output frame of the clip, and the first and last
    // clip frames this part needs (a little wider for the conversion).
    let per = sf * speed / fps;
    let (r0, r_end) = if renumbered {
        (j0, j0 + n + 1)
    } else {
        (
            ((j0 as f64 * per).floor() as u64).saturating_sub(2),
            ((j0 + n) as f64 * per).floor() as u64 + 2,
        )
    };
    let (start, end) = if seg.reverse {
        let end = if r0 == 0 { b } else { b - r0 as f64 / sf - EPS };
        (a.max(b - (r_end + 1) as f64 / sf - EPS), end)
    } else {
        let start = if r0 == 0 { a } else { a + r0 as f64 / sf - EPS };
        (start, b.min(a + (r_end + 1) as f64 / sf))
    };
    if renumbered {
        return Some(PartChain {
            trim_in: secs9(start),
            trim_out: secs9(end),
            setpts: "setpts=PTS-STARTPTS".to_string(),
            vfps: vfps.to_string(),
        });
    }
    let offset = r0 as f64 / sf;
    let div = if speed == 1.0 {
        String::new()
    } else {
        format!("/{}", super::fnum(speed))
    };
    Some(PartChain {
        trim_in: secs9(start),
        trim_out: secs9(end),
        setpts: format!("setpts=(PTS-STARTPTS+round({}/TB)){div}", secs9(offset)),
        vfps: format!(
            "{vfps}:start_time={},setpts=PTS-STARTPTS",
            secs9(j0 as f64 / fps)
        ),
    })
}

/// The chain's `(trim_in, trim_out, setpts, vfps)`, replaced by
/// [`part_chain`]'s when `seg` is a window part.
pub(super) fn apply(
    project: &Project,
    seg: &EdlSegment,
    fps: f64,
    (trim_in, trim_out, setpts): (String, String, String),
    vfps: String,
) -> (String, String, String, String) {
    match part_chain(project, seg, fps, &vfps) {
        Some(p) => (p.trim_in, p.trim_out, p.setpts, p.vfps),
        None => (
            frameless_start(project, seg).unwrap_or(trim_in),
            trim_out,
            setpts,
            vfps,
        ),
    }
}

/// A clip shorter than one source frame between two frame starts (seen: the 1
/// ms clips `edit.speed` leaves at the end of a track) trims no frame at all,
/// and its slot then had no picture: the export came out a frame short. It
/// shows the source frame on screen at its in point: the trim starts one
/// source frame earlier. `None` for every clip that holds a frame start.
fn frameless_start(project: &Project, seg: &EdlSegment) -> Option<String> {
    if seg.freeze.is_some() {
        return None;
    }
    let (src_in, src_out) = (seg.src_in_ms?, seg.src_out_ms?);
    let sf = source_rate(project.assets.get(seg.asset.as_deref()?)?)?;
    let first = (src_in as f64 * sf / 1000.0 - 1e-6).ceil();
    let after_last = (src_out as f64 * sf / 1000.0 - 1e-6).ceil();
    (after_last <= first).then(|| secs9((first - 1.0).max(0.0) / sf))
}
