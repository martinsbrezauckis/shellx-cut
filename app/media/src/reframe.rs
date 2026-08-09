//! reframe.rs — subject-aware auto-reframe crop path (perception contract reframe rework).
//!
//! Role: turn a per-frame SUBJECT TRACK (the `subject` perception instrument's
//! normalized, aspect-independent observations) + a TARGET ASPECT into a smoothed,
//! per-frame CROP RECT in source pixels. The render then drives an ffmpeg
//! `sendcmd`+`crop` pass from these rects — a
//! post-pass on the rendered edit, the industry "new sequence from the finished
//! edit" model.
//!
//! This module is the DETERMINISTIC core: a confidence-weighted critically-damped
//! spring (based on the MIT-licensed auto-vertical-reframe reference) that holds
//! the crop on static shots and follows a moving subject without jitter or jarring
//! jumps. Pure arithmetic, no I/O, no Python — fully unit-tested. The heavy CV that
//! produces the track lives in the sidecar (app/perception); the SAME track drives
//! any aspect (9:16 / 1:1 / 4:5) through this module.
//!
//! Callers: cut_media::render (the reframe ffmpeg pass) + the reframe verb. Inputs
//! are decoupled from cut_perception: the caller passes a slice of [`FrameObs`]
//! (mapped from the perception SubjectTrack) so this crate needs no perception dep.
//!
//! Dependencies: none beyond std.

/// One frame's reframe input — the subject's normalized FOCUS BOUNDS (the box to
/// keep in frame) + confidence + scene index. Mapped by the caller from a
/// `cut_perception::types::SubjectFrame` (fx1..fy2/conf/scene). `focus == None`
/// means no subject detected this frame → the crop holds the last target / centres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameObs {
    /// Normalized focus rect `[x1, y1, x2, y2]` ∈ [0,1]; `None` = no subject.
    pub focus: Option<[f64; 4]>,
    /// Detector confidence `[0,1]` (weights the spring's responsiveness).
    pub conf: f64,
    /// Scene index — the crop state resets at each new scene (a hard cut).
    pub scene: u32,
}

/// A crop rectangle in SOURCE pixels (the ffmpeg `crop=w:h:x:y` for this frame).
/// `w`/`h` are even (yuv420 requires it); `x`/`y` are the top-left, clamped on-frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub w: u32,
    pub h: u32,
    pub x: u32,
    pub y: u32,
}

/// Smoothing + zoom limits for the spring (per reframe preset). Defaults match the
/// reference's `talking_head`. `min_zoom > 1.0` keeps a slight push so the subject
/// fills the vertical frame; `max_zoom` caps how tight the crop can get.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReframeParams {
    pub min_zoom: f64,
    pub max_zoom: f64,
    pub max_step_x: f64,
    pub max_step_y: f64,
    pub target_alpha: f64,
    pub motion_response: f64,
    pub motion_damping: f64,
    pub zoom_response: f64,
    pub zoom_damping: f64,
    pub zoom_alpha: f64,
}

impl Default for ReframeParams {
    fn default() -> Self {
        // talking_head defaults (auto-vertical-reframe), the safe general starting point.
        ReframeParams {
            min_zoom: 1.05,
            max_zoom: 1.85,
            max_step_x: 7.0,
            max_step_y: 5.0,
            target_alpha: 0.12,
            motion_response: 0.12,
            motion_damping: 0.82,
            zoom_response: 0.08,
            zoom_damping: 0.85,
            zoom_alpha: 0.035,
        }
    }
}

