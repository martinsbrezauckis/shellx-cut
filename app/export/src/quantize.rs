//! Milliseconds to integer-frame quantization, preserving exact frame bounds.
//! Every NLE format is frame-addressed; our timeline is measured in milliseconds.
//!
//! Rules implemented exactly as specified:
//! 1. Quantize each clip's SOURCE boundaries (in/out independently, one
//!    rounding mode: round-half-away-from-zero = f64::round).
//! 2. Timeline positions = RUNNING SUM of quantized durations — never quantize
//!    timeline ms per clip independently (creates 1-frame gaps/overlaps that
//!    NLEs reject or silently "fix").
//! 3. Gap clips contribute round(duration_ms * fps / 1000) frames to the sum.
//!
//! All serializers consume the same [`Quantized`] result, so the invariant
//! "Σdur == last end, no overlaps, dur > 0" holds for every format at once.

use crate::error::ExportError;
use crate::model::{ExportSpeedRamp, ExportTimeline};

/// Timeline timebase as a rational fps (num/den): 30/1, 30000/1001, …
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timebase {
    pub num: i64,
    pub den: i64,
}

impl Timebase {
    /// Derive the rational timebase from the settings fps float.
    /// Integer fps -> fps/1; NTSC family (23.976, 29.97, 59.94, given as the
    /// usual rounded floats) -> X*1000/1001. Anything else is unrepresentable
    /// in xmeml's timebase+ntsc pair -> rejected for cross-format consistency.
    pub fn from_fps(fps: f64) -> Result<Self, ExportError> {
        if fps <= 0.0 || !fps.is_finite() {
            return Err(ExportError::BadFps(fps));
        }
        const FPS_EPSILON: f64 = 1e-3;
        // Integer fps (tolerance covers float noise like 30.000000001).
        if (fps - fps.round()).abs() < FPS_EPSILON {
            return Ok(Self {
                num: fps.round() as i64,
                den: 1,
            });
        }
        // NTSC: fps * 1001/1000 lands on an integer (29.97 -> 29.99997 ~ 30).
        let ntsc = fps * 1001.0 / 1000.0;
        if (ntsc - ntsc.round()).abs() < FPS_EPSILON {
            return Ok(Self {
                num: ntsc.round() as i64 * 1000,
                den: 1001,
            });
        }
        Err(ExportError::BadFps(fps))
    }

    /// ms -> frames at this timebase, round-half-up for non-negative times.
    pub fn frames_from_ms(&self, ms: u64) -> Result<i64, ExportError> {
        let fps_num = u128::try_from(self.num).map_err(|_| ExportError::BadFps(self.fps_f64()))?;
        let fps_den = u128::try_from(self.den).map_err(|_| ExportError::BadFps(self.fps_f64()))?;
        let denom = fps_den.checked_mul(1000).ok_or(ExportError::TimeOverflow {
            ms,
            fps_num: self.num,
            fps_den: self.den,
        })?;
        let numerator = (ms as u128)
            .checked_mul(fps_num)
            .ok_or(ExportError::TimeOverflow {
                ms,
                fps_num: self.num,
                fps_den: self.den,
            })?;
        let rounded = numerator
            .checked_add(denom / 2)
            .ok_or(ExportError::TimeOverflow {
                ms,
                fps_num: self.num,
                fps_den: self.den,
            })?
            / denom;
        if rounded > i64::MAX as u128 {
            return Err(ExportError::TimeOverflow {
                ms,
                fps_num: self.num,
                fps_den: self.den,
            });
        }
        Ok(rounded as i64)
    }

    /// Exact fps as f64 (MLT wall-clock timecodes).
    pub fn fps_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Rounded integer fps — SMPTE frame math and xmeml timebase.
    pub fn rounded(&self) -> i64 {
        self.fps_f64().round() as i64
    }

    /// True for NTSC rationals (den 1001) — xmeml `<ntsc>` flag.
    pub fn is_ntsc(&self) -> bool {
        self.den != 1
    }
}

/// One media clip in frame units.
/// (PartialEq only — gain_db is f64, so no Eq.)
#[derive(Debug, Clone, PartialEq)]
pub struct XClip {
    /// Key into the timeline assets map.
    pub asset: String,
    /// Timeline position, frames @ tb (running-sum derived).
    pub start: i64,
    /// Length, frames @ tb.
    pub dur: i64,
    /// Source in-point, frames @ tb.
    pub offset: i64,
    /// Audio stream index in source (currently stream 0).
    pub stream: i32,
    /// Per-clip gain (MLT volume filter — the only format exporting it).
    pub gain_db: f64,
    /// Original clip id, for error messages and op records.
    pub clip_id: String,
}

