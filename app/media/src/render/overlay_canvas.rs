//! Time-varying placement and source-shaped size on a transparent canvas.
use super::keyframe_cmd::{
    attach_cmd_file, build_video_cmd_script, eval_kf, fmt_cmd_float, overlay_pos_body_bytes,
    shaped_scale_body_bytes, KfCursor,
};
use super::secs;
use cut_core::{error_codes, CutError, Keyframe, KfInterp, KfParam};
use std::fmt::Write;

/// A shaped bubble is never wider than twice the output frame's shorter side.
/// The expression clamp also catches easing curves that overshoot their points.
fn max_bubble_diameter(w: u32, h: u32) -> u32 {
    w.min(h).saturating_mul(2).max(2) & !1
}

pub(super) enum AxisAnim<'a> {
    Static(f64),
    Keys(&'a [(f64, f64)], KfInterp),
}

impl<'a> AxisAnim<'a> {
    pub(super) fn from_opt(opt: Option<&'a (Vec<(f64, f64)>, KfInterp)>, static_px: u32) -> Self {
        match opt {
            Some((pts, interp)) => Self::Keys(pts.as_slice(), *interp),
            None => Self::Static(f64::from(static_px)),
        }
    }
}

pub(super) struct CanvasCmdArgs<'a> {
    pub(super) input_chain: &'a str,
    pub(super) small: &'a str,
    pub(super) canvas: &'a str,
    pub(super) output: &'a str,
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) fps_s: &'a str,
    pub(super) fps: f64,
    pub(super) dur_ms: u64,
    pub(super) vopacity: &'a str,
    pub(super) ow: u32,
    pub(super) oh: u32,
    pub(super) static_opacity: &'a str,
    pub(super) x_anim: AxisAnim<'a>,
    pub(super) y_anim: AxisAnim<'a>,
    pub(super) vfade: &'a str,
    pub(super) shaped_scale: Option<(&'a [(f64, f64)], KfInterp)>,
    pub(super) uniq: &'a str,
    /// Keep HDR: a 10-bit transparent canvas and a 10-bit blend.
    pub(super) keep_hdr: bool,
}

pub(super) fn shaped_scale_points(
    keyframes: &[Keyframe],
    ow: u32,
    w: u32,
    h: u32,
    offset_sec: f64,
) -> Result<Option<(Vec<(f64, f64)>, KfInterp)>, CutError> {
    let Some((pts, interp)) =
        super::keyframe_cmd::extract_kf_points(keyframes, KfParam::Scale, offset_sec)
    else {
        return Ok(None);
    };
    if pts
        .iter()
        .any(|(_, value)| !value.is_finite() || *value <= 0.0)
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "invalid shaped overlay Scale keyframe",
            "Scale points must be finite and positive",
        ));
    }
    let max_scale = f64::from(max_bubble_diameter(w, h)) / f64::from(ow.max(2));
    let clamped = pts
        .into_iter()
        .map(|(t, v)| (t, v.min(max_scale)))
        .collect();
    Ok(Some((clamped, interp)))
}

/// The per-frame reader of an axis whose curve is too long for an expression.
fn long_cursor<'p>(anim: &AxisAnim<'p>, dim: u32) -> Option<(KfCursor<'p>, f64)> {
    match anim {
        AxisAnim::Keys(pts, interp)
            if pts.len() > cut_core::keyframe_limit::MAX_KEYFRAME_POINTS =>
        {
            Some((KfCursor::new(pts, *interp), f64::from(dim)))
        }
        _ => None,
    }
}

