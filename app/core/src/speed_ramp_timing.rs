//! Frame-grid timing for expanded variable-speed ramps.
//!
//! A ramp is rendered as several constant-speed ffmpeg segments. Those
//! segments must share one output-frame budget: rounding each millisecond slice
//! independently can lose a frame when ffmpeg concats them. This module assigns
//! frames from cumulative ramp time, so the model, EDL, video, and audio agree.

use crate::types::{Clip, Project, RampSeg, SpeedRamp};

fn usable_fps(fps: Option<f64>) -> Option<f64> {
    fps.filter(|value| value.is_finite() && *value > 0.0)
}

/// Nearest whole output frames for a timeline duration.
pub fn timeline_frame_count(duration_ms: u64, fps: f64) -> u64 {
    if duration_ms == 0 || !fps.is_finite() || fps <= 0.0 {
        return 0;
    }
    ((duration_ms as f64 * fps / 1000.0).round() as u64).max(1)
}

/// Milliseconds represented by a frame count, rounded once at its boundary.
pub fn timeline_duration_ms_for_frames(frames: u64, fps: f64) -> u64 {
    if !fps.is_finite() || fps <= 0.0 {
        return 0;
    }
    (frames as f64 * 1000.0 / fps).round() as u64
}

/// Nearest audio samples for a frame-grid duration.
pub fn timeline_sample_count_for_frames(frames: u64, fps: f64, sample_rate: u32) -> u64 {
    if !fps.is_finite() || fps <= 0.0 || sample_rate == 0 {
        return 0;
    }
    (frames as f64 * sample_rate as f64 / fps).round() as u64
}

/// Apply the established minimum-output-frame cap to one ramp at a known grid.
pub(crate) fn clamp_speed_ramp_segments(
    src_in: u64,
    src_out: u64,
    ramp: &SpeedRamp,
    fps: f64,
) -> usize {
    let src_dur = src_out.saturating_sub(src_in);
    let max_factor = ramp
        .points
        .iter()
        .map(|point| point.factor)
        .fold(1.0_f64, f64::max);
    let frame_cap = ((src_dur as f64 * fps)
        / (max_factor * crate::types::MIN_FRAMES_PER_SUBSEG as f64 * 1000.0))
        .floor() as usize;
    ramp.preferred_segments
        .unwrap_or(ramp.segments)
        .min(frame_cap.max(crate::types::MIN_RAMP_SEGMENTS))
        .clamp(
            crate::types::MIN_RAMP_SEGMENTS,
            crate::types::MAX_RAMP_SEGMENTS,
        )
}

/// Regrid new frame-aware ramps after a project format change. The retained
/// preference restores detail when the output grid later permits it. Legacy ramps
/// deliberately have no persisted timebase and retain their millisecond replay.
pub(crate) fn regrid_timebased_speed_ramps(project: &mut Project, fps: f64, audio_rate: u32) {
    let Some(fps) = usable_fps(Some(fps)) else {
        return;
    };
    let audio_rate = (audio_rate > 0).then_some(audio_rate);
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            let Clip::Media(clip) = clip else {
                continue;
            };
            let Some(ramp) = clip.speed_ramp.as_mut() else {
                continue;
            };
            if ramp.timebase_fps.is_none() {
                continue;
            }
            // Upgrade an older frame-aware cache to retain the effective value it
            // already had. New journal replays carry the original request through
            // `edit::speed_ramp`; this fallback prevents any further loss for a
            // cache written before `preferred_segments` existed.
            ramp.preferred_segments = Some(ramp.preferred_segments.unwrap_or(ramp.segments).clamp(
                crate::types::MIN_RAMP_SEGMENTS,
                crate::types::MAX_RAMP_SEGMENTS,
            ));
            ramp.timebase_fps = Some(fps);
            ramp.timebase_audio_rate = audio_rate;
            ramp.segments = clamp_speed_ramp_segments(clip.src_in_ms, clip.src_out_ms, ramp, fps);
        }
    }
}

