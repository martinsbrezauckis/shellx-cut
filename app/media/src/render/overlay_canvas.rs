//! Time-varying placement and diameter on a stable transparent canvas.
use super::secs;
use cut_core::{error_codes, CutError, Keyframe, KfParam};

/// A shaped bubble is never wider than twice the output frame's shorter side.
/// The expression clamp also catches easing curves that overshoot their points.
fn max_bubble_diameter(w: u32, h: u32) -> u32 {
    w.min(h).saturating_mul(2).max(2) & !1
}

pub(super) fn shaped_scale_expr(
    keyframes: &[Keyframe],
    ow: u32,
    w: u32,
    h: u32,
    offset: f64,
) -> Result<Option<String>, CutError> {
    let Some((mut points, interp)) = super::kf_points(keyframes, KfParam::Scale) else {
        return Ok(None);
    };
    if points
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
    for (time, value) in &mut points {
        *time -= offset;
        *value = (*value).min(max_scale);
    }
    Ok(Some(super::kf_expr(&points, "t", interp)))
}

/// Opacity and fade apply once to the completed overlay. `size` is a shaped
/// bubble diameter multiplier; absent preserves the original fixed-size graph.
#[allow(clippy::too_many_arguments)]
pub(super) fn animated_overlay_canvas_block(
    input_chain: &str,
    small: &str,
    canvas: &str,
    output: &str,
    w: u32,
    h: u32,
    fps: &str,
    dur_ms: u64,
    vopacity: &str,
    ow: u32,
    oh: u32,
    static_opacity: &str,
    xexpr: &str,
    yexpr: &str,
    vfade: &str,
    size: Option<&str>,
) -> String {
    let resize = size
        .map(|expr| {
            let max_d = max_bubble_diameter(w, h);
            format!("scale=w='max(2,min({max_d},trunc({ow}*({expr})/2)*2))':h='ow':eval=frame")
        })
        .unwrap_or_else(|| format!("scale={ow}:{oh}"));
    format!(
        "{input_chain}{vopacity},{resize},setsar=1{static_opacity}[{small}];\n\
         color=c=black@0.0:s={w}x{h}:r={fps}:d={},format=yuva420p[{canvas}];\n\
         [{canvas}][{small}]overlay=x='{xexpr}':y='{yexpr}':eof_action=pass{vfade}[{output}];",
        secs(dur_ms),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::{KfInterp, KfPoint};

    #[test]
    fn shaped_scale_caps_overshoot_at_twice_shorter_frame_side() {
        let graph = animated_overlay_canvas_block(
            "[in]",
            "small",
            "canvas",
            "out",
            1920,
            1080,
            "30",
            1000,
            "",
            800,
            800,
            "",
            "0",
            "0",
            "",
            Some("999999999"),
        );
        assert!(graph.contains("max(2,min(2160,trunc(800*(999999999)/2)*2))"));
        assert!(graph.contains(":h='ow':eval=frame"));
    }

    #[test]
    fn shaped_scale_validates_points_before_expression() {
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
            shaped_scale_expr(&keys, 800, 1920, 1080, 0.0).unwrap(),
            Some("2.7".into())
        );
        keys[0].points[0].value = f64::INFINITY;
        assert!(shaped_scale_expr(&keys, 800, 1920, 1080, 0.0).is_err());
        keys[0].points[0].value = 0.0;
        assert!(shaped_scale_expr(&keys, 800, 1920, 1080, 0.0).is_err());
    }

    #[test]
    fn rectangular_overlay_graph_remains_exact() {
        let graph = animated_overlay_canvas_block(
            "[in]", "small", "canvas", "out", 1920, 1080, "30", 1000, "", 800, 800, "", "0", "0",
            "", None,
        );
        assert_eq!(
            graph,
            "[in],scale=800:800,setsar=1[small];\n\
             color=c=black@0.0:s=1920x1080:r=30:d=1.000,format=yuva420p[canvas];\n\
             [canvas][small]overlay=x='0':y='0':eof_action=pass[out];"
        );
    }
}