pub(super) fn animated_overlay_canvas_cmd_block(
    args: CanvasCmdArgs<'_>,
    cmd_files: Option<&mut Vec<tempfile::TempPath>>,
) -> Result<String, CutError> {
    let CanvasCmdArgs {
        input_chain,
        small,
        canvas,
        output,
        w,
        h,
        fps_s,
        fps,
        dur_ms,
        vopacity,
        ow,
        oh,
        static_opacity,
        x_anim,
        y_anim,
        vfade,
        shaped_scale,
        uniq,
        keep_hdr,
    } = args;

    let mut files = cmd_files;
    let max_d = max_bubble_diameter(w, h);
    let resize = match shaped_scale {
        // The released diameter expression, for curves it could hold.
        Some((s_pts, s_interp)) if s_pts.len() <= cut_core::keyframe_limit::MAX_KEYFRAME_POINTS => {
            let expr = super::kf_expr(s_pts, "t", s_interp);
            let height = if ow == oh {
                "ow".into()
            } else {
                format!("max(2,trunc(ow*{oh}/{ow}/2)*2)")
            };
            format!(
                ",scale=w='max(2,min({max_d},trunc({ow}*({expr})/2)*2))':h='{height}':eval=frame"
            )
        }
        Some((s_pts, s_interp)) => {
            let diam_at = |s_val: f64| -> u32 {
                let truncated = (f64::from(ow) * s_val / 2.0).floor() * 2.0;
                truncated.clamp(2.0, f64::from(max_d)) as u32
            };
            // The released height: the width for a square box, else the box's
            // own aspect, kept even.
            let tall = |d: u32| -> u32 {
                if ow == oh {
                    d
                } else {
                    ((f64::from(d) * f64::from(oh) / f64::from(ow) / 2.0).trunc() * 2.0).max(2.0)
                        as u32
                }
            };
            let d0 = diam_at(eval_kf(s_pts, s_interp, 0.0));
            let mut s_cur = KfCursor::new(s_pts, s_interp);
            let script = build_video_cmd_script(
                fps,
                dur_ms,
                shaped_scale_body_bytes(uniq.len() as u64),
                |t_sec, body| {
                    let d = diam_at(s_cur.eval(t_sec));
                    let _ = write!(body, "scale@sc{uniq} w {d}, scale@sc{uniq} h {}", tall(d));
                },
            )?;
            let escaped = attach_cmd_file(&script, files.as_deref_mut())?;
            let h0 = tall(d0);
            format!(",setpts=N/({fps_s}*TB),sendcmd=f={escaped},scale@sc{uniq}=w={d0}:h={h0}")
        }
        None => format!(",scale={ow}:{oh}"),
    };

    // An axis whose curve the released expression could hold keeps that
    // expression (overlay evaluates it per frame, as before); a longer curve
    // is set per frame from a command file.
    let long = |anim: &AxisAnim<'_>| long_cursor(anim, 0).is_some();
    let axis = |anim: &AxisAnim<'_>, dim: u32| match anim {
        AxisAnim::Static(v) => super::fnum(*v),
        AxisAnim::Keys(pts, interp) if long(anim) => {
            fmt_cmd_float(f64::from(dim) * eval_kf(pts, *interp, 0.0))
        }
        AxisAnim::Keys(pts, interp) => format!("{dim}*({})", super::kf_expr(pts, "t", *interp)),
    };
    let (x0, y0) = (axis(&x_anim, w), axis(&y_anim, h));
    let (x_long, y_long) = (long(&x_anim), long(&y_anim));

    let (canvas_sendcmd, ov_target) = if x_long || y_long {
        let mut axes = [
            ("x", long_cursor(&x_anim, w)),
            ("y", long_cursor(&y_anim, h)),
        ];
        let body_bytes =
            overlay_pos_body_bytes(uniq.len() as u64, axes[0].1.is_some(), axes[1].1.is_some());
        let script = build_video_cmd_script(fps, dur_ms, body_bytes, |t_sec, body| {
            for (name, axis) in &mut axes {
                if let Some((cur, dim)) = axis {
                    if !body.is_empty() {
                        body.push_str(", ");
                    }
                    let v = fmt_cmd_float(*dim * cur.eval(t_sec));
                    let _ = write!(body, "overlay@ov{uniq} {name} {v}");
                }
            }
        })?;
        let escaped = attach_cmd_file(&script, files)?;
        (format!(",sendcmd=f={escaped}"), format!("overlay@ov{uniq}"))
    } else {
        (String::new(), "overlay".into())
    };

    let afmt = if keep_hdr { "yuva420p10le" } else { "yuva420p" };
    let ov_fmt = if keep_hdr { ":format=auto" } else { "" };
    Ok(format!(
        "{input_chain}{vopacity}{resize},setsar=1{static_opacity}[{small}];\n\
         color=c=black@0.0:s={w}x{h}:r={fps_s}:d={},format={afmt}{canvas_sendcmd}[{canvas}];\n\
         [{canvas}][{small}]{ov_target}=threads=1:x='{x0}':y='{y0}':eof_action=pass{ov_fmt}{vfade}[{output}];",
        secs(dur_ms),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::KfPoint;

    fn args<'a>(
        x_anim: AxisAnim<'a>,
        shaped_scale: Option<(&'a [(f64, f64)], KfInterp)>,
    ) -> CanvasCmdArgs<'a> {
        CanvasCmdArgs {
            input_chain: "[in]",
            small: "small",
            canvas: "canvas",
            output: "out",
            w: 1920,
            h: 1080,
            fps_s: "30",
            fps: 30.0,
            dur_ms: 1000,
            vopacity: "",
            ow: 800,
            oh: 800,
            static_opacity: "",
            x_anim,
            y_anim: AxisAnim::Static(0.0),
            vfade: "",
            shaped_scale,
            uniq: "u",
            keep_hdr: false,
        }
    }

    #[test]
    fn rectangular_overlay_graph_remains_exact() {
        let graph =
            animated_overlay_canvas_cmd_block(args(AxisAnim::Static(0.0), None), None).unwrap();
        assert_eq!(
            graph,
            "[in],scale=800:800,setsar=1[small];\n\
             color=c=black@0.0:s=1920x1080:r=30:d=1.000,format=yuva420p[canvas];\n\
             [canvas][small]overlay=threads=1:x='0':y='0':eof_action=pass[out];"
        );
    }

    #[test]
    fn moving_overlay_reads_positions_from_a_command_file() {
        let short = [(0.0, 0.0), (1.0, 0.5)];
        let graph = animated_overlay_canvas_cmd_block(
            args(AxisAnim::Keys(&short, KfInterp::Linear), None),
            None,
        )
        .unwrap();
        assert!(
            graph.contains("[canvas][small]overlay=threads=1:x='1920*(if(lt(t,0),0,"),
            "a curve the released expression holds keeps it: {graph}"
        );
        let pts: Vec<_> = (0..=64).map(|i| (i as f64 / 64.0, 0.5)).collect();
        let graph = animated_overlay_canvas_cmd_block(
            args(AxisAnim::Keys(&pts, KfInterp::Linear), None),
            None,
        )
        .unwrap();
        assert!(
            graph.contains("format=yuva420p,sendcmd=f=<dry-run>[canvas]"),
            "{graph}"
        );
        assert!(
            graph.contains("overlay@ovu=threads=1:x='960':y='0'"),
            "positioned blend must be repeatable: {graph}"
        );
        assert!(!graph.contains("if(lt("), "{graph}");
    }

    #[test]
    fn shaped_scale_caps_overshoot_at_twice_shorter_frame_side() {
        let pts = [(0.0, 999_999_999.0), (1.0, 1.0)];
        let graph = animated_overlay_canvas_cmd_block(
            args(AxisAnim::Static(0.0), Some((&pts, KfInterp::Linear))),
            None,
        )
        .unwrap();
        assert!(
            graph.contains("max(2,min(2160,trunc(800*(if(lt(t,0),999999999,"),
            "{graph}"
        );
        let long: Vec<_> = (0..=64).map(|i| (i as f64, 999_999_999.0)).collect();
        let graph = animated_overlay_canvas_cmd_block(
            args(AxisAnim::Static(0.0), Some((&long, KfInterp::Linear))),
            None,
        )
        .unwrap();
        assert!(graph.contains("scale@scu=w=2160:h=2160"), "{graph}");
    }

    #[test]
    fn keep_hdr_canvas_stays_ten_bit_with_command_files() {
        let pts: Vec<_> = (0..=64).map(|i| (i as f64 / 64.0, 0.5)).collect();
        let mut a = args(AxisAnim::Keys(&pts, KfInterp::Linear), None);
        a.keep_hdr = true;
        let graph = animated_overlay_canvas_cmd_block(a, None).unwrap();
        assert!(
            graph.contains("format=yuva420p10le,sendcmd=f=<dry-run>[canvas]"),
            "{graph}"
        );
        assert!(
            graph.contains("eof_action=pass:format=auto[out]"),
            "{graph}"
        );
    }

    #[test]
    fn shaped_scale_validates_points() {
        let mut keys = vec![Keyframe {
            lead_in: None,
            param: KfParam::Scale,
            points: vec![KfPoint {
                t_ms: 0,
                value: f64::MAX,
            }],
            interp: KfInterp::Linear,
        }];
        assert_eq!(
            shaped_scale_points(&keys, 800, 1920, 1080, 0.0).unwrap(),
            Some((vec![(0.0, 2.7)], KfInterp::Linear))
        );
        keys[0].points[0].value = f64::INFINITY;
        assert!(shaped_scale_points(&keys, 800, 1920, 1080, 0.0).is_err());
        keys[0].points[0].value = 0.0;
        assert!(shaped_scale_points(&keys, 800, 1920, 1080, 0.0).is_err());
    }
}