/// Track item: a media clip or a gap (formats emit their own gap construct).
#[derive(Debug, Clone, PartialEq)]
pub enum XItem {
    Clip(XClip),
    Gap { start: i64, dur: i64 },
}

impl XItem {
    /// End position on the timeline (frames, exclusive).
    pub fn end(&self) -> i64 {
        match self {
            XItem::Clip(c) => c.start + c.dur,
            XItem::Gap { start, dur } => start + dur,
        }
    }
}

/// Quantized EDL: everything the serializers need, in frames.
#[derive(Debug, Clone)]
pub struct Quantized {
    pub tb: Timebase,
    /// First video track's items (the interchange spine uses one video track).
    pub video: Vec<XItem>,
    /// Every video track, in timeline order. `video` remains the first layer for
    /// older serializers/tests; this list is used by formats that can emit
    /// multiple visual layers.
    pub video_tracks: Vec<QTrack>,
    /// Audio tracks ("layers"), in timeline order.
    pub audio: Vec<Vec<XItem>>,
    /// Audio tracks with ids, in timeline order.
    pub audio_tracks: Vec<QTrack>,
    /// Total timeline length in frames (max track end).
    pub total_frames: i64,
}

#[derive(Debug, Clone)]
pub struct QTrack {
    pub id: String,
    pub items: Vec<XItem>,
}

impl Quantized {
    /// Media clips only (gaps skipped) from a track's items.
    pub fn clips(items: &[XItem]) -> Vec<&XClip> {
        items
            .iter()
            .filter_map(|i| match i {
                XItem::Clip(c) => Some(c),
                XItem::Gap { .. } => None,
            })
            .collect()
    }
}

/// Quantize the whole timeline under the export mapping rules. Caption tracks are
/// ignored here (they export via SRT only). Errors carry clip id + ms + fps.
pub fn quantize(tl: &ExportTimeline) -> Result<Quantized, ExportError> {
    let tb = Timebase::from_fps(tl.settings.fps)?;
    let mut video: Vec<XItem> = Vec::new();
    let mut video_tracks: Vec<QTrack> = Vec::new();
    let mut audio: Vec<Vec<XItem>> = Vec::new();
    let mut audio_tracks: Vec<QTrack> = Vec::new();

    for track in &tl.tracks {
        match track.kind.as_str() {
            "video" => {
                let items = quantize_track(tl, &tb, track)?;
                if video.is_empty() {
                    video = items.clone();
                }
                video_tracks.push(QTrack {
                    id: track.id.clone(),
                    items,
                });
            }
            "audio" => {
                let items = quantize_track(tl, &tb, track)?;
                audio.push(items.clone());
                audio_tracks.push(QTrack {
                    id: track.id.clone(),
                    items,
                });
            }
            _ => {} // caption + unknown kinds: not frame-quantized
        }
    }

    let total_frames = video_tracks
        .iter()
        .filter_map(|t| t.items.last().map(XItem::end))
        .chain(audio.iter().filter_map(|t| t.last().map(XItem::end)))
        .max()
        .unwrap_or(0);

    Ok(Quantized {
        tb,
        video,
        video_tracks,
        audio,
        audio_tracks,
        total_frames,
    })
}