fn raw_slices(src_in: u64, src_out: u64, ramp: &SpeedRamp) -> Vec<(u64, u64, f64)> {
    let src_dur = src_out.saturating_sub(src_in);
    if src_dur == 0 {
        return Vec::new();
    }
    let n = ramp.segments.max(1) as u128;
    let span = src_dur as u128;
    (0..n)
        .filter_map(|i| {
            let a_off = (i * span / n) as u64;
            let b_off = ((i + 1) * span / n) as u64;
            (b_off > a_off).then(|| {
                let speed = crate::types::speed_ramp_factor_at(ramp, (a_off + b_off) / 2);
                (src_in + a_off, src_in + b_off, speed)
            })
        })
        .collect()
}

fn legacy_segments(raw: &[(u64, u64, f64)]) -> Vec<RampSeg> {
    let mut out: Vec<RampSeg> = Vec::new();
    let mut pending_src_in: Option<u64> = None;
    for &(src_in, src_out, speed) in raw {
        let seg_src_in = pending_src_in.take().unwrap_or(src_in);
        let dur_ms = crate::types::src_off_to_tl(src_out - src_in, speed);
        if dur_ms == 0 {
            if let Some(prev) = out.last_mut() {
                prev.src_out = src_out;
            } else {
                pending_src_in = Some(seg_src_in);
            }
            continue;
        }
        out.push(RampSeg {
            src_in: seg_src_in,
            src_out,
            speed,
            dur_ms,
            frame_count: None,
            sample_count: None,
        });
    }
    out
}

fn frame_grid_segments(raw: &[(u64, u64, f64)], fps: f64, audio_rate: Option<u32>) -> Vec<RampSeg> {
    let raw_total_ms: f64 = raw
        .iter()
        .map(|(src_in, src_out, speed)| (src_out - src_in) as f64 / speed)
        .sum();
    let total_frames = (raw_total_ms * fps / 1000.0).round().max(1.0) as u64;
    let mut out: Vec<RampSeg> = Vec::new();
    let mut pending_src_in: Option<u64> = None;
    let mut raw_elapsed_ms = 0.0;
    let mut prior_frames = 0u64;
    let mut prior_samples = 0u64;

    for (index, &(src_in, src_out, speed)) in raw.iter().enumerate() {
        raw_elapsed_ms += (src_out - src_in) as f64 / speed;
        let end_frames = if index + 1 == raw.len() {
            total_frames
        } else {
            (raw_elapsed_ms * fps / 1000.0)
                .round()
                .clamp(prior_frames as f64, total_frames as f64) as u64
        };
        let frame_count = end_frames.saturating_sub(prior_frames);
        let seg_src_in = pending_src_in.take().unwrap_or(src_in);
        if frame_count == 0 {
            if let Some(prev) = out.last_mut() {
                prev.src_out = src_out;
            } else {
                pending_src_in = Some(seg_src_in);
            }
            continue;
        }
        let dur_ms = timeline_duration_ms_for_frames(end_frames, fps)
            .saturating_sub(timeline_duration_ms_for_frames(prior_frames, fps));
        let sample_count = audio_rate.map(|rate| {
            let end_samples = timeline_sample_count_for_frames(end_frames, fps, rate);
            let count = end_samples.saturating_sub(prior_samples);
            prior_samples = end_samples;
            count
        });
        out.push(RampSeg {
            src_in: seg_src_in,
            src_out,
            speed,
            dur_ms,
            frame_count: Some(frame_count),
            sample_count,
        });
        prior_frames = end_frames;
    }
    out
}