impl ReframeParams {
    /// Per-preset params. Unknown name → talking_head default. Preset names match
    /// the `subject` instrument (`_SUBJECT_PRESETS`): talking_head/sports/pets/cars/general.
    pub fn for_preset(name: &str) -> Self {
        let d = ReframeParams::default();
        match name {
            // Wider subjects, faster motion, less zoom (action stays in frame).
            "sports" => ReframeParams {
                min_zoom: 1.0,
                max_zoom: 1.35,
                max_step_x: 12.0,
                max_step_y: 8.0,
                ..d
            },
            "pets" => ReframeParams {
                min_zoom: 1.0,
                max_zoom: 1.55,
                max_step_x: 10.0,
                max_step_y: 7.0,
                ..d
            },
            "cars" => ReframeParams {
                min_zoom: 1.0,
                max_zoom: 1.30,
                max_step_x: 11.0,
                max_step_y: 7.0,
                ..d
            },
            "general" => ReframeParams {
                min_zoom: 1.0,
                max_zoom: 1.6,
                max_step_x: 10.0,
                max_step_y: 7.0,
                ..d
            },
            _ => d, // talking_head
        }
    }
}

#[inline]
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[inline]
fn lerp(a: f64, b: f64, alpha: f64) -> f64 {
    a + (b - a) * alpha
}

/// The target-aspect rectangle at FULL zoom (zoom = 1.0) inside the source frame —
/// the largest `aspect_w:aspect_h` box that fits. Crop sizes shrink from this as the
/// spring zooms in. Ported from the reference's `compute_base_crop`.
pub fn base_crop(frame_w: u32, frame_h: u32, aspect_w: u32, aspect_h: u32) -> (u32, u32) {
    if frame_w == 0 || frame_h == 0 || aspect_w == 0 || aspect_h == 0 {
        return (0, 0);
    }

    let frame_w64 = u64::from(frame_w);
    let frame_h64 = u64::from(frame_h);
    let aspect_w64 = u64::from(aspect_w);
    let aspect_h64 = u64::from(aspect_h);
    if frame_w64 * aspect_h64 >= aspect_w64 * frame_h64 {
        let cw = round_div(frame_h64 * aspect_w64, aspect_h64).min(frame_w64);
        (u32::try_from(cw).unwrap_or(frame_w), frame_h)
    } else {
        let ch = round_div(frame_w64 * aspect_h64, aspect_w64).min(frame_h64);
        (frame_w, u32::try_from(ch).unwrap_or(frame_h))
    }
}

#[inline]
fn round_div(n: u64, d: u64) -> u64 {
    (n + (d / 2)) / d
}

/// Crop size at a given zoom (≥1), clamped to ≥64px and ≤ the frame. Ported from
/// the reference's `current_crop_size`.
fn crop_size(base_w: u32, base_h: u32, zoom: f64, frame_w: u32, frame_h: u32) -> (f64, f64) {
    let cw = (base_w as f64 / zoom).round();
    let ch = (base_h as f64 / zoom).round();
    (
        cw.max(64.0).min(frame_w as f64),
        ch.max(64.0).min(frame_h as f64),
    )
}

/// One spring observation: where the crop WANTS to be this frame (center + zoom)
/// and how much to trust it (confidence). Derived from the subject's focus bounds.
#[derive(Debug, Clone, Copy)]
struct Observation {
    center_x: f64,
    center_y: f64,
    zoom: f64,
    confidence: f64,
}

/// Focus bounds (source px) → desired crop center + zoom. Pads the subject box a
/// little, fits the target aspect around it, and clamps zoom to [min,max]. Ported
/// from the reference's `compute_observation_from_bounds`.
fn observation_from_bounds(
    bounds: [f64; 4],
    confidence: f64,
    base_w: u32,
    base_h: u32,
    frame_w: u32,
    frame_h: u32,
    min_zoom: f64,
    max_zoom: f64,
) -> Observation {
    let fw = frame_w as f64;
    let fh = frame_h as f64;
    let left = clamp(bounds[0], 0.0, fw - 1.0);
    let top = clamp(bounds[1], 0.0, fh - 1.0);
    let right = clamp(bounds[2], left + 1.0, fw);
    let bottom = clamp(bounds[3], top + 1.0, fh);
    let region_w = (right - left).max(1.0);
    let region_h = (bottom - top).max(1.0);
    let pad_x = (region_w * 0.12).max(16.0);
    let pad_y = (region_h * 0.16).max(20.0);
    let fit_w = (region_w + pad_x * 2.0).min(fw);
    let fit_h = (region_h + pad_y * 2.0).min(fh);
    let zoom = clamp(
        (base_w as f64 / fit_w.max(1.0)).min(base_h as f64 / fit_h.max(1.0)),
        min_zoom,
        max_zoom,
    );
    let (cw, ch) = crop_size(base_w, base_h, zoom, frame_w, frame_h);
    Observation {
        center_x: clamp((left + right) / 2.0, cw / 2.0, fw - cw / 2.0),
        center_y: clamp((top + bottom) / 2.0, ch / 2.0, fh - ch / 2.0),
        zoom,
        confidence,
    }
}