/// Quantize one track: source boundaries per clip, positions by running sum.
fn quantize_track(
    tl: &ExportTimeline,
    tb: &Timebase,
    track: &crate::model::ExportTrack,
) -> Result<Vec<XItem>, ExportError> {
    let mut items = Vec::new();
    let mut pos: i64 = 0; // running sum — rule 2

    for clip in &track.clips {
        if clip.is_gap() {
            let clip_id = clip.id_str();
            let Some(duration_ms) = clip.duration_ms else {
                return Err(ExportError::BadClip {
                    clip_id,
                    cause: "gap clip missing positive duration_ms".to_string(),
                });
            };
            if duration_ms == 0 {
                return Err(ExportError::BadClip {
                    clip_id,
                    cause: "gap clip duration_ms must be positive".to_string(),
                });
            }
            let dur = tb.frames_from_ms(duration_ms)?;
            if dur <= 0 {
                return Err(ExportError::BadClip {
                    clip_id,
                    cause: "gap clip is shorter than one export frame".to_string(),
                });
            }
            items.push(XItem::Gap { start: pos, dur });
            pos = pos.checked_add(dur).ok_or_else(|| ExportError::BadClip {
                clip_id: clip.id_str(),
                cause: "timeline frame position overflow after gap".to_string(),
            })?;
            continue;
        }

        let clip_id = clip.id_str();
        let asset = clip.asset.clone().ok_or_else(|| ExportError::BadClip {
            clip_id: clip_id.clone(),
            cause: "media clip missing 'asset'".to_string(),
        })?;
        if !tl.assets.contains_key(&asset) {
            return Err(ExportError::MissingAsset { clip_id, asset });
        }
        let (src_in_ms, src_out_ms) = match (clip.src_in_ms, clip.src_out_ms) {
            (Some(i), Some(o)) => (i, o),
            _ => {
                return Err(ExportError::BadClip {
                    clip_id,
                    cause: "media clip missing src_in_ms/src_out_ms".to_string(),
                })
            }
        };

        // Rule 1: quantize SOURCE boundaries; duration from the frame delta.
        let in_f = tb.frames_from_ms(src_in_ms)?;
        let out_f = tb.frames_from_ms(src_out_ms)?;
        let src_frames = out_f - in_f;
        if src_frames <= 0 {
            return Err(ExportError::EmptyClip {
                clip_id,
                src_in_ms,
                src_out_ms,
                fps: tl.settings.fps,
            });
        }
        let dur = if clip.speed_ramp.is_none()
            && ((clip.speed - 1.0).abs() <= f64::EPSILON
                || !clip.speed.is_finite()
                || clip.speed <= 0.0)
        {
            src_frames
        } else {
            let timeline_ms = timeline_duration_ms(clip, src_in_ms, src_out_ms)?;
            tb.frames_from_ms(timeline_ms)?
        };
        if dur <= 0 {
            return Err(ExportError::EmptyClip {
                clip_id,
                src_in_ms,
                src_out_ms,
                fps: tl.settings.fps,
            });
        }

        items.push(XItem::Clip(XClip {
            asset,
            start: pos,
            dur,
            offset: in_f,
            stream: clip.stream.unwrap_or(0),
            gain_db: clip.gain_db.unwrap_or(0.0),
            clip_id,
        }));
        pos = pos.checked_add(dur).ok_or_else(|| ExportError::BadClip {
            clip_id: clip.id_str(),
            cause: "timeline frame position overflow after clip".to_string(),
        })?;
    }
    Ok(items)
}

fn timeline_duration_ms(
    clip: &crate::model::ExportClip,
    src_in_ms: u64,
    src_out_ms: u64,
) -> Result<u64, ExportError> {
    let src_span = src_out_ms.saturating_sub(src_in_ms);
    if let Some(ramp) = clip.speed_ramp.as_ref() {
        return speed_ramp_duration_ms(src_in_ms, src_out_ms, ramp, &clip.id_str());
    }
    Ok(src_off_to_tl(src_span, clip.speed))
}

fn src_off_to_tl(src_off: u64, speed: f64) -> u64 {
    if speed == 1.0 {
        return src_off;
    }
    if speed.is_finite() && speed > 0.0 {
        (src_off as f64 / speed).round() as u64
    } else {
        src_off
    }
}

fn speed_ramp_duration_ms(
    src_in_ms: u64,
    src_out_ms: u64,
    ramp: &ExportSpeedRamp,
    clip_id: &str,
) -> Result<u64, ExportError> {
    if ramp.points.len() < 2 || ramp.segments < 2 {
        return Err(ExportError::BadClip {
            clip_id: clip_id.to_string(),
            cause: "speed_ramp needs at least two points and two segments".to_string(),
        });
    }
    if ramp.points.windows(2).any(|w| w[0].at_ms >= w[1].at_ms) {
        return Err(ExportError::BadClip {
            clip_id: clip_id.to_string(),
            cause: "speed_ramp points must be sorted by increasing at_ms".to_string(),
        });
    }
    if ramp
        .points
        .iter()
        .any(|p| !p.factor.is_finite() || p.factor <= 0.0)
    {
        return Err(ExportError::BadClip {
            clip_id: clip_id.to_string(),
            cause: "speed_ramp factors must be finite and positive".to_string(),
        });
    }

    let span = src_out_ms.saturating_sub(src_in_ms);
    if span == 0 {
        return Ok(0);
    }
    let mut total = 0u64;
    for i in 0..ramp.segments {
        let a_off = (span as u128 * i as u128 / ramp.segments as u128) as u64;
        let b_off = (span as u128 * (i + 1) as u128 / ramp.segments as u128) as u64;
        if b_off <= a_off {
            continue;
        }
        let mid = a_off + (b_off - a_off) / 2;
        let speed = speed_ramp_factor_at(ramp, mid);
        total = total.saturating_add(src_off_to_tl(b_off - a_off, speed));
    }
    Ok(total)
}