/// Expand a ramp into constant-speed slices. New ramps store the project frame
/// rate; old projects without it retain their historical millisecond replay.
pub fn speed_ramp_segments(src_in: u64, src_out: u64, ramp: &SpeedRamp) -> Vec<RampSeg> {
    let raw = raw_slices(src_in, src_out, ramp);
    match usable_fps(ramp.timebase_fps) {
        Some(fps) => frame_grid_segments(&raw, fps, ramp.timebase_audio_rate),
        None => legacy_segments(&raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SpeedRampPoint;

    fn ramp(fps: f64, segments: usize) -> SpeedRamp {
        SpeedRamp {
            points: vec![
                SpeedRampPoint {
                    at_ms: 0,
                    factor: 0.5,
                },
                SpeedRampPoint {
                    at_ms: 2000,
                    factor: 2.0,
                },
                SpeedRampPoint {
                    at_ms: 4000,
                    factor: 0.5,
                },
            ],
            segments,
            preferred_segments: Some(segments),
            timebase_fps: Some(fps),
            timebase_audio_rate: Some(48_000),
        }
    }

    #[test]
    fn representative_ramp_uses_one_cumulative_frame_budget() {
        let fps = 30.0;
        let segs = speed_ramp_segments(0, 4000, &ramp(fps, 24));
        let frames: u64 = segs.iter().map(|seg| seg.frame_count.unwrap()).sum();
        let duration: u64 = segs.iter().map(|seg| seg.dur_ms).sum();
        assert_eq!(frames, timeline_frame_count(duration, fps));
        assert_eq!(duration, timeline_duration_ms_for_frames(frames, fps));
        assert_eq!(segs.first().unwrap().src_in, 0);
        assert_eq!(segs.last().unwrap().src_out, 4000);
    }

    #[test]
    fn ntsc_and_short_endpoints_remain_contiguous() {
        let fps = 30_000.0 / 1001.0;
        let segs = speed_ramp_segments(7, 101, &ramp(fps, 24));
        assert_eq!(segs.first().unwrap().src_in, 7);
        assert_eq!(segs.last().unwrap().src_out, 101);
        for pair in segs.windows(2) {
            assert_eq!(pair[0].src_out, pair[1].src_in);
            assert!(pair[0].frame_count.unwrap() > 0);
        }
        let frames: u64 = segs.iter().map(|seg| seg.frame_count.unwrap()).sum();
        assert_eq!(
            segs.iter().map(|seg| seg.dur_ms).sum::<u64>(),
            timeline_duration_ms_for_frames(frames, fps)
        );
    }

    #[test]
    fn frame_and_sample_budgets_hold_for_representative_segment_counts() {
        for fps in [24.0, 30.0, 30_000.0 / 1001.0, 59.94] {
            for segments in [2, 3, 7, 24] {
                let segs = speed_ramp_segments(0, 4000, &ramp(fps, segments));
                let frames: u64 = segs.iter().map(|seg| seg.frame_count.unwrap()).sum();
                let samples: u64 = segs.iter().map(|seg| seg.sample_count.unwrap()).sum();
                assert_eq!(
                    samples,
                    timeline_sample_count_for_frames(frames, fps, 48_000),
                    "fps={fps} segments={segments}"
                );
            }
        }
    }

    #[test]
    fn legacy_ramps_keep_their_per_slice_millisecond_replay() {
        let mut legacy = ramp(30.0, 3);
        legacy.preferred_segments = None;
        legacy.timebase_fps = None;
        legacy.timebase_audio_rate = None;
        let segs = speed_ramp_segments(0, 4000, &legacy);
        let expected: u64 = segs
            .iter()
            .map(|seg| crate::types::src_off_to_tl(seg.src_out - seg.src_in, seg.speed))
            .sum();
        assert_eq!(segs.iter().map(|seg| seg.dur_ms).sum::<u64>(), expected);
        assert!(segs.iter().all(|seg| seg.frame_count.is_none()));
        assert!(segs.iter().all(|seg| seg.sample_count.is_none()));
    }

    #[test]
    fn older_frame_aware_cache_retains_its_existing_effective_detail() {
        let mut project = Project::new("old", Default::default());
        let mut media = crate::edit::make_media_clip("c1", "a1", 0, 5000);
        let mut old_cache_ramp = ramp(60.0, 25);
        old_cache_ramp.preferred_segments = None;
        media.speed_ramp = Some(old_cache_ramp);
        project.track_mut("v1").unwrap().clips = vec![Clip::Media(media)];

        regrid_timebased_speed_ramps(&mut project, 24.0, 48_000);
        let Clip::Media(media) = &project.track("v1").unwrap().clips[0] else {
            unreachable!()
        };
        let ramp = media.speed_ramp.as_ref().unwrap();
        assert_eq!(ramp.preferred_segments, Some(25));
        assert_eq!(
            ramp.segments, 15,
            "24fps cap applies to the preserved detail"
        );
    }

    #[test]
    fn zero_duration_has_no_output_frames_or_samples() {
        assert_eq!(timeline_frame_count(0, 24.0), 0);
        assert_eq!(timeline_frame_count(0, 30_000.0 / 1001.0), 0);
        assert_eq!(timeline_duration_ms_for_frames(0, 30.0), 0);
        assert_eq!(timeline_sample_count_for_frames(0, 30.0, 48_000), 0);
    }
}
