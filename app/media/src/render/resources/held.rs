//! Source frames a render holds for segments that play later (items 82, 164).
//!
//! The graph opens ONE input per source (`graph_inputs.rs`) and every segment
//! cut from it starts with `[N:v]trim=...`. The decoder only moves forward, so
//! frames a LATER segment needs that the decoder passes early wait in memory
//! (at that segment's `concat` input) until the timeline reaches them.
//!
//! What waits depends on the segment's filter chain, which ffmpeg runs as soon
//! as a frame arrives:
//! - pass-through chain (source already at the output size, 8-bit 4:2:0, no
//!   grade, colour conversion, transform, fade, mask, matte, animation or
//!   stabilization): the waiting frame is a reference to the DECODER's buffer.
//!   Every segment that waits for the same source frame shares that one
//!   buffer, so the cost is the UNION of the waiting source time.
//! - any other chain: each segment holds its own converted copy at the output
//!   size, so the cost is the SUM over segments.
//!
//! Measured on the Mac Studio with the HQ phone clip (4K60, 13.3 s) five times
//! in a row: 10.2 GB (one copy of the clip); the old sum counted four copies
//! (39 GB). The per-case table is in the item-164 record.

use cut_core::{Edl, EdlSegment, Project, TrackKind};
use std::collections::BTreeMap;

/// One use of a source on the timeline, in ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SourceUse {
    pub tl_in: u64,
    pub tl_out: u64,
    pub src_in: u64,
    pub src_out: u64,
    /// Source ms per timeline ms (edit.speed).
    pub speed: f64,
    /// `reverse` buffers the whole cut before its first frame plays.
    pub reverse: bool,
    /// Waiting frames are shared decoder buffers (see the module comment).
    pub shared: bool,
}

/// Bytes and rates of one source in one render.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frames {
    /// One decoded source frame.
    pub source_bytes: u64,
    /// One converted frame at the output size (yuv420p).
    pub output_bytes: u64,
    pub source_fps: f64,
    pub output_fps: f64,
}

impl SourceUse {
    fn len_ms(&self) -> f64 {
        (self.tl_out - self.tl_in) as f64
    }
    /// How far the decoder must have read for this use at timeline `t`.
    fn decoder_pos(&self, t: u64) -> u64 {
        if t < self.tl_in {
            return 0;
        }
        if self.reverse {
            return self.src_out;
        }
        let played = (t.min(self.tl_out) - self.tl_in) as f64 * self.speed;
        (self.src_in as f64 + played).min(self.src_out as f64) as u64
    }
    /// Source range still to be played at timeline `t` (None when finished).
    fn still_needed(&self, t: u64) -> Option<(u64, u64)> {
        if t >= self.tl_out {
            return None;
        }
        if t <= self.tl_in {
            return Some((self.src_in, self.src_out));
        }
        let left = (self.tl_out - t) as f64 / self.len_ms().max(1.0);
        let span = (self.src_out - self.src_in) as f64;
        Some(if self.reverse {
            // Reverse plays the end first: what is left is the start.
            (self.src_in, self.src_in + (span * left) as u64)
        } else {
            (self.src_out - (span * left) as u64, self.src_out)
        })
    }
    /// Distinct frames per source second that reach the concat input.
    fn frames_per_source_sec(&self, f: &Frames) -> f64 {
        f.source_fps.min(f.output_fps / self.speed.max(0.01))
    }
}

fn union_ms(mut ranges: Vec<(u64, u64)>) -> u64 {
    ranges.sort_unstable();
    let (mut total, mut end) = (0u64, 0u64);
    for (a, b) in ranges {
        let a = a.max(end);
        if b > a {
            total += b - a;
            end = b;
        }
    }
    total
}

