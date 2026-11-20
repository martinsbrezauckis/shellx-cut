//! Every video segment carries exactly its timeline slot's frames (row 194).
//!
//! A segment chain trims SOURCE frames (`trim` keeps every source frame that
//! starts before the out point) and then renumbers them (same rate) or
//! converts them with `fps=` (other rates or speeds). Its length is therefore a
//! whole number of source frames, not its slot: a 513 ms clip of a 30 fps
//! source is 16 frames (533 ms) in a 30 fps project, 32 frames in a 60 fps one.
//! `concat` appends segments one after another, so each such cut moved every
//! later frame late (measured: 24 frames, 0.4 s, after 19 cuts; the audio is cut
//! to the sample, so the picture also drifted from the sound), and the file
//! was longer than the timeline.
//!
//! The timeline's frame grid is the one `cut_core::timeline_frame_count` and
//! the speed-ramp frame budgets use: a position of `t` ms is frame
//! `round(t * fps / 1000)`. A segment from `in` to `out` gets
//! `F(out) - F(in)` frames, so the segments of a track add up to
//! `F(duration)` and each cut takes effect on its own frame. Inside a segment
//! the clip still starts on its in-point frame (nothing in a segment moves);
//! only frames past its slot are dropped, or the last one held when a source
//! ends early.
//!
//! A segment whose chain already yields exactly that count (the same rate at
//! speed 1, or a rate conversion, with every edge on a source frame and on a
//! timeline frame) gets no extra filter, so those renders keep their bytes.

use cut_core::{Asset, Edl, EdlSegment, Project};
use std::fmt::Write as _;

/// One stream of a video track in concat order: its timeline span and the
/// frame count its chain yields by itself (`None` = not known exactly).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Span {
    pub(super) in_ms: u64,
    pub(super) out_ms: u64,
    pub(super) natural: Option<u64>,
}

/// Timeline frame of a position: `round(t * fps / 1000)`.
pub(super) fn frame_at(t_ms: u64, fps: f64) -> u64 {
    (t_ms as f64 * fps / 1000.0).round() as u64
}

/// Frames of the slot `[in, out)` on the timeline grid.
pub(super) fn slot_frames(in_ms: u64, out_ms: u64, fps: f64) -> u64 {
    frame_at(out_ms, fps).saturating_sub(frame_at(in_ms, fps))
}

const EPS: f64 = 1e-6;

fn whole(x: f64) -> Option<u64> {
    (x >= -EPS && (x - x.round()).abs() < EPS).then(|| x.round() as u64)
}

/// Frames a generated span (gap, transparent filler, tail pad:
/// `color=...:r=fps:d=secs`) yields, when exact.
fn generated_frames(in_ms: u64, out_ms: u64, fps: f64) -> Option<u64> {
    whole(out_ms.saturating_sub(in_ms) as f64 * fps / 1000.0)
}

/// Where a window's 0 is on the whole timeline (0 for a whole timeline): the
/// frame grid is the whole timeline's, so a window counts frames from there.
fn origin(edl: &Edl) -> u64 {
    edl.segments
        .iter()
        .find_map(|s| s.window.map(|w| w.origin_ms))
        .unwrap_or(0)
}

/// True when `seg` is a part of a clip that a render window cut.
fn is_part(seg: &EdlSegment) -> bool {
    seg.window.is_some_and(|w| {
        w.clip_timeline_in_ms < w.origin_ms + seg.timeline_in_ms
            || w.clip_timeline_out_ms > w.origin_ms + seg.timeline_out_ms
    })
}