/// Critically-damped velocity step: pull `current` toward `target`, accumulating
/// velocity (damped), capped at `max_velocity`. Returns (next_value, next_velocity).
/// Ported from the reference's `advance_value_with_velocity` — the heart of the
/// "no jitter, no jarring jump, hold on static" feel.
fn advance(
    current: f64,
    target: f64,
    velocity: f64,
    response: f64,
    damping: f64,
    max_velocity: f64,
) -> (f64, f64) {
    let mut v = velocity * damping + (target - current) * response;
    v = clamp(v, -max_velocity, max_velocity);
    (current + v, v)
}

/// Mutable spring state for the crop camera (center + zoom + their velocities/targets).
#[derive(Debug, Clone, Copy)]
struct CameraState {
    cx: f64,
    cy: f64,
    zoom: f64,
    tcx: f64,
    tcy: f64,
    tzoom: f64,
    vx: f64,
    vy: f64,
    vzoom: f64,
}

impl CameraState {
    fn centered(frame_w: u32, frame_h: u32, min_zoom: f64) -> Self {
        let (cx, cy) = (frame_w as f64 / 2.0, frame_h as f64 / 2.0);
        CameraState {
            cx,
            cy,
            zoom: min_zoom,
            tcx: cx,
            tcy: cy,
            tzoom: min_zoom,
            vx: 0.0,
            vy: 0.0,
            vzoom: 0.0,
        }
    }
}

/// Advance the spring one frame toward `obs`, returning the crop (w,h) for the
/// frame; `state` is mutated with the new smoothed center. Ported faithfully from
/// the reference's `apply_camera_motion` (confidence-weighted alpha + per-axis
/// max-step). The center lives in `state.cx/cy` after this call.
fn step(
    state: &mut CameraState,
    obs: Observation,
    p: &ReframeParams,
    base_w: u32,
    base_h: u32,
    frame_w: u32,
    frame_h: u32,
) -> (f64, f64) {
    let fw = frame_w as f64;
    let fh = frame_h as f64;
    let w = clamp(obs.confidence, 0.15, 1.0);
    let alpha = clamp(p.target_alpha * (0.55 + w * 0.65), 0.04, 0.4);

    state.tzoom = lerp(state.tzoom, obs.zoom, alpha);
    let (z, vz) = advance(
        state.zoom,
        state.tzoom,
        state.vzoom,
        p.zoom_response,
        p.zoom_damping,
        p.zoom_alpha,
    );
    state.zoom = z;
    state.vzoom = vz;

    let (cw, ch) = crop_size(base_w, base_h, state.zoom, frame_w, frame_h);

    state.tcx = clamp(
        lerp(state.tcx, obs.center_x, alpha),
        cw / 2.0,
        fw - cw / 2.0,
    );
    state.tcy = clamp(
        lerp(state.tcy, obs.center_y, alpha),
        ch / 2.0,
        fh - ch / 2.0,
    );

    let (nx, vx) = advance(
        state.cx,
        state.tcx,
        state.vx,
        p.motion_response * (0.65 + w * 0.35),
        clamp(p.motion_damping + 0.08, 0.0, 0.96),
        (p.max_step_x * (0.35 + w * 0.65)).max(1.0),
    );
    let (ny, vy) = advance(
        state.cy,
        state.tcy,
        state.vy,
        p.motion_response * (0.7 + w * 0.3),
        clamp(p.motion_damping + 0.06, 0.0, 0.96),
        (p.max_step_y * (0.35 + w * 0.65)).max(1.0),
    );
    state.vx = vx;
    state.vy = vy;
    state.cx = clamp(nx, cw / 2.0, fw - cw / 2.0);
    state.cy = clamp(ny, ch / 2.0, fh - ch / 2.0);
    (cw, ch)
}

