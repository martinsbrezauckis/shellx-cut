//! Key times belong to the whole clip, not to the current render window.
use cut_core::{EdlSegment, KfInterp, KfParam};

pub(super) fn offset(seg: &EdlSegment, fps: f64) -> f64 {
    let Some(place) = seg.window else {
        return 0.0;
    };
    let first = super::frame_budget::frame_at(place.clip_timeline_in_ms, fps);
    let part = super::frame_budget::frame_at(place.origin_ms + seg.timeline_in_ms, fps);
    part.saturating_sub(first) as f64 / fps
}

pub(super) fn points(
    seg: &EdlSegment,
    param: KfParam,
    fps: f64,
) -> Option<(Vec<(f64, f64)>, KfInterp)> {
    let (mut points, interp) = super::kf_points(&seg.keyframes, param)?;
    let offset = offset(seg, fps);
    // Signed seconds preserve the full interpolation curve across the cut,
    // including its preceding point. Saturating u64 key times would not.
    for (time, _) in &mut points {
        *time -= offset;
    }
    Some((points, interp))
}

pub(super) fn opacity_cmd(
    seg: &EdlSegment,
    fps: f64,
    dur_ms: u64,
    uniq: &str,
    keep_hdr: bool,
    cmd_files: Option<&mut Vec<tempfile::TempPath>>,
) -> Result<String, cut_core::CutError> {
    let Some((pts, interp)) = points(seg, KfParam::Opacity, fps) else {
        return Ok(String::new());
    };
    super::keyframe_cmd::opacity_kf_filter_cmd(&pts, interp, fps, dur_ms, uniq, keep_hdr, cmd_files)
}

/// Whether `param`'s curve is longer than the released expression could hold
/// ([`MAX_KEYFRAME_POINTS`](cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)).
pub(super) fn long(seg: &EdlSegment, param: KfParam) -> bool {
    seg.keyframes
        .iter()
        .any(|k| k.param == param && k.points.len() > cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)
}

/// Animated zoom. Curves the released expression could hold (at most
/// [`MAX_KEYFRAME_POINTS`](cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)) keep
/// its exact `zoompan` graph, so saved projects render unchanged; longer
/// curves read a per-frame crop from a command file.
pub(super) fn scale_cmd(
    seg: &EdlSegment,
    w: u32,
    h: u32,
    fps: f64,
    dur_ms: u64,
    uniq: &str,
    cmd_files: Option<&mut Vec<tempfile::TempPath>>,
) -> Result<Option<String>, cut_core::CutError> {
    let Some((pts, interp)) = points(seg, KfParam::Scale, fps) else {
        return Ok(None);
    };
    if !long(seg, KfParam::Scale) {
        return Ok(scale(seg, w, h, fps, dur_ms));
    }
    super::keyframe_cmd::scale_kf_cmd(&pts, interp, w, h, fps, dur_ms, uniq, cmd_files)
}

fn scale(seg: &EdlSegment, w: u32, h: u32, fps: f64, duration: u64) -> Option<String> {
    if offset(seg, fps) == 0.0 {
        return super::scale_kf_zoompan(&seg.keyframes, w, h, fps, duration);
    }
    let (points, interp) = points(seg, KfParam::Scale, fps)?;
    if points.is_empty() {
        return None;
    }
    let frames: Vec<_> = points
        .iter()
        .map(|(t, v)| ((t * fps).round(), v.clamp(1.0, 10.0)))
        .collect();
    let z = super::kf_expr(&frames, "on", interp);
    let rate = super::fps_str(fps);
    Some(format!(",fps={rate},zoompan=z='max(1,min(10,{z}))':d=1:x='iw/2-(iw/zoom)/2':y='ih/2-(ih/zoom)/2':s={w}x{h}:fps={rate},setpts=N/({rate}*TB)"))
}