fn speed_ramp_factor_at(ramp: &ExportSpeedRamp, off: u64) -> f64 {
    let first = &ramp.points[0];
    if off <= first.at_ms {
        return first.factor;
    }
    for pair in ramp.points.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if off <= b.at_ms {
            let span = b.at_ms.saturating_sub(a.at_ms).max(1) as f64;
            let t = (off.saturating_sub(a.at_ms) as f64 / span).clamp(0.0, 1.0);
            return a.factor + (b.factor - a.factor) * t;
        }
    }
    ramp.points.last().map(|p| p.factor).unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_timeline;
    use serde_json::json;

    /// Build a minimal timeline with the given fps and (in,out) ms clip pairs.
    fn tl(fps: f64, clips: &[(u64, u64)]) -> ExportTimeline {
        let clips_json: Vec<_> = clips
            .iter()
            .enumerate()
            .map(|(i, (a, b))| json!({"id": format!("c{i}"), "asset": "a1", "src_in_ms": a, "src_out_ms": b}))
            .collect();
        parse_timeline(&json!({
            "settings": {"width": 1920, "height": 1080, "fps": fps, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/talk.mp4"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": clips_json}]
        }))
        .unwrap()
    }

    #[test]
    fn cumulative_invariant_holds_at_2997() {
        // Word-boundary-ish ragged ms ranges at NTSC fps: positions must be
        // gapless (running sum), Σdur == last end, all durs positive.
        let t = tl(
            29.97,
            &[(0, 1234), (5678, 9012), (10010, 10500), (20000, 60000)],
        );
        let q = quantize(&t).unwrap();
        let clips = Quantized::clips(&q.video);
        let mut expect_start = 0;
        for c in &clips {
            assert_eq!(c.start, expect_start, "gap/overlap at clip {}", c.clip_id);
            assert!(c.dur > 0);
            expect_start += c.dur;
        }
        assert_eq!(q.total_frames, expect_start);
        assert_eq!(
            q.tb,
            Timebase {
                num: 30000,
                den: 1001
            }
        );
    }

    #[test]
    fn timebase_rejects_borderline_non_ntsc_fps() {
        assert_eq!(
            Timebase::from_fps(23.976).unwrap(),
            Timebase {
                num: 24_000,
                den: 1001
            }
        );
        assert_eq!(
            Timebase::from_fps(29.97).unwrap(),
            Timebase {
                num: 30_000,
                den: 1001
            }
        );
        assert_eq!(
            Timebase::from_fps(59.94).unwrap(),
            Timebase {
                num: 60_000,
                den: 1001
            }
        );
        assert!(
            matches!(Timebase::from_fps(29.961), Err(ExportError::BadFps(_))),
            "near-NTSC values outside the shared tolerance must not be mislabeled as 30000/1001"
        );
    }

    #[test]
    fn subframe_clip_is_actionable_error() {
        let t = tl(30.0, &[(100, 110)]); // 10ms < one 33ms frame
        let err = quantize(&t).unwrap_err();
        assert!(matches!(err, ExportError::EmptyClip { .. }), "got {err:?}");
        assert!(err.to_string().contains("c0"), "error must name the clip");
    }

    #[test]
    fn gap_contributes_frames() {
        let t = parse_timeline(&json!({
            "settings": {"fps": 30},
            "assets": {"a1": {"path": "/m/talk.mp4"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000},
                {"kind": "gap", "duration_ms": 500},
                {"id": "c2", "asset": "a1", "src_in_ms": 2000, "src_out_ms": 3000}
            ]}]
        }))
        .unwrap();
        let q = quantize(&t).unwrap();
        assert_eq!(q.video.len(), 3);
        assert_eq!(q.video[1], XItem::Gap { start: 30, dur: 15 });
        match &q.video[2] {
            XItem::Clip(c) => assert_eq!(c.start, 45), // resumes after the gap
            other => unreachable!("expected clip, got {other:?}"),
        }
        assert_eq!(q.total_frames, 75);
    }

    #[test]
    fn malformed_gap_duration_is_actionable_error() {
        for gap in [
            json!({"id": "g_missing", "kind": "gap"}),
            json!({"id": "g_zero", "kind": "gap", "duration_ms": 0}),
        ] {
            let t = parse_timeline(&json!({
                "settings": {"fps": 30},
                "assets": {"a1": {"path": "/m/talk.mp4"}},
                "tracks": [{"id": "v1", "kind": "video", "clips": [gap]}]
            }))
            .unwrap();
            let err = quantize(&t).unwrap_err();
            assert!(
                matches!(err, ExportError::BadClip { .. }),
                "malformed gap should not be silently dropped: {err:?}"
            );
        }
    }

    #[test]
    fn huge_ms_values_do_not_saturate_into_frame_counts() {
        let t = parse_timeline(&json!({
            "settings": {"fps": 1_000_000.0},
            "assets": {"a1": {"path": "/m/talk.mp4"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "huge", "asset": "a1", "src_in_ms": 0_u64, "src_out_ms": u64::MAX}
            ]}]
        }))
        .unwrap();
        let err = quantize(&t).unwrap_err();
        assert!(
            matches!(err, ExportError::TimeOverflow { .. }),
            "huge frame counts should error instead of saturating: {err:?}"
        );
    }

    #[test]
    fn speed_changes_advance_record_cursor_by_timeline_duration() {
        let t = parse_timeline(&json!({
            "settings": {"fps": 30},
            "assets": {"a1": {"path": "/m/talk.mp4"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "fast", "asset": "a1", "src_in_ms": 0, "src_out_ms": 10000, "speed": 2.0},
                {"id": "next", "asset": "a1", "src_in_ms": 10000, "src_out_ms": 11000}
            ]}]
        }))
        .unwrap();
        let q = quantize(&t).unwrap();
        let clips = Quantized::clips(&q.video);
        assert_eq!(clips[0].dur, 150, "10s at 2x occupies 5s @30fps");
        assert_eq!(clips[1].start, 150, "next clip starts after retimed slot");
        assert_eq!(q.total_frames, 180);
    }

    #[test]
    fn speed_ramp_changes_advance_record_cursor_by_integrated_duration() {
        let t = parse_timeline(&json!({
            "settings": {"fps": 30},
            "assets": {"a1": {"path": "/m/talk.mp4"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "ramp", "asset": "a1", "src_in_ms": 0, "src_out_ms": 4000,
                 "speed_ramp": {"segments": 2, "points": [
                    {"at_ms": 0, "factor": 2.0},
                    {"at_ms": 4000, "factor": 2.0}
                 ]}},
                {"id": "next", "asset": "a1", "src_in_ms": 4000, "src_out_ms": 5000}
            ]}]
        }))
        .unwrap();
        let q = quantize(&t).unwrap();
        let clips = Quantized::clips(&q.video);
        assert_eq!(clips[0].dur, 60, "4s at 2x occupies 2s @30fps");
        assert_eq!(clips[1].start, 60);
    }

    #[test]
    fn quantize_keeps_and_validates_all_video_tracks() {
        let t = parse_timeline(&json!({
            "settings": {"fps": 30},
            "assets": {
                "a1": {"path": "/m/base.mp4"},
                "a2": {"path": "/m/overlay.mp4"}
            },
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "base", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}
                ]},
                {"id": "v2", "kind": "video", "clips": [
                    {"id": "overlay", "asset": "a2", "src_in_ms": 0, "src_out_ms": 500}
                ]}
            ]
        }))
        .unwrap();
        let q = quantize(&t).unwrap();
        assert_eq!(q.video_tracks.len(), 2);
        assert_eq!(q.video_tracks[0].id, "v1");
        assert_eq!(q.video_tracks[1].id, "v2");
        assert_eq!(
            Quantized::clips(&q.video_tracks[1].items)[0].clip_id,
            "overlay"
        );

        let bad = parse_timeline(&json!({
            "settings": {"fps": 30},
            "assets": {"a1": {"path": "/m/base.mp4"}},
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "base", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}
                ]},
                {"id": "v2", "kind": "video", "clips": [
                    {"id": "missing", "asset": "missing_asset", "src_in_ms": 0, "src_out_ms": 500}
                ]}
            ]
        }))
        .unwrap();
        assert!(matches!(
            quantize(&bad).unwrap_err(),
            ExportError::MissingAsset { .. }
        ));
    }
}
