//! Compile bounded keyframe tracks without recopying a growing expression.
use super::{ease_frac_expr, fnum};
use cut_core::KfInterp;
use std::fmt::Write as _;

/// Emit the legacy piecewise FFmpeg time expression in `var` (`t`, `T`, or `on`).
/// Values clamp before the first and after the last point. Each segment and
/// closing parenthesis is appended once, so construction is linear in points.
pub(super) fn kf_expr(points: &[(f64, f64)], var: &str, interp: KfInterp) -> String {
    let hold = matches!(interp, KfInterp::Hold);
    let linear = matches!(interp, KfInterp::Linear);
    match points.len() {
        0 => "0".to_string(),
        1 => fnum(points[0].1),
        n => {
            let mut expr = String::with_capacity(n.saturating_mul(128));
            write!(
                expr,
                "if(lt({},{}),{},",
                var,
                fnum(points[0].0),
                fnum(points[0].1)
            )
            .unwrap();
            for pair in points.windows(2) {
                let (t0, v0) = pair[0];
                let (t1, v1) = pair[1];
                let seg = if hold || (t1 - t0).abs() < 1e-9 {
                    fnum(v0)
                } else if linear {
                    format!(
                        "({}+({})*({}-{})/{})",
                        fnum(v0),
                        fnum(v1 - v0),
                        var,
                        fnum(t0),
                        fnum(t1 - t0)
                    )
                } else {
                    let frac = format!("(({}-{})/{})", var, fnum(t0), fnum(t1 - t0));
                    format!(
                        "({}+({})*{})",
                        fnum(v0),
                        fnum(v1 - v0),
                        ease_frac_expr(interp, &frac)
                    )
                };
                write!(expr, "if(lt({},{}),{},", var, fnum(t1), seg).unwrap();
            }
            expr.push_str(&fnum(points[n - 1].1));
            for _ in 0..n {
                expr.push(')');
            }
            expr
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured by the unmodified 16a78de60 builder before changing its loop.
    #[test]
    fn pre_fix_golden_strings_for_one_two_and_limit_points() {
        assert_eq!(
            kf_expr(&[(1.0, 0.5)], "T", KfInterp::Linear),
            include_str!("goldens/golden_one.txt")
        );
        assert_eq!(
            kf_expr(&[(0.0, 0.0), (2.0, 1.0)], "T", KfInterp::Linear),
            include_str!("goldens/golden_two.txt")
        );
        let points: Vec<_> = (0..cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)
            .map(|i| (i as f64, (i % 2) as f64))
            .collect();
        assert_eq!(
            kf_expr(&points, "T", KfInterp::Hold),
            include_str!("goldens/golden_limit.txt")
        );
    }

    // The former builder is retained as the text oracle; it is used only by
    // these small tests and deliberately keeps its old repeated-copy behavior.
    fn legacy_kf_expr(points: &[(f64, f64)], var: &str, interp: cut_core::KfInterp) -> String {
        let hold = matches!(interp, cut_core::KfInterp::Hold);
        let linear = matches!(interp, cut_core::KfInterp::Linear);
        match points.len() {
            0 => "0".to_string(),
            1 => fnum(points[0].1),
            n => {
                // var >= last t → the last value (the innermost else).
                let mut expr = fnum(points[n - 1].1);
                for i in (0..n - 1).rev() {
                    let (t0, v0) = points[i];
                    let (t1, v1) = points[i + 1];
                    let seg = if hold || (t1 - t0).abs() < 1e-9 {
                        fnum(v0)
                    } else if linear {
                        // v0 + (v1-v0)*(var-t0)/(t1-t0) — the EXACT legacy string.
                        format!(
                            "({}+({})*({}-{})/{})",
                            fnum(v0),
                            fnum(v1 - v0),
                            var,
                            fnum(t0),
                            fnum(t1 - t0)
                        )
                    } else {
                        // v0 + (v1-v0)*ease( (var-t0)/(t1-t0) )
                        let frac = format!("(({}-{})/{})", var, fnum(t0), fnum(t1 - t0));
                        format!(
                            "({}+({})*{})",
                            fnum(v0),
                            fnum(v1 - v0),
                            ease_frac_expr(interp, &frac)
                        )
                    };
                    expr = format!("if(lt({},{}),{},{})", var, fnum(t1), seg, expr);
                }
                // Clamp before the first point to v0.
                format!(
                    "if(lt({},{}),{},{})",
                    var,
                    fnum(points[0].0),
                    fnum(points[0].1),
                    expr
                )
            }
        }
    }

    #[test]
    fn legacy_expression_text_for_one_two_and_limit_points() {
        let cases = [
            vec![(1.0, 0.5)],
            vec![(0.0, 0.0), (2.0, 1.0)],
            (0..cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)
                .map(|i| (i as f64, (i % 2) as f64))
                .collect(),
            (0..96).map(|i| (i as f64, (i % 2) as f64)).collect(),
            (0..300).map(|i| (i as f64, (i % 2) as f64)).collect(),
        ];
        for points in cases {
            for interp in [KfInterp::Hold, KfInterp::Linear, KfInterp::EaseInOutBounce] {
                assert_eq!(
                    kf_expr(&points, "T", interp),
                    legacy_kf_expr(&points, "T", interp)
                );
            }
        }
    }

    #[test]
    fn ffmpeg_accepts_limit_with_deep_easing() {
        if std::process::Command::new(crate::ffmpeg::ffmpeg_bin())
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let points: Vec<_> = (0..cut_core::keyframe_limit::MAX_KEYFRAME_POINTS)
            .map(|i| (i as f64, (i % 2) as f64))
            .collect();
        let expr = kf_expr(&points, "T", KfInterp::EaseInOutBounce);
        let filter = format!(
            "format=yuva444p,geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='255*clip(({expr}),0,1)'"
        );
        // A real render uses a complex graph and the shared runner moves long
        // graphs to a file using the selected FFmpeg build's supported option.
        // Passing this expression directly as -vf exceeds Windows' command line.
        assert!(filter.len() > 30_000);
        let args = [
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=s=16x16:d=0.1",
            "-filter_complex",
            &filter,
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ]
        .map(str::to_owned);
        crate::ffmpeg::run_ffmpeg(&args).unwrap();
    }
}
