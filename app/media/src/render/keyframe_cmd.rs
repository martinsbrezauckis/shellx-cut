//! Command-file (`sendcmd`/`asendcmd`) keyframe evaluator and FFmpeg filter emitters.
//!
//! Replaces large inline `if(lt(T,...))` FFmpeg expressions with per-frame
//! sampled command files under the session's private work folder. Keeps
//! FFmpeg expression nesting depth O(1) on FFmpeg 6.1+ (Linux/macOS/Windows)
//! without requiring optional `libzmq`.

use cut_core::{CutError, Keyframe, KfInterp, KfParam};
use std::fmt::Write;
use tempfile::TempPath;

use crate::ffmpeg::escape_filter_path;
use crate::internal_files::private_list_file;

/// Maximum combined size of generated FFmpeg `sendcmd`/`asendcmd` scripts
/// allowed for one render plan before refusing prior to allocation (256 MiB).
pub const MAX_RENDER_CMD_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum number of timed audio updates allowed before collecting timestamps.
pub const MAX_AUDIO_CMD_TIMESTAMPS: u64 = 2_000_000;
const MAX_VIDEO_CMD_FRAMES: usize = 2_000_000;
const AUDIO_CMD_STEP_MS: u64 = 20;

pub(super) fn effective_fps(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    }
}

pub(super) fn decimal_digits(mut n: u64) -> u64 {
    if n == 0 {
        return 1;
    }
    let mut digits = 0u64;
    while n > 0 {
        n /= 10;
        digits += 1;
    }
    digits
}

pub(super) fn video_cmd_frame_count(fps: f64, dur_ms: u64) -> u64 {
    let fps = effective_fps(fps);
    let raw = ((dur_ms as f64 / 1000.0) * fps).ceil();
    if !raw.is_finite() {
        return u64::MAX;
    }
    (raw.max(1.0) as u64).saturating_add(1)
}

pub(super) fn line_overhead_bytes(dur_ms: u64) -> u64 {
    // "{cmd_t:.6} {body};\n": integer digits (up to dur_ms / 1000) + ".000000 " + ";\n".
    decimal_digits(dur_ms / 1000).saturating_add(10)
}

pub(super) fn estimate_video_cmd_bytes(fps: f64, dur_ms: u64, body_bytes: u64) -> u64 {
    if body_bytes == 0 {
        return 0;
    }
    video_cmd_frame_count(fps, dur_ms)
        .saturating_mul(body_bytes.saturating_add(line_overhead_bytes(dur_ms)))
}

pub(super) fn base_opacity_body_bytes(uniq_len: u64) -> u64 {
    // "colorchannelmixer@bop{uniq} rr {v}, colorchannelmixer@bop{uniq} gg {v}, colorchannelmixer@bop{uniq} bb {v}"
    94u64.saturating_add(3u64.saturating_mul(uniq_len))
}

pub(super) fn overlay_opacity_body_bytes(uniq_len: u64, keep_hdr: bool) -> u64 {
    if keep_hdr {
        // "lut@op{uniq} a val*{v}"
        19u64.saturating_add(uniq_len)
    } else {
        // "colorchannelmixer@op{uniq} aa {v}"
        30u64.saturating_add(uniq_len)
    }
}

pub(super) fn scale_crop_body_bytes(uniq_len: u64) -> u64 {
    // "crop@zc{uniq} w {cw}, crop@zc{uniq} h {ch}, crop@zc{uniq} x {cx}, crop@zc{uniq} y {cy}"
    62u64.saturating_add(4u64.saturating_mul(uniq_len))
}

pub(super) fn shaped_scale_body_bytes(uniq_len: u64) -> u64 {
    // "scale@sc{uniq} w {d}, scale@sc{uniq} h {h}"
    34u64.saturating_add(2u64.saturating_mul(uniq_len))
}

pub(super) fn overlay_pos_body_bytes(uniq_len: u64, x_long: bool, y_long: bool) -> u64 {
    let axes = u64::from(x_long) + u64::from(y_long);
    if axes == 0 {
        return 0;
    }
    axes.saturating_mul(21u64.saturating_add(uniq_len))
        .saturating_add(if axes == 2 { 2 } else { 0 })
}