/// Peak bytes one source's waiting frames take, over the whole render.
pub(crate) fn peak_held_bytes(uses: &[SourceUse], f: &Frames) -> u64 {
    let mut times: Vec<u64> = uses.iter().flat_map(|u| [u.tl_in, u.tl_out]).collect();
    times.sort_unstable();
    times.dedup();
    let mut peak = 0f64;
    for &t in &times {
        let decoded = uses.iter().map(|u| u.decoder_pos(t)).max().unwrap_or(0);
        let mut shared = Vec::new();
        let mut shared_rate = 0f64;
        let mut copies = 0f64;
        for u in uses {
            let Some((a, b)) = u.still_needed(t) else {
                continue;
            };
            let b = b.min(decoded);
            if b <= a {
                continue;
            }
            if u.shared {
                shared.push((a, b));
                shared_rate = shared_rate.max(if u.reverse {
                    f.source_fps
                } else {
                    u.frames_per_source_sec(f)
                });
            } else {
                let frames = (b - a) as f64 / 1000.0 * u.frames_per_source_sec(f);
                let converted = frames * f.output_bytes as f64;
                // Reverse buffers source frames before scaling or dropping frames.
                // Size the larger stage: the source buffer or the converted queue.
                let reversed = if u.reverse {
                    (b - a) as f64 / 1000.0 * f.source_fps * f.source_bytes as f64
                } else {
                    0.0
                };
                copies += converted.max(reversed);
            }
        }
        let shared_bytes = union_ms(shared) as f64 / 1000.0 * shared_rate * f.source_bytes as f64;
        peak = peak.max(shared_bytes + copies);
    }
    peak.round() as u64
}

/// The decoded pixel format named by ffprobe for the first video stream.
pub(crate) fn source_pix_fmt(asset: &cut_core::Asset) -> Option<String> {
    asset
        .probe
        .as_ref()?
        .get("raw")?
        .get("streams")?
        .as_array()?
        .iter()
        .find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"))?
        .get("pix_fmt")?
        .as_str()
        .map(str::to_string)
}

fn is_image(asset: &cut_core::Asset) -> bool {
    asset
        .probe
        .as_ref()
        .and_then(|p| p.get("kind"))
        .and_then(|k| k.as_str())
        == Some("image")
}

/// Whether a BASE-track segment's chain hands the decoder's own buffer on
/// unchanged (mirrors the per-segment chain in `build_graph`).
fn passes_through(
    project: &Project,
    seg: &EdlSegment,
    asset: &cut_core::Asset,
    w: u32,
    h: u32,
) -> bool {
    use crate::render as r;
    let fit = r::RenderOptions::default().fit;
    let conform_noop =
        r::conform_filter_for_asset(asset, seg.crop.as_ref(), w, h, fit) == "setsar=1";
    let transform = project
        .find_clip(seg.clip_id.as_deref().unwrap_or_default())
        .and_then(|(tid, i)| match &project.track(tid)?.clips[i] {
            cut_core::Clip::Media(c) => c.transform.clone(),
            _ => None,
        })
        .unwrap_or_else(cut_core::ClipTransform::identity);
    conform_noop
        && !is_image(asset)
        && source_pix_fmt(asset).as_deref() == Some("yuv420p")
        && r::source_prep::clip_source_prep(Some(asset), seg.input_color_space.as_ref()).is_empty()
        && r::source_color::clip_color_chain(
            Some(asset),
            seg.input_color_space.as_ref(),
            &project.settings.color,
        )
        .is_empty()
        && r::grade_stack_filter(seg.grade.as_ref(), &seg.grade_stack).is_empty()
        && r::base_transform_filter(&transform, w, h).is_empty()
        && r::base_opacity_filter(&transform, &seg.keyframes).is_empty()
        && seg.grade_windows.is_empty()
        && seg.effects.is_empty()
        && seg.fade.is_none()
        && seg.mask.is_none()
        && seg.matte.is_none()
        && seg.animation.is_none()
        && seg.keyframes.is_empty()
        && seg.stabilize.is_none()
}