/// Frames a media segment's chain yields by itself, when exact.
fn media_frames(project: &Project, seg: &EdlSegment, fps: f64) -> Option<u64> {
    if is_part(seg) {
        return None; // a window part is always locked to its frames
    }
    if !super::ramp_timing::video_suffix(project, seg).is_empty() {
        return None; // a ramp slice carries its own budget; lock it to the grid too
    }
    let (src_in, src_out) = (seg.src_in_ms?, seg.src_out_ms?);
    if seg.freeze.is_some() {
        // freeze_chain holds one frame for round(slot * fps) frames.
        let slot = seg.timeline_out_ms - seg.timeline_in_ms;
        return Some(((slot as f64 / 1000.0) * fps).round().max(1.0) as u64);
    }
    let asset: &Asset = project.assets.get(seg.asset.as_deref()?)?;
    let variable = asset
        .probe
        .as_ref()
        .and_then(|p| p.get("variable_frame_rate"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sf = super::source_fps(asset)?;
    if variable {
        return None;
    }
    let first = whole(src_in as f64 * sf / 1000.0)?;
    let last = whole(src_out as f64 * sf / 1000.0)?;
    if !video_reaches(asset, last) {
        return None; // the picture ends before the out point: lock (hold its last frame)
    }
    let source_frames = last.checked_sub(first)?;
    let same_rate = (sf - fps).abs() < 0.01;
    if same_rate && seg.speed == 1.0 {
        return Some(source_frames);
    }
    // fps= fills the trimmed duration: exact when that is whole frames.
    whole(source_frames as f64 * fps / (sf * seg.speed))
}

/// True when the asset's video stream holds source frame `end - 1`, by its
/// probed frame count (or stream duration). A recovered or cut-short recording
/// can declare a longer duration than its frames (seen: a 6.900 s clip whose
/// video is 206 frames at 30 fps); its chain then yields fewer frames than the
/// range asks for, so only a source known to reach the out point counts as
/// exact (measured: an export 2 frames short).
fn video_reaches(asset: &Asset, end: u64) -> bool {
    let Some(raw) = asset.probe.as_ref().and_then(|p| p.get("raw")) else {
        return false;
    };
    let Some(video) = crate::probe::stream_extents_from_raw(raw)
        .into_iter()
        .find(|e| e.kind == "video")
    else {
        return false;
    };
    match (video.frames, video.duration_ms, super::source_fps(asset)) {
        (Some(frames), _, _) => end <= frames,
        (None, Some(ms), Some(sf)) => end as f64 <= ms as f64 * sf / 1000.0 + EPS,
        _ => false,
    }
}

/// The spans of the BASE track's streams, in the order `build_graph` pushes
/// them: the black canvas of a hidden base, or each segment (media or gap),
/// then the black tail pad when an overlay outlasts the base.
pub(super) fn base_spans(
    project: &Project,
    edl: &Edl,
    track_id: &str,
    visible: bool,
    fps: f64,
) -> Vec<Span> {
    let dur = edl.duration_ms.max(1);
    let o = origin(edl);
    if !visible {
        return vec![Span {
            in_ms: o,
            out_ms: o + dur,
            natural: generated_frames(o, o + dur, fps),
        }];
    }
    let mut out: Vec<Span> = edl
        .track_segments(track_id)
        .map(|seg| {
            let media = seg.asset.is_some() && seg.src_in_ms.is_some() && seg.src_out_ms.is_some();
            Span {
                in_ms: o + seg.timeline_in_ms,
                out_ms: o + seg.timeline_out_ms,
                natural: if media {
                    media_frames(project, seg, fps)
                } else {
                    generated_frames(o + seg.timeline_in_ms, o + seg.timeline_out_ms, fps)
                },
            }
        })
        .collect();
    let base_end = edl
        .track_segments(track_id)
        .map(|s| s.timeline_out_ms)
        .max()
        .unwrap_or(0);
    if base_end < edl.duration_ms {
        out.push(Span {
            in_ms: o + base_end,
            out_ms: o + edl.duration_ms,
            natural: generated_frames(o + base_end, o + edl.duration_ms, fps),
        });
    }
    out
}

/// The spans of an OVERLAY track's streams in `build_graph`'s order:
/// transparent filler up to each media segment, the segment, a final filler.
pub(super) fn overlay_spans(project: &Project, edl: &Edl, track_id: &str, fps: f64) -> Vec<Span> {
    let mut out = Vec::new();
    let mut cursor = 0u64;
    let o = origin(edl);
    let filler = |from: u64, to: u64, out: &mut Vec<Span>| {
        if to > from {
            out.push(Span {
                in_ms: o + from,
                out_ms: o + to,
                natural: generated_frames(o + from, o + to, fps),
            });
        }
    };
    for seg in edl.track_segments(track_id) {
        if seg.asset.is_none() || seg.src_in_ms.is_none() || seg.src_out_ms.is_none() {
            continue;
        }
        filler(cursor, seg.timeline_in_ms, &mut out);
        out.push(Span {
            in_ms: o + seg.timeline_in_ms,
            out_ms: o + seg.timeline_out_ms,
            natural: media_frames(project, seg, fps),
        });
        cursor = seg.timeline_out_ms;
    }
    filler(cursor, edl.duration_ms, &mut out);
    out
}

/// Lock `label`'s stream to its slot's frames when its chain does not yield
/// exactly that many: writes one chain and returns the new label. A slot of
/// no frame (shorter than half a frame between two grid points) is sunk and
/// returns `Some("")`, which the caller drops from the concat.
pub(super) fn lock(f: &mut String, label: &str, span: &Span, fps: f64) -> Option<String> {
    let frames = slot_frames(span.in_ms, span.out_ms, fps);
    if span.natural == Some(frames) {
        return None;
    }
    if frames == 0 {
        writeln!(f, "[{label}]nullsink;").unwrap();
        return Some(String::new());
    }
    let pad = frames as f64 / fps;
    let out = format!("{label}fb");
    writeln!(
        f,
        "[{label}]tpad=stop_mode=clone:stop_duration={pad:.3},trim=end_frame={frames},\
         setpts=PTS-STARTPTS[{out}];"
    )
    .unwrap();
    Some(out)
}

/// [`lock_track`] for the base track's segment streams (a dropped stream
/// leaves the concat; its crossfade, if any, goes with it).
pub(super) fn lock_base(
    f: &mut String,
    segs: &mut Vec<super::SegStream>,
    spans: &[Span],
    fps: f64,
) {
    if segs.len() != spans.len() {
        return;
    }
    for (seg, span) in segs.iter_mut().zip(spans) {
        if let Some(new) = lock(f, &seg.label, span, fps) {
            seg.label = new;
        }
    }
    segs.retain(|s| !s.label.is_empty());
}

/// Lock every stream of one track; `labels` and `spans` are in concat order.
/// Spans that do not line up with the labels (a track `build_graph` built
/// differently) leave the track as it is.
pub(super) fn lock_track(f: &mut String, labels: &mut Vec<String>, spans: &[Span], fps: f64) {
    if labels.len() != spans.len() {
        return;
    }
    for (label, span) in labels.iter_mut().zip(spans) {
        if let Some(new) = lock(f, label, span, fps) {
            *label = new;
        }
    }
    labels.retain(|l| !l.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slots_of_a_track_add_up_to_the_timeline_frames() {
        // 20 clips of 513 ms at 30 fps: 10 260 ms = 308 frames, not 20 x 16.
        let fps = 30.0;
        let total: u64 = (0..20)
            .map(|k| slot_frames(k * 513, (k + 1) * 513, fps))
            .sum();
        assert_eq!(total, cut_core::timeline_frame_count(10_260, fps));
        assert_eq!(slot_frames(0, 10_270, 60.0), 616);
    }

    #[test]
    fn an_exact_span_gets_no_filter_and_an_inexact_one_is_locked() {
        let mut f = String::new();
        let exact = Span {
            in_ms: 0,
            out_ms: 1000,
            natural: Some(30),
        };
        assert_eq!(lock(&mut f, "v0", &exact, 30.0), None);
        assert!(f.is_empty());
        let long = Span {
            in_ms: 0,
            out_ms: 513,
            natural: Some(16),
        };
        assert_eq!(lock(&mut f, "v0", &long, 30.0).as_deref(), Some("v0fb"));
        assert!(f.contains("trim=end_frame=15"), "{f}");
        let unknown = Span {
            in_ms: 513,
            out_ms: 1026,
            natural: None,
        };
        assert_eq!(lock(&mut f, "v1", &unknown, 30.0).as_deref(), Some("v1fb"));
    }

    #[test]
    fn generated_spans_are_exact_only_on_whole_frames() {
        assert_eq!(generated_frames(0, 1000, 30.0), Some(30));
        assert_eq!(generated_frames(0, 513, 30.0), None);
        assert_eq!(generated_frames(0, 1001, 30_000.0 / 1001.0), Some(30));
    }
}