/// Estimate upper bound on `(bytes, timestamps)` produced by [`build_audio_cmd_script`].
pub(super) fn estimate_audio_cmd_usage(
    points_len: usize,
    dur_ms: u64,
    uniq_len: u64,
) -> (u64, u64) {
    let steps = (dur_ms / AUDIO_CMD_STEP_MS).saturating_add(1);
    let tail = u64::from(!dur_ms.is_multiple_of(AUDIO_CMD_STEP_MS));
    let timestamps = steps.saturating_add(tail).saturating_add(points_len as u64);
    let per_line = line_overhead_bytes(dur_ms).saturating_add(24u64.saturating_add(uniq_len));
    (timestamps.saturating_mul(per_line), timestamps)
}

pub(super) fn command_budget_error(estimated_bytes: u64, clip: Option<&str>) -> CutError {
    let mb = estimated_bytes.div_ceil(1024 * 1024).max(1);
    let limit_mb = MAX_RENDER_CMD_BYTES / (1024 * 1024);
    let mut err = CutError::new(
        cut_core::error_codes::INVALID_ARGS,
        format!(
            "This export would need {mb} MB of movement data; split the clip or reduce the moving regions"
        ),
        format!("generated frame command files would reach {mb} MB (limit {limit_mb} MB)"),
    )
    .with_suggested_action(
        "split the clip into shorter segments or reduce the moving regions or dense keyframe curves",
    );
    if let Some(id) = clip.filter(|s| !s.is_empty()) {
        err = err.with_clip(id);
    }
    err
}

pub(super) fn audio_timestamps_budget_error(
    estimated_bytes: u64,
    timestamps: u64,
    clip: Option<&str>,
) -> CutError {
    if estimated_bytes > MAX_RENDER_CMD_BYTES {
        return command_budget_error(estimated_bytes, clip);
    }
    let mb = estimated_bytes.div_ceil(1024 * 1024).max(1);
    let mut err = CutError::new(
        cut_core::error_codes::INVALID_ARGS,
        format!(
            "This export would need {mb} MB of movement data; split the clip or reduce the moving regions"
        ),
        format!(
            "dense audio volume curve would emit up to {timestamps} timed updates (~{mb} MB; limit {MAX_AUDIO_CMD_TIMESTAMPS})"
        ),
    )
    .with_suggested_action("split the clip into shorter segments or reduce dense volume keyframes");
    if let Some(id) = clip.filter(|s| !s.is_empty()) {
        err = err.with_clip(id);
    }
    err
}

/// Format a float compactly for FFmpeg `sendcmd`/`asendcmd` arguments.
pub(super) fn fmt_cmd_float(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let clean = if v == 0.0 { 0.0 } else { v };
    let mut s = format!("{clean:.6}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s
    }
}

/// Evaluate a keyframe track at `t` exactly as the released expression form
/// (`kf_expr`) did: the first value before the first point, the last value
/// after the last; inside, the first pair whose end lies after `t`. Hold and a
/// zero-length pair keep the pair's first value; linear does not clamp its
/// fraction; easings clamp it to `[0, 1]`.
pub(super) fn eval_kf(points: &[(f64, f64)], interp: KfInterp, t: f64) -> f64 {
    match points {
        [] => 0.0,
        [only] => finite_or_zero(only.1),
        _ if t < points[0].0 => finite_or_zero(points[0].1),
        _ => points
            .windows(2)
            .find(|pair| t < pair[1].0)
            .map_or(finite_or_zero(points[points.len() - 1].1), |pair| {
                pair_value(pair[0], pair[1], interp, t)
            }),
    }
}

fn pair_value((t0, v0): (f64, f64), (t1, v1): (f64, f64), interp: KfInterp, t: f64) -> f64 {
    if matches!(interp, KfInterp::Hold) || (t1 - t0).abs() < 1e-9 {
        return finite_or_zero(v0);
    }
    let frac = (t - t0) / (t1 - t0);
    let eased = if matches!(interp, KfInterp::Linear) {
        frac
    } else {
        interp.sample(frac.clamp(0.0, 1.0))
    };
    finite_or_zero(v0 + (v1 - v0) * eased)
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// [`eval_kf`] for rising sample times: a time-sorted track is walked once.
pub(super) struct KfCursor<'a> {
    points: &'a [(f64, f64)],
    interp: KfInterp,
    sorted: bool,
    idx: usize,
}

impl<'a> KfCursor<'a> {
    pub(super) fn new(points: &'a [(f64, f64)], interp: KfInterp) -> Self {
        let sorted = points.windows(2).all(|w| w[1].0 >= w[0].0);
        Self {
            points,
            interp,
            sorted,
            idx: 0,
        }
    }