/// Walk the spring over the whole subject track → one [`CropRect`] per frame.
///
/// The deterministic heart of the reframe path. For each frame it derives the desired observation
/// from the subject's focus bounds (denormalized to source px), advances the spring,
/// and emits an integer even-sized, on-frame crop rect. Resets the camera at each
/// scene start (a hard cut shouldn't drag the crop across). Frames with no subject
/// hold the last target with decaying confidence (settle, don't snap); a track that
/// never sees a subject stays centred at `min_zoom`.
///
/// CONSTANT CROP SIZE (critical for the ffmpeg pass): every returned rect has the
/// SAME `w`/`h`. The render drives the moving crop with ffmpeg `sendcmd`, and
/// changing the crop's output W/H per frame STALLS the downstream `scale` filter
/// (it negotiates a fixed input size once — a per-frame size change deadlocks the
/// graph at frame 0; verified. So we pick ONE representative zoom for
/// the whole render — the MEDIAN of the per-frame target zooms (robust to a few
/// outlier frames) — and only PAN (vary x/y). The spring still smooths the centre
/// frame-to-frame, so the follow stays buttery; dynamic per-frame zoom is a v2
/// refinement (needs a different mechanic, e.g. per-scene segments or zoompan).
///
/// `frames` is one [`FrameObs`] per output frame, in order. `scene_starts` is the
/// set of 0-based frame indices that begin a scene (frame 0 is implicitly a start).
pub fn crop_path(
    frames: &[FrameObs],
    frame_w: u32,
    frame_h: u32,
    aspect_w: u32,
    aspect_h: u32,
    params: &ReframeParams,
    scene_starts: &[u32],
) -> Vec<CropRect> {
    let (base_w, base_h) = base_crop(frame_w, frame_h, aspect_w, aspect_h);
    if frames.is_empty() || base_w < 2 || base_h < 2 || (frame_w & !1) < 2 || (frame_h & !1) < 2 {
        return Vec::new();
    }
    let scene_set: std::collections::HashSet<u32> = scene_starts.iter().copied().collect();
    let mut state = CameraState::centered(frame_w, frame_h, params.min_zoom);
    let mut last_obs: Option<Observation> = None;

    // PASS 1 — run the spring to get the smoothed CENTRE per frame + the desired
    // zoom per frame. We keep the centres; the zooms only vote on the constant size.
    let mut centres: Vec<(f64, f64)> = Vec::new();
    let mut zooms: Vec<f64> = Vec::new();
    for (i, fo) in frames.iter().enumerate() {
        if i > 0
            && u32::try_from(i)
                .ok()
                .is_some_and(|idx| scene_set.contains(&idx))
        {
            state = CameraState::centered(frame_w, frame_h, params.min_zoom);
            last_obs = None;
        }
        let obs = if let Some(b) = fo.focus {
            let bounds = [
                b[0] * frame_w as f64,
                b[1] * frame_h as f64,
                b[2] * frame_w as f64,
                b[3] * frame_h as f64,
            ];
            let o = observation_from_bounds(
                bounds,
                clamp(fo.conf, 0.2, 1.0),
                base_w,
                base_h,
                frame_w,
                frame_h,
                params.min_zoom,
                params.max_zoom,
            );
            last_obs = Some(o);
            zooms.push(o.zoom); // only real-subject frames vote on the size
            o
        } else if let Some(mut o) = last_obs {
            // No subject this frame: keep the last target, decay confidence so the
            // crop settles instead of snapping.
            o.confidence = (o.confidence * 0.9).max(0.15);
            last_obs = Some(o);
            o
        } else {
            Observation {
                center_x: frame_w as f64 / 2.0,
                center_y: frame_h as f64 / 2.0,
                zoom: params.min_zoom,
                confidence: 0.15,
            }
        };
        step(&mut state, obs, params, base_w, base_h, frame_w, frame_h);
        centres.push((state.cx, state.cy));
    }

    // The ONE crop size for the whole render = median target zoom (min_zoom if no
    // subject was ever seen). Even dimensions (yuv420), clamped on-frame.
    let const_zoom = median(&mut zooms).unwrap_or(params.min_zoom);
    let (cwf, chf) = crop_size(base_w, base_h, const_zoom, frame_w, frame_h);
    let cw = ((cwf.round() as u32) & !1).clamp(2, frame_w & !1);
    let ch = ((chf.round() as u32) & !1).clamp(2, frame_h & !1);

    // PASS 2 — emit constant-size rects, panning the fixed window to each smoothed
    // centre, clamped fully on-frame.
    centres
        .into_iter()
        .map(|(cx, cy)| {
            let x = clamp(cx - cw as f64 / 2.0, 0.0, (frame_w - cw) as f64).round() as u32;
            let y = clamp(cy - ch as f64 / 2.0, 0.0, (frame_h - ch) as f64).round() as u32;
            CropRect { w: cw, h: ch, x, y }
        })
        .collect()
}

