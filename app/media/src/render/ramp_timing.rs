//! FFmpeg timing locks for frame-grid speed-ramp EDL segments.
//!
//! The core assigns each new ramp slice a cumulative output-frame and audio-
//! sample budget. FFmpeg still resamples every slice independently, so these
//! suffixes pad/trim each generated stream to that same budget before concat.

use cut_core::{Clip, EdlSegment, Project};

struct Budget {
    frames: u64,
    samples: Option<u64>,
    fps: f64,
}

fn budget(project: &Project, segment: &EdlSegment) -> Option<Budget> {
    let clip_id = segment.clip_id.as_deref()?;
    let (track_id, index) = project.find_clip(clip_id)?;
    let Clip::Media(clip) = project.track(track_id)?.clips.get(index)? else {
        return None;
    };
    let ramp = clip.speed_ramp.as_ref()?;
    let fps = ramp.timebase_fps?;
    cut_core::speed_ramp_segments(clip.src_in_ms, clip.src_out_ms, ramp)
        .into_iter()
        .find(|slice| {
            slice.src_in == segment.src_in_ms.unwrap_or_default()
                && slice.src_out == segment.src_out_ms.unwrap_or_default()
        })
        .and_then(|slice| {
            slice.frame_count.map(|frames| Budget {
                frames,
                samples: slice.sample_count,
                fps,
            })
        })
}

/// Lock a ramp's video segment to its core-assigned frame count. `tpad` makes
/// up a source-frame boundary shortfall and `trim=end_frame` removes any excess.
pub fn video_suffix(project: &Project, segment: &EdlSegment) -> String {
    let Some(budget) = budget(project, segment) else {
        return String::new();
    };
    let duration_ms = cut_core::timeline_duration_ms_for_frames(budget.frames, budget.fps);
    format!(
        ",tpad=stop_mode=clone:stop_duration={:.3},trim=end_frame={},setpts=PTS-STARTPTS",
        duration_ms as f64 / 1000.0,
        budget.frames
    )
}

/// Lock a ramp's audio segment to the matching sample count. The pad is only
/// ever a boundary correction; the following trim makes the count exact.
pub fn audio_suffix(project: &Project, segment: &EdlSegment) -> String {
    let Some(samples) = budget(project, segment).and_then(|budget| budget.samples) else {
        return String::new();
    };
    format!(",apad=pad_len={samples},atrim=end_sample={samples},asetpts=N/SR/TB")
}