    pub(super) fn eval(&mut self, t: f64) -> f64 {
        if !self.sorted || self.points.len() < 2 || t < self.points[0].0 {
            return eval_kf(self.points, self.interp, t);
        }
        while self.idx + 1 < self.points.len() && t >= self.points[self.idx + 1].0 {
            self.idx += 1;
        }
        if self.idx + 1 >= self.points.len() {
            return finite_or_zero(self.points[self.points.len() - 1].1);
        }
        pair_value(
            self.points[self.idx],
            self.points[self.idx + 1],
            self.interp,
            t,
        )
    }
}

/// Build an FFmpeg `sendcmd` script for video frames over `[0, dur_ms]`.
///
/// Frame 0 executes at `0.000000`; frame `k >= 1` executes at midpoint
/// `(k - 0.5) / fps` so PTS jitter/rounding never shifts a transition by a frame.
/// Consecutive frames whose formatted command body matches are deduplicated.
pub(super) fn build_video_cmd_script<F>(
    fps: f64,
    dur_ms: u64,
    estimated_body_bytes: u64,
    mut emit_frame: F,
) -> Result<String, CutError>
where
    F: FnMut(f64, &mut String),
{
    let est = estimate_video_cmd_bytes(fps, dur_ms, estimated_body_bytes);
    if est > MAX_RENDER_CMD_BYTES {
        return Err(command_budget_error(est, None));
    }
    let fps = effective_fps(fps);
    let total_frames = ((dur_ms as f64 / 1000.0) * fps)
        .ceil()
        .clamp(1.0, MAX_VIDEO_CMD_FRAMES as f64) as usize;

    let mut out = String::new();
    let mut last_body: Option<String> = None;
    let mut body = String::new();
    let line_over = line_overhead_bytes(dur_ms);
    for k in 0..=total_frames {
        body.clear();
        let t_sec = k as f64 / fps;
        emit_frame(t_sec, &mut body);
        if body.is_empty() || last_body.as_deref() == Some(body.as_str()) {
            continue;
        }
        let projected = (out.len() as u64)
            .saturating_add(body.len() as u64)
            .saturating_add(line_over);
        if projected > MAX_RENDER_CMD_BYTES {
            return Err(command_budget_error(projected, None));
        }
        let cmd_t = if k == 0 { 0.0 } else { (k as f64 - 0.5) / fps };
        let _ = writeln!(out, "{cmd_t:.6} {body};");
        last_body = Some(body.clone());
    }
    Ok(out)
}

/// Build an FFmpeg `asendcmd` script sampled across audio frames and every exact
/// keyframe boundary (so `Hold` jumps land right before their frame PTS).
pub(super) fn build_audio_cmd_script(
    points: &[(f64, f64)],
    interp: KfInterp,
    dur_ms: u64,
    uniq: &str,
) -> Result<String, CutError> {
    let (est_bytes, est_timestamps) =
        estimate_audio_cmd_usage(points.len(), dur_ms, uniq.len() as u64);
    if est_bytes > MAX_RENDER_CMD_BYTES {
        return Err(command_budget_error(est_bytes, None));
    }
    if est_timestamps > MAX_AUDIO_CMD_TIMESTAMPS {
        return Err(audio_timestamps_budget_error(
            est_bytes,
            est_timestamps,
            None,
        ));
    }
    let mut times_ms: Vec<u64> = (0..=dur_ms).step_by(AUDIO_CMD_STEP_MS as usize).collect();
    if times_ms.last().copied() != Some(dur_ms) {
        times_ms.push(dur_ms);
    }
    for &(t_sec, _) in points {
        if t_sec.is_finite() && t_sec >= 0.0 {
            let ms = (t_sec * 1000.0).round() as u64;
            if ms <= dur_ms {
                times_ms.push(ms);
            }
        }
    }
    times_ms.sort_unstable();
    times_ms.dedup();

    let mut out = String::new();
    let mut cursor = KfCursor::new(points, interp);
    let mut last_body: Option<String> = None;
    let mut last_cmd_t = -1.0_f64;
    let line_over = line_overhead_bytes(dur_ms);

    for (idx, ms) in times_ms.into_iter().enumerate() {
        let t_sec = ms as f64 / 1000.0;
        let val = cursor.eval(t_sec).clamp(0.0, 16.0);
        let body = format!("volume@vol{uniq} volume {}", fmt_cmd_float(val));
        if last_body.as_deref() == Some(body.as_str()) {
            continue;
        }
        let projected = (out.len() as u64)
            .saturating_add(body.len() as u64)
            .saturating_add(line_over);
        if projected > MAX_RENDER_CMD_BYTES {
            return Err(command_budget_error(projected, None));
        }
        let mut cmd_t = if idx == 0 && ms == 0 {
            0.0
        } else {
            (t_sec - 0.0005).max(0.000001)
        };
        if cmd_t <= last_cmd_t {
            cmd_t = last_cmd_t + 0.000001;
        }
        let _ = writeln!(out, "{cmd_t:.6} {body};");
        last_cmd_t = cmd_t;
        last_body = Some(body);
    }
    Ok(out)
}

