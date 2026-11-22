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

pub(super) fn opacity(seg: &EdlSegment, fps: f64) -> String {
    if offset(seg, fps) == 0.0 {
        return super::opacity_kf_filter(&seg.keyframes);
    }
    match points(seg, KfParam::Opacity, fps) {
        Some((points, interp)) if !points.is_empty() => format!(
            ",geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='255*clip(({}),0,1)'",
            super::kf_expr(&points, "T", interp)
        ),
        _ => String::new(),
    }
}

pub(super) fn scale(seg: &EdlSegment, w: u32, h: u32, fps: f64, duration: u64) -> Option<String> {
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