/// Every visible video use of every source in `edl`, by asset id. Overlay
/// tracks always convert (they are padded onto a transparent canvas).
pub(crate) fn uses_by_source<'a>(
    project: &'a Project,
    edl: &'a Edl,
    w: u32,
    h: u32,
) -> BTreeMap<&'a str, Vec<SourceUse>> {
    let base = crate::render::planned_video_tracks(project)
        .first()
        .map(|t| t.id.to_string());
    let mut out: BTreeMap<&str, Vec<SourceUse>> = BTreeMap::new();
    for seg in &edl.segments {
        if seg.track_kind != TrackKind::Video
            || !crate::render::graph_inputs::segment_video_track_visible(project, seg)
        {
            continue;
        }
        let (Some(id), Some(a), Some(b)) = (seg.asset.as_deref(), seg.src_in_ms, seg.src_out_ms)
        else {
            continue;
        };
        let Some(asset) = project.assets.get(id) else {
            continue;
        };
        let fps = project.settings.fps.max(1.0);
        // A frozen clip decodes one frame at its freeze point (freeze_chain).
        let (a, b) = match &seg.freeze {
            Some(fz) => {
                let at = (a + fz.at_ms).min(b.saturating_sub(1)).max(a);
                (at, (at + (1000.0 / fps) as u64).min(b.max(at + 1)))
            }
            None => (a, b.max(a)),
        };
        let on_base = base.as_deref() == Some(seg.track.as_str());
        out.entry(id).or_default().push(SourceUse {
            tl_in: seg.timeline_in_ms,
            tl_out: seg.timeline_out_ms.max(seg.timeline_in_ms + 1),
            src_in: a,
            src_out: b,
            speed: if seg.speed > 0.0 { seg.speed } else { 1.0 },
            reverse: seg.reverse && seg.freeze.is_none(),
            shared: on_base && passes_through(project, seg, asset, w, h),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: Frames = Frames {
        source_bytes: 1_000,
        output_bytes: 1_000,
        source_fps: 60.0,
        output_fps: 60.0,
    };

    fn cut(tl_in: u64, src_in: u64, src_out: u64, shared: bool) -> SourceUse {
        SourceUse {
            tl_in,
            tl_out: tl_in + (src_out - src_in),
            src_in,
            src_out,
            speed: 1.0,
            reverse: false,
            shared,
        }
    }

    /// The HQ record's case: one 13.3 s clip five times in a row. The
    /// decoder reads it once; four later uses wait for the same frames.
    #[test]
    fn repeated_clip_shares_one_copy_when_the_chain_passes_through() {
        let uses: Vec<SourceUse> = (0..5).map(|i| cut(i * 13_313, 0, 13_313, true)).collect();
        // One copy of the clip: 13.313 s x 60 frames.
        assert_eq!(peak_held_bytes(&uses, &FRAMES), 798_780);
        // Converting chains hold their own copy each: four.
        let copies: Vec<SourceUse> = uses
            .iter()
            .map(|u| SourceUse {
                shared: false,
                ..*u
            })
            .collect();
        assert_eq!(peak_held_bytes(&copies, &FRAMES), 4 * 798_780);
    }

    /// Disjoint re-cut (3-6, 9-12, 0-3 s): 3 s wait, shared or not.
    #[test]
    fn out_of_order_cut_holds_what_the_decoder_passed() {
        let uses = [
            cut(0, 3_000, 6_000, true),
            cut(3_000, 9_000, 12_000, true),
            cut(6_000, 0, 3_000, true),
        ];
        assert_eq!(peak_held_bytes(&uses, &FRAMES), 180_000);
        let in_order = [cut(0, 0, 3_000, true), cut(3_000, 3_000, 6_000, true)];
        assert_eq!(peak_held_bytes(&in_order, &FRAMES), 0);
    }

    /// A reversed cut decodes its whole range before its first frame.
    #[test]
    fn reverse_holds_its_whole_cut() {
        let mut r = cut(0, 0, 5_000, true);
        r.reverse = true;
        assert_eq!(peak_held_bytes(&[r], &FRAMES), 300_000);
    }

    /// A 30 fps project keeps every other frame of a 60 fps source.
    #[test]
    fn lower_project_rate_holds_fewer_frames() {
        let f = Frames {
            output_fps: 30.0,
            ..FRAMES
        };
        let uses = [cut(0, 0, 2_000, true), cut(2_000, 0, 2_000, true)];
        assert_eq!(peak_held_bytes(&uses, &f), 60_000);
    }

    #[test]
    fn union_merges_overlaps() {
        assert_eq!(union_ms(vec![(0, 10), (5, 15), (20, 30)]), 25);
        assert_eq!(union_ms(vec![]), 0);
    }
}