/// Persist a command script into the process's private work folder and return
/// its FFmpeg filter-escaped path, or return a placeholder when dry-running.
pub(super) fn attach_cmd_file(
    script: &str,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<String, CutError> {
    if let Some(files) = cmd_files {
        let temp = private_list_file(".cut-cmd-", ".txt", script)?;
        let escaped = escape_filter_path(temp.as_ref());
        files.push(temp);
        Ok(escaped)
    } else {
        Ok("<dry-run>".into())
    }
}

/// One parameter's effective control points in seconds (a clipped easing's
/// lead-in included, exactly as the expression form read them), moved
/// `offset_sec` earlier for a render window that starts inside the clip.
pub(super) fn extract_kf_points(
    keyframes: &[Keyframe],
    param: KfParam,
    offset_sec: f64,
) -> Option<(Vec<(f64, f64)>, KfInterp)> {
    let (mut pts, interp) = super::kf_points(keyframes, param)?;
    if pts.is_empty() {
        return None;
    }
    for (t, _) in &mut pts {
        *t -= offset_sec;
    }
    Some((pts, interp))
}

/// Base-track opacity filter supporting multi-point keyframe curves via `sendcmd`.
/// Keep HDR (`keep_hdr`) scales the same planar RGB in 16 bits and returns to
/// 10-bit YUV; SDR keeps the 8-bit graph.
#[allow(clippy::too_many_arguments)]
pub(super) fn base_opacity_filter_cmd(
    t: &cut_core::ClipTransform,
    keyframes: &[Keyframe],
    fps: f64,
    dur_ms: u64,
    offset_sec: f64,
    uniq: &str,
    keep_hdr: bool,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<String, CutError> {
    let (rgb, yuv) = if keep_hdr {
        ("gbrp16le", "yuv420p10le")
    } else {
        ("gbrp", "yuv420p")
    };
    let Some((pts, interp)) = extract_kf_points(keyframes, KfParam::Opacity, offset_sec) else {
        if t.opacity == 1.0 {
            return Ok(String::new());
        }
        let opacity = super::fnum(t.opacity.clamp(0.0, 1.0));
        return Ok(format!(
            ",format={rgb},geq=r='r(X,Y)*{opacity}':g='g(X,Y)*{opacity}':b='b(X,Y)*{opacity}',format={yuv}"
        ));
    };

    let v0 = fmt_cmd_float(eval_kf(&pts, interp, 0.0).clamp(0.0, 1.0));
    if pts.len() == 1 {
        return Ok(format!(
            ",format={rgb},colorchannelmixer=rr={v0}:gg={v0}:bb={v0},format={yuv}"
        ));
    }

    let mut cursor = KfCursor::new(&pts, interp);
    let script = build_video_cmd_script(
        fps,
        dur_ms,
        base_opacity_body_bytes(uniq.len() as u64),
        |t_sec, body| {
            let v = fmt_cmd_float(cursor.eval(t_sec).clamp(0.0, 1.0));
            let _ = write!(
                body,
                "colorchannelmixer@bop{uniq} rr {v}, colorchannelmixer@bop{uniq} gg {v}, colorchannelmixer@bop{uniq} bb {v}"
            );
        },
    )?;
    let escaped = attach_cmd_file(&script, cmd_files)?;
    Ok(format!(
        ",format={rgb},sendcmd=f={escaped},colorchannelmixer@bop{uniq}=rr={v0}:gg={v0}:bb={v0},format={yuv}"
    ))
}

/// Overlay opacity filter supporting multi-point keyframe curves via `sendcmd`.
/// SDR keeps the released `colorchannelmixer` graph. Keep HDR scales the alpha
/// plane of the 10-bit YUVA frame in place with `lut` (no RGB round trip, so the
/// BT.2020 picture is untouched), driven by the same command file.
#[allow(clippy::too_many_arguments)]
pub(super) fn opacity_kf_filter_cmd(
    pts: &[(f64, f64)],
    interp: KfInterp,
    fps: f64,
    dur_ms: u64,
    uniq: &str,
    keep_hdr: bool,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<String, CutError> {
    if pts.is_empty() {
        return Ok(String::new());
    }
    let v0 = fmt_cmd_float(eval_kf(pts, interp, 0.0).clamp(0.0, 1.0));
    if pts.len() == 1 {
        return Ok(if keep_hdr {
            format!(",format=yuva420p10le,lut=a=val*{v0}")
        } else {
            format!(",format=gbrap,colorchannelmixer=aa={v0},format=yuva420p")
        });
    }

    let mut cursor = KfCursor::new(pts, interp);
    let script = build_video_cmd_script(
        fps,
        dur_ms,
        overlay_opacity_body_bytes(uniq.len() as u64, keep_hdr),
        |t_sec, body| {
            let v = fmt_cmd_float(cursor.eval(t_sec).clamp(0.0, 1.0));
            if keep_hdr {
                let _ = write!(body, "lut@op{uniq} a val*{v}");
            } else {
                let _ = write!(body, "colorchannelmixer@op{uniq} aa {v}");
            }
        },
    )?;
    let escaped = attach_cmd_file(&script, cmd_files)?;
    Ok(if keep_hdr {
        format!(",format=yuva420p10le,sendcmd=f={escaped},lut@op{uniq}=a=val*{v0}")
    } else {
        format!(
            ",format=gbrap,sendcmd=f={escaped},colorchannelmixer@op{uniq}=aa={v0},format=yuva420p"
        )
    })
}

/// Animated clip zoom filter (`[1, 10]`) via centered `crop@zc` + `scale={w}:{h}`.
pub(super) fn scale_kf_cmd(
    pts: &[(f64, f64)],
    interp: KfInterp,
    w: u32,
    h: u32,
    fps: f64,
    dur_ms: u64,
    uniq: &str,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<Option<String>, CutError> {
    if pts.is_empty() {
        return Ok(None);
    }
    let crop_dims = |z: f64| -> (u32, u32, u32, u32) {
        let z = z.clamp(1.0, 10.0);
        let cw = (w as f64 / z).round().clamp(2.0, w as f64) as u32;
        let ch = (h as f64 / z).round().clamp(2.0, h as f64) as u32;
        let cx = ((w - cw) as f64 / 2.0).round() as u32;
        let cy = ((h - ch) as f64 / 2.0).round() as u32;
        (cw, ch, cx, cy)
    };
    let (cw0, ch0, cx0, cy0) = crop_dims(eval_kf(pts, interp, 0.0));
    let fps_s = super::fps_str(fps);
    if pts.len() == 1 {
        return Ok(Some(format!(
            ",fps={fps_s},setpts=N/({fps_s}*TB),crop=w={cw0}:h={ch0}:x={cx0}:y={cy0}:exact=1,scale={w}:{h},setsar=1"
        )));
    }

    let mut cursor = KfCursor::new(pts, interp);
    let script = build_video_cmd_script(
        fps,
        dur_ms,
        scale_crop_body_bytes(uniq.len() as u64),
        |t_sec, body| {
            let (cw, ch, cx, cy) = crop_dims(cursor.eval(t_sec));
            let _ = write!(
                body,
                "crop@zc{uniq} w {cw}, crop@zc{uniq} h {ch}, crop@zc{uniq} x {cx}, crop@zc{uniq} y {cy}"
            );
        },
    )?;
    let escaped = attach_cmd_file(&script, cmd_files)?;
    Ok(Some(format!(
        ",fps={fps_s},setpts=N/({fps_s}*TB),sendcmd=f={escaped},crop@zc{uniq}=w={cw0}:h={ch0}:x={cx0}:y={cy0}:exact=1,scale={w}:{h},setsar=1"
    )))
}

/// Audio keyframed `volume` filter via `asendcmd`.
pub(super) fn volume_kf_filter_cmd(
    keyframes: &[Keyframe],
    dur_ms: u64,
    offset_sec: f64,
    uniq: &str,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<String, CutError> {
    let Some((pts, interp)) = extract_kf_points(keyframes, KfParam::Volume, offset_sec) else {
        return Ok(String::new());
    };
    let v0 = fmt_cmd_float(eval_kf(&pts, interp, 0.0).clamp(0.0, 16.0));
    if pts.len() == 1 {
        return Ok(format!(",volume={v0}"));
    }

    let script = build_audio_cmd_script(&pts, interp, dur_ms, uniq)?;
    let escaped = attach_cmd_file(&script, cmd_files)?;
    Ok(format!(
        ",asendcmd=f={escaped},volume@vol{uniq}=volume={v0}"
    ))
}

/// Clip volume and Track Gain for an audio chain. Curves the released
/// expression could hold keep its exact filter; a longer curve is read from a
/// command file, followed by the separately carried Track Gain.
pub(super) fn audio_gain_filters_cmd(
    seg: &cut_core::EdlSegment,
    dur_ms: u64,
    offset_sec: f64,
    uniq: &str,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<String, CutError> {
    if !super::keyframe_window::long(seg, KfParam::Volume) {
        return Ok(super::audio_gain_filters(seg));
    }
    let mut filters = volume_kf_filter_cmd(&seg.keyframes, dur_ms, offset_sec, uniq, cmd_files)?;
    if seg.track_gain_db != 0.0 {
        let _ = write!(filters, ",volume={:.2}dB", seg.track_gain_db);
    }
    Ok(filters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_kf_clamps_handles_duplicates_and_matches_sample() {
        assert_eq!(eval_kf(&[], KfInterp::Linear, 1.0), 0.0);
        assert_eq!(eval_kf(&[(2.0, 0.7)], KfInterp::Linear, 0.0), 0.7);
        assert_eq!(eval_kf(&[(2.0, 0.7)], KfInterp::Linear, 5.0), 0.7);

        // At a repeated time the released expression already reads the later pair.
        let dup = vec![(0.0, 0.1), (1.0, 0.2), (1.0, 0.9), (2.0, 0.5)];
        assert!((eval_kf(&dup, KfInterp::Linear, 1.0) - 0.9).abs() < 1e-12);
        assert!((eval_kf(&dup, KfInterp::Linear, 0.5) - 0.15).abs() < 1e-12);
        // Unsorted saved tracks keep the released first-matching-pair reading.
        let unsorted = vec![(0.0, 0.0), (2.0, 1.0), (1.0, 0.0)];
        assert!((eval_kf(&unsorted, KfInterp::Linear, 1.5) - 0.75).abs() < 1e-12);
        assert_eq!(eval_kf(&unsorted, KfInterp::Linear, 3.0), 0.0);

        let pts = vec![(0.0, 0.2), (2.0, 0.8), (4.0, 0.1)];
        for interp in [
            KfInterp::Hold,
            KfInterp::Linear,
            KfInterp::EaseInQuad,
            KfInterp::EaseOutCubic,
            KfInterp::EaseInOutBounce,
            KfInterp::EaseInOutElastic,
            KfInterp::EaseOutBack,
        ] {
            assert!((eval_kf(&pts, interp, -1.0) - 0.2).abs() < 1e-12);
            assert!((eval_kf(&pts, interp, 5.0) - 0.1).abs() < 1e-12);
            let mut cur = KfCursor::new(&pts, interp);
            for step in 0..=40 {
                let t = step as f64 * 0.1;
                let a = eval_kf(&pts, interp, t);
                let b = cur.eval(t);
                assert!((a - b).abs() < 1e-12, "{interp:?} t={t}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn video_cmd_script_uses_midpoint_timestamps_and_deduplicates() {
        let script = build_video_cmd_script(30.0, 200, 15, |t, body| {
            let val = if t < 0.06 { 10 } else { 20 };
            let _ = write!(body, "drawbox@db x {val}");
        })
        .unwrap();
        let lines: Vec<_> = script.lines().collect();
        assert_eq!(lines.len(), 2, "duplicate frames should collapse: {script}");
        assert!(lines[0].starts_with("0.000000 "), "{lines:?}");
        // Frame 2 is at t = 2/30 = 0.066666... >= 0.06 -> cmd timestamp is 1.5/30 = 0.050000.
        assert!(lines[1].starts_with("0.050000 "), "{lines:?}");
    }

    #[test]
    fn dry_run_leaves_no_file_and_returns_placeholder() {
        let out = attach_cmd_file("0.000000 x y;\n", None).unwrap();
        assert_eq!(out, "<dry-run>");
    }

    #[test]
    fn keep_hdr_opacity_curves_stay_ten_bit() {
        let pts: Vec<_> = (0..=80)
            .map(|i| (i as f64 / 80.0, i as f64 / 80.0))
            .collect();
        let hdr =
            opacity_kf_filter_cmd(&pts, KfInterp::Linear, 30.0, 1000, "u", true, None).unwrap();
        assert_eq!(
            hdr,
            ",format=yuva420p10le,sendcmd=f=<dry-run>,lut@opu=a=val*0"
        );
        let sdr =
            opacity_kf_filter_cmd(&pts, KfInterp::Linear, 30.0, 1000, "u", false, None).unwrap();
        assert_eq!(
            sdr,
            ",format=gbrap,sendcmd=f=<dry-run>,colorchannelmixer@opu=aa=0,format=yuva420p"
        );
        let keys = vec![Keyframe {
            lead_in: None,
            param: KfParam::Opacity,
            points: (0..=80)
                .map(|i| cut_core::KfPoint {
                    t_ms: i * 12,
                    value: 0.5,
                })
                .collect(),
            interp: KfInterp::Linear,
        }];
        let t = cut_core::ClipTransform::identity();
        let base = base_opacity_filter_cmd(&t, &keys, 30.0, 1000, 0.0, "b", true, None).unwrap();
        assert_eq!(
            base,
            ",format=gbrp16le,sendcmd=f=<dry-run>,colorchannelmixer@bopb=rr=0.5:gg=0.5:bb=0.5,format=yuv420p10le"
        );
    }

    #[test]
    fn audio_cmd_script_refuses_excessive_timestamps_before_vec_allocation() {
        let keys = vec![Keyframe {
            lead_in: None,
            param: KfParam::Volume,
            points: (0..=65)
                .map(|i| cut_core::KfPoint {
                    t_ms: i * 1_000,
                    value: 0.2 + 0.01 * i as f64,
                })
                .collect(),
            interp: KfInterp::Linear,
        }];
        let err = volume_kf_filter_cmd(&keys, 200_000_000, 0.0, "a0_0", None)
            .expect_err("55-hour volume curve must refuse before allocating timestamps");
        assert_eq!(err.code, cut_core::error_codes::INVALID_ARGS);
        assert!(
            err.message.starts_with("This export would need ")
                && err.message.contains("split the clip"),
            "{}",
            err.message
        );
    }

    #[test]
    fn audio_timestamp_cap_refuses_even_when_bytes_under_byte_limit() {
        // 11.7 hours * 50 updates/s = ~2.11M timestamps (> MAX_AUDIO_CMD_TIMESTAMPS = 2_000_000),
        // even though ~95 MB of command text sits well below the 256 MiB byte budget.
        let pts: Vec<(f64, f64)> = (0..=65)
            .map(|i| (i as f64, 0.2 + 0.01 * i as f64))
            .collect();
        let err = build_audio_cmd_script(&pts, KfInterp::Linear, 42_000_000, "a0_0")
            .expect_err("over 2M audio timestamps must refuse before collecting Vec<u64>");
        assert_eq!(err.code, cut_core::error_codes::INVALID_ARGS);
        assert!(
            err.message.starts_with("This export would need "),
            "{}",
            err.message
        );
        assert!(err.cause.contains("timed updates"), "{}", err.cause);
    }

    #[test]
    fn video_cmd_budget_handles_extreme_durations_and_bad_fps_without_overflow() {
        let start = std::time::Instant::now();
        let err = build_video_cmd_script(1e9, u64::MAX, 100, |_t, _body| {
            panic!("callback must never run when pre-check refuses")
        })
        .expect_err("extreme frame count must refuse before allocation");
        assert_eq!(err.code, cut_core::error_codes::INVALID_ARGS);
        assert!(start.elapsed().as_secs_f64() < 0.2);

        // Zero / NaN fps falls back safely to 30 fps for small durations.
        let ok = build_video_cmd_script(f64::NAN, 100, 15, |_t, b| b.push('x'))
            .expect("small duration with NaN fps stays valid");
        assert!(!ok.is_empty());

        // Zero body bytes keeps short/empty scripts at zero byte estimate.
        assert_eq!(estimate_video_cmd_bytes(30.0, u64::MAX, 0), 0);
    }
}