/// Median of a zoom sample (sorts in place). `None` for an empty sample.
fn median(vals: &mut [f64]) -> Option<f64> {
    if vals.is_empty() {
        return None;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = vals.len();
    Some(if n % 2 == 1 {
        vals[n / 2]
    } else {
        (vals[n / 2 - 1] + vals[n / 2]) / 2.0
    })
}

/// Build the ffmpeg `sendcmd` script that PANS a constant-size crop from a
/// [`crop_path`].
///
/// One line per frame: at the frame's presentation time, set the crop `x`/`y` only.
/// The crop SIZE is fixed by the render's `crop=<cw>:<ch>:…` (every rect from
/// `crop_path` carries the same w/h) — commanding only x/y is what keeps the
/// downstream `scale` happy (a per-frame W/H change stalls the graph; see
/// `crop_path`). The render runs:
/// `sendcmd=f=<file>,crop=<cw>:<ch>:<x0>:<y0>,scale=<out_w>:<out_h>,setsar=1`.
///
/// Times use the frame index / `fps`; `fps` must be > 0.
pub fn sendcmd_script(rects: &[CropRect], fps: f64) -> String {
    let fps = if fps > 0.0 { fps } else { 30.0 };
    let mut s = String::new();
    for (i, r) in rects.iter().enumerate() {
        let t = i as f64 / fps;
        // e.g. "0.0417 crop x 244, crop y 7;"
        s.push_str(&format!("{:.4} crop x {}, crop y {};\n", t, r.x, r.y));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs_at(cx: f64, cy: f64, half: f64) -> FrameObs {
        // a square focus box of side 2*half centred at (cx,cy), normalized
        FrameObs {
            focus: Some([cx - half, cy - half, cx + half, cy + half]),
            conf: 0.9,
            scene: 0,
        }
    }

    #[test]
    fn base_crop_169_to_916_is_portrait_slice() {
        // 1920x1080 → 9:16: crop height = full 1080, width = 1080*9/16 = 607.5 → 608
        let (w, h) = base_crop(1920, 1080, 9, 16);
        assert_eq!(h, 1080);
        assert_eq!(w, 608);
    }

    #[test]
    fn base_crop_square_from_landscape() {
        let (w, h) = base_crop(1920, 1080, 1, 1);
        assert_eq!((w, h), (1080, 1080)); // tallest square that fits
    }

    #[test]
    fn base_crop_zero_frame_or_aspect_is_empty() {
        assert_eq!(base_crop(0, 1080, 9, 16), (0, 0));
        assert_eq!(base_crop(1920, 0, 9, 16), (0, 0));
        assert_eq!(base_crop(1920, 1080, 0, 16), (0, 0));
        assert_eq!(base_crop(1920, 1080, 9, 0), (0, 0));
    }

    #[test]
    fn crop_path_returns_empty_for_frames_too_small_to_crop() {
        let frames = vec![obs_at(0.5, 0.5, 0.1)];
        assert!(
            crop_path(&frames, 1, 1, 9, 16, &ReframeParams::default(), &[0]).is_empty(),
            "1px frames cannot produce an even ffmpeg crop rect"
        );
        assert!(
            crop_path(&frames, 0, 1080, 9, 16, &ReframeParams::default(), &[0]).is_empty(),
            "zero-width frames cannot produce a crop rect"
        );
    }

    #[test]
    fn every_rect_is_even_and_on_frame() {
        let (fw, fh) = (1920u32, 1080u32);
        // a subject drifting left→right across the middle
        let frames: Vec<FrameObs> = (0..120)
            .map(|i| obs_at(0.2 + 0.6 * (i as f64 / 119.0), 0.5, 0.12))
            .collect();
        let rects = crop_path(&frames, fw, fh, 9, 16, &ReframeParams::default(), &[0]);
        assert_eq!(rects.len(), 120);
        for r in &rects {
            assert_eq!(r.w % 2, 0, "w even");
            assert_eq!(r.h % 2, 0, "h even");
            assert!(r.x + r.w <= fw, "x+w on frame: {}+{} <= {}", r.x, r.w, fw);
            assert!(r.y + r.h <= fh, "y+h on frame");
        }
    }

    #[test]
    fn static_subject_holds_crop_steady() {
        // A perfectly still subject → after settling, the crop centre must stop moving.
        let frames: Vec<FrameObs> = (0..90).map(|_| obs_at(0.5, 0.5, 0.12)).collect();
        let rects = crop_path(&frames, 1920, 1080, 9, 16, &ReframeParams::default(), &[0]);
        let a = rects[80];
        let b = rects[89];
        assert!(
            (a.x as i64 - b.x as i64).abs() <= 1,
            "x steady once settled: {} vs {}",
            a.x,
            b.x
        );
        assert!(
            (a.y as i64 - b.y as i64).abs() <= 1,
            "y steady once settled"
        );
    }

    #[test]
    fn moving_subject_crop_follows_within_max_step() {
        // The crop centre must never jump more than ~max_step per frame (the no-jarring
        // guarantee). Subject jumps instantly from far left to far right at frame 30.
        let mut frames: Vec<FrameObs> = (0..30).map(|_| obs_at(0.2, 0.5, 0.1)).collect();
        frames.extend((0..60).map(|_| obs_at(0.8, 0.5, 0.1)));
        let p = ReframeParams::default();
        let rects = crop_path(&frames, 1920, 1080, 9, 16, &p, &[0]);
        for w in rects.windows(2) {
            let dx = (w[1].x as i64 - w[0].x as i64).abs();
            // max_step_x scaled by confidence weight upper bound (0.35+0.65) = max_step_x
            assert!(
                dx as f64 <= p.max_step_x + 1.0,
                "per-frame x step {} <= {}",
                dx,
                p.max_step_x + 1.0
            );
        }
        // ...and it DID move toward the new position (followed, not stuck).
        assert!(rects[89].x > rects[0].x, "crop followed the subject right");
    }

    #[test]
    fn no_subject_track_stays_centered() {
        let frames: Vec<FrameObs> = (0..30)
            .map(|_| FrameObs {
                focus: None,
                conf: 0.0,
                scene: 0,
            })
            .collect();
        let rects = crop_path(&frames, 1920, 1080, 9, 16, &ReframeParams::default(), &[0]);
        let r = rects[29];
        let cx = r.x + r.w / 2;
        assert!(
            (cx as i64 - 960).abs() <= 4,
            "centred horizontally when no subject: {}",
            cx
        );
    }

    #[test]
    fn scene_cut_resets_camera() {
        // Subject on the left for scene 0, then a hard cut to a subject on the right.
        // Without the reset the crop would drag across; with it, scene 1 starts fresh
        // and reaches the right faster than a no-reset walk would.
        let mut frames: Vec<FrameObs> = (0..30).map(|_| obs_at(0.2, 0.5, 0.1)).collect();
        frames.extend((30..60).map(|_| FrameObs {
            scene: 1,
            ..obs_at(0.8, 0.5, 0.1)
        }));
        let rects = crop_path(
            &frames,
            1920,
            1080,
            9,
            16,
            &ReframeParams::default(),
            &[0, 30],
        );
        // The frame right at the cut is re-centred (camera reset to frame centre).
        let at_cut = rects[30];
        let cx = at_cut.x + at_cut.w / 2;
        assert!(
            (cx as i64 - 960).abs() < 300,
            "camera reset toward centre at the cut: {}",
            cx
        );
    }

    #[test]
    fn sendcmd_script_pans_xy_only() {
        let rects = vec![
            CropRect {
                w: 142,
                h: 252,
                x: 248,
                y: 6,
            },
            CropRect {
                w: 142,
                h: 252,
                x: 244,
                y: 6,
            },
        ];
        let s = sendcmd_script(&rects, 24.0);
        let lines: Vec<&str> = s.trim().lines().collect();
        assert_eq!(lines.len(), 2, "one line per frame");
        // frame 0 @ t=0, frame 1 @ t=1/24=0.0417 — x/y ONLY (no per-frame size).
        assert_eq!(lines[0], "0.0000 crop x 248, crop y 6;");
        assert_eq!(lines[1], "0.0417 crop x 244, crop y 6;");
        // Per-frame crop W/H would stall the downstream scale — must NOT be emitted.
        assert!(
            !s.contains("crop w") && !s.contains("crop h"),
            "no per-frame w/h"
        );
        // zero fps must not panic / divide-by-zero (falls back to 30)
        assert!(!sendcmd_script(&rects, 0.0).is_empty());
    }

    #[test]
    fn crop_size_is_constant_across_the_path() {
        // A subject that "grows" would tempt per-frame zoom; the path must still emit
        // ONE constant crop size (a per-frame size change stalls the ffmpeg scale).
        let frames: Vec<FrameObs> = (0..90)
            .map(|i| obs_at(0.5, 0.5, 0.06 + 0.10 * (i as f64 / 89.0)))
            .collect();
        let rects = crop_path(&frames, 1920, 1080, 9, 16, &ReframeParams::default(), &[0]);
        let (w0, h0) = (rects[0].w, rects[0].h);
        for r in &rects {
            assert_eq!(
                (r.w, r.h),
                (w0, h0),
                "crop size must be constant for the ffmpeg pass"
            );
        }
    }

    #[test]
    fn sendcmd_times_are_monotonic() {
        let rects: Vec<CropRect> = (0..50)
            .map(|_| CropRect {
                w: 100,
                h: 200,
                x: 10,
                y: 20,
            })
            .collect();
        let s = sendcmd_script(&rects, 30.0);
        let times: Vec<f64> = s
            .trim()
            .lines()
            .map(|l| l.split_whitespace().next().unwrap().parse().unwrap())
            .collect();
        for w in times.windows(2) {
            assert!(
                w[1] > w[0],
                "sendcmd times strictly increasing: {} !> {}",
                w[1],
                w[0]
            );
        }
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let frames: Vec<FrameObs> = (0..50)
            .map(|i| obs_at(0.3 + 0.004 * i as f64, 0.5, 0.1))
            .collect();
        let p = ReframeParams::for_preset("talking_head");
        let a = crop_path(&frames, 1280, 720, 9, 16, &p, &[0]);
        let b = crop_path(&frames, 1280, 720, 9, 16, &p, &[0]);
        assert_eq!(a, b, "crop_path must be deterministic");
    }
}
