//! Tracked redaction masks. The released form painted every region with one
//! per-pixel `geq` whose nesting grew with each motion point; FFmpeg refuses
//! that beyond about 96 points and it is slow well before. Here each region is
//! a plain shape (`drawbox` for a rectangle, a pre-drawn ellipse patch placed
//! with `overlay`) moved by one per-frame `sendcmd` file, so the cost no longer
//! depends on the number of points. Shape sizes, centres, the feather and the
//! inclusive pixel edges follow the released expression.

use cut_core::{ClipMask, CutError, KfInterp, MaskShape, MaskTrackPoint};
use std::fmt::Write;
use tempfile::TempPath;

use super::fnum;
use super::hdr::GraphColor;
use super::keyframe_cmd::{
    attach_cmd_file, build_video_cmd_script, command_budget_error, decimal_digits,
    estimate_video_cmd_bytes, eval_kf, KfCursor, MAX_RENDER_CMD_BYTES,
};
use super::region_blocks::mask_effect_filter;

type Track = Vec<(f64, f64)>;

/// The released expression carried every number with six decimals.
fn r6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// One region: half sizes (rectangle) or radii (ellipse) in pixels, and its
/// centre over time. A centre track needs two points, as in the released form;
/// otherwise the static centre from `points` is used.
struct Region {
    ellipse: bool,
    a: f64,
    b: f64,
    cx: Track,
    cy: Track,
    target: String,
}

impl Region {
    fn new(
        shape: MaskShape,
        points: &[[f64; 2]],
        track: Option<&[MaskTrackPoint]>,
        target: String,
        (wf, hf): (f64, f64),
    ) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let (a, b, scx, scy) = match shape {
            MaskShape::Rect => (
                (points[1][0] - points[0][0]).abs() / 2.0 * wf,
                (points[1][1] - points[0][1]).abs() / 2.0 * hf,
                (points[0][0] + points[1][0]) / 2.0 * wf,
                (points[0][1] + points[1][1]) / 2.0 * hf,
            ),
            MaskShape::Ellipse => (
                (points[1][0] * wf).max(1.0),
                (points[1][1] * hf).max(1.0),
                points[0][0] * wf,
                points[0][1] * hf,
            ),
            MaskShape::Polygon => return None,
        };
        let (cx, cy) = match track {
            Some(tr) if tr.len() >= 2 => tr
                .iter()
                .map(|p| {
                    let t = r6(p.t_ms as f64 / 1000.0);
                    ((t, r6(p.cx * wf)), (t, r6(p.cy * hf)))
                })
                .unzip(),
            _ => (vec![(0.0, r6(scx))], vec![(0.0, r6(scy))]),
        };
        Some(Self {
            ellipse: matches!(shape, MaskShape::Ellipse),
            a: r6(a),
            b: r6(b),
            cx,
            cy,
            target,
        })
    }

    fn moving(&self) -> bool {
        self.cx.len() >= 2
    }

    fn filter(&self) -> &'static str {
        if self.ellipse {
            "overlay"
        } else {
            "drawbox"
        }
    }

    /// The pixels the released expression covered for a centre `(cx, cy)`.
    /// Rectangle: `x y w h` with `cx-a <= X <= cx+a`; ellipse: the patch's top
    /// left corner, placing its centre on the nearest whole pixel.
    fn place(&self, cx: f64, cy: f64) -> [i64; 4] {
        if self.ellipse {
            let (px, py) = self.patch_centre();
            return [cx.round() as i64 - px, cy.round() as i64 - py, 0, 0];
        }
        let span = |c: f64, half: f64| {
            let (lo, hi) = ((c - half).ceil(), (c + half).floor());
            if hi < lo || !lo.is_finite() || !hi.is_finite() {
                // Nothing is covered; a zero size would mean "whole frame".
                (-100_000, 1)
            } else {
                (lo as i64, (hi - lo) as i64 + 1)
            }
        };
        let (x, w) = span(cx, self.a);
        let (y, h) = span(cy, self.b);
        [x, y, w, h]
    }

    fn patch_centre(&self) -> (i64, i64) {
        (self.a.ceil() as i64 + 1, self.b.ceil() as i64 + 1)
    }

    fn command(&self, [x, y, w, h]: [i64; 4], out: &mut String) {
        let f = self.filter();
        let t = &self.target;
        let _ = write!(out, "{f}@{t} x {x}, {f}@{t} y {y}");
        if !self.ellipse {
            let _ = write!(out, ", {f}@{t} w {w}, {f}@{t} h {h}");
        }
    }
}

pub(super) fn moving_region_body_bytes(mask: &ClipMask, uniq_len: u64) -> u64 {
    let mut total = 0u64;
    let mut moving = 0u64;
    for (i, (shape, points, track)) in
        std::iter::once((mask.shape, &mask.points, mask.track.as_deref()))
            .chain(
                mask.regions
                    .iter()
                    .map(|r| (r.shape, &r.points, r.track.as_deref())),
            )
            .enumerate()
    {
        if points.len() < 2
            || matches!(shape, MaskShape::Polygon)
            || !track.is_some_and(|tr| tr.len() >= 2)
        {
            continue;
        }
        // Target is "m{uniq}_r{i}" -> 3 + uniq_len + digits(i).
        let target_len = 3u64
            .saturating_add(uniq_len)
            .saturating_add(decimal_digits(i as u64));
        let cmd_bytes = if matches!(shape, MaskShape::Ellipse) {
            // "overlay@{t} x {x}, overlay@{t} y {y}"
            30u64.saturating_add(2u64.saturating_mul(target_len))
        } else {
            // "drawbox@{t} x {x}, drawbox@{t} y {y}, drawbox@{t} w {w}, drawbox@{t} h {h}"
            62u64.saturating_add(4u64.saturating_mul(target_len))
        };
        if moving > 0 {
            total = total.saturating_add(2);
        }
        total = total.saturating_add(cmd_bytes);
        moving = moving.saturating_add(1);
    }
    total
}

/// The masked-region composite for a tracked mask: split the clip three ways
/// (base / effected / mask source), draw the moving regions white on black,
/// `alphamerge` the effect into them and lay it back over the base (still
/// `shortest=1`, still range-gated). Command files join `cmd_files`, which the
/// caller keeps until FFmpeg finishes; `None` emits a placeholder path.
#[allow(clippy::too_many_arguments)]
pub(super) fn mask_block_tracked_cmd(
    mask: &ClipMask,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    fps: f64,
    dur_ms: u64,
    vfade: &str,
    gc: GraphColor,
    cmd_files: Option<&mut Vec<TempPath>>,
) -> Result<String, CutError> {
    let body_bytes = moving_region_body_bytes(mask, uniq.len() as u64);
    if body_bytes > 0 {
        let est = estimate_video_cmd_bytes(fps, dur_ms, body_bytes);
        if est > MAX_RENDER_CMD_BYTES {
            return Err(command_budget_error(est, None));
        }
    }

    let size = (w as f64, h as f64);
    let fps_s = super::fps_str(fps);
    let regions: Vec<Region> = std::iter::once((mask.shape, &mask.points, mask.track.as_deref()))
        .chain(
            mask.regions
                .iter()
                .map(|r| (r.shape, &r.points, r.track.as_deref())),
        )
        .enumerate()
        .filter_map(|(i, (shape, points, track))| {
            Region::new(shape, points, track, format!("m{uniq}_r{i}"), size)
        })
        .collect();

    let mut out = String::new();
    for (i, r) in regions.iter().enumerate().filter(|(_, r)| r.ellipse) {
        let (px, py) = r.patch_centre();
        let _ = writeln!(
            out,
            "color=c=black@0.0:s={}x{}:r={fps_s}:d=0.1,format=gbrap,\
             geq=r=255:g=255:b=255:a='255*lte(pow((X-{px})/{a},2)+pow((Y-{py})/{b},2),1)',\
             trim=end_frame=1,loop=-1:size=1:start=0,setpts=N/({fps_s}*TB)[mp{uniq}_{i}];",
            2 * px + 1,
            2 * py + 1,
            a = fnum(r.a),
            b = fnum(r.b),
        );
    }
    let fx = mask_effect_filter(mask, gc.keep_hdr());
    let _ = writeln!(
        out,
        "[{in_label}]split=3[mb{uniq}][mf{uniq}][mg{uniq}];\n[mf{uniq}]{fx}[mx{uniq}];"
    );

    let sendcmd = if regions.iter().any(Region::moving) {
        let mut cursors: Vec<_> = regions
            .iter()
            .filter(|r| r.moving())
            .map(|r| {
                (
                    r,
                    KfCursor::new(&r.cx, KfInterp::Linear),
                    KfCursor::new(&r.cy, KfInterp::Linear),
                )
            })
            .collect();
        let script = build_video_cmd_script(fps, dur_ms, body_bytes, |t, body| {
            for (i, (r, cx, cy)) in cursors.iter_mut().enumerate() {
                if i > 0 {
                    body.push_str(", ");
                }
                r.command(r.place(cx.eval(t), cy.eval(t)), body);
            }
        })?;
        format!(",sendcmd=f={}", attach_cmd_file(&script, cmd_files)?)
    } else {
        String::new()
    };
    let mut cur = format!("mc{uniq}");
    let _ = writeln!(
        out,
        "[mg{uniq}]format=gbrp,lutrgb=r=0:g=0:b=0{sendcmd}[{cur}];"
    );
    for (i, r) in regions.iter().enumerate() {
        let next = format!("mc{uniq}_{i}");
        let [x, y, bw, bh] = r.place(
            eval_kf(&r.cx, KfInterp::Linear, 0.0),
            eval_kf(&r.cy, KfInterp::Linear, 0.0),
        );
        let name = if r.moving() {
            format!("{}@{}", r.filter(), r.target)
        } else {
            r.filter().to_string()
        };
        if r.ellipse {
            let _ = writeln!(
                out,
                "[{cur}][mp{uniq}_{i}]{name}=format=gbrp:shortest=1:x={x}:y={y}[{next}];"
            );
        } else {
            let _ = writeln!(
                out,
                "[{cur}]{name}=x={x}:y={y}:w={bw}:h={bh}:color=white:t=fill[{next}];"
            );
        }
        cur = next;
    }

    // Exact 0/255 coverage whatever range the conversions assume, then the
    // released feather (a fraction of frame height) and invert.
    let feather_px = mask.feather * h as f64;
    let feather = if feather_px > 0.1 {
        format!(",gblur=sigma={}", fnum(feather_px))
    } else {
        String::new()
    };
    let neg = if mask.invert { ",negate" } else { "" };
    let enable = match mask.range_ms {
        Some([a, b]) => format!(
            ":enable='between(t,{},{})'",
            fnum(a as f64 / 1000.0),
            fnum(b as f64 / 1000.0)
        ),
        None => String::new(),
    };
    // Keep HDR: the effected copy keeps 10 bits through the merge and the
    // overlay blends in 10-bit YUV; SDR keeps the released graph.
    let ma_fmt = if gc.keep_hdr() {
        ",format=yuva420p10le"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "[{cur}]format=gray,lut=c0='if(gt(val,127),255,0)'{feather}{neg}[mm{uniq}];\n\
         [mx{uniq}][mm{uniq}]alphamerge{ma_fmt}[ma{uniq}];\n\
         [mb{uniq}][ma{uniq}]overlay=shortest=1{enable}{ov},format={pix}{vfade}[{out_label}];",
        ov = gc.ov,
        pix = gc.pix,
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::types::MaskRegion;
    use cut_core::MaskEffect;

    fn track(n: u64, bias: f64) -> Vec<MaskTrackPoint> {
        (0..n)
            .map(|i| MaskTrackPoint {
                t_ms: i * 33,
                cx: 0.2 + 0.002 * i as f64 + bias,
                cy: 0.5,
            })
            .collect()
    }

    #[test]
    fn eight_regions_300_points_use_one_command_file_without_nested_expressions() {
        let regions = (1..8)
            .map(|i| MaskRegion {
                shape: if i % 2 == 0 {
                    MaskShape::Rect
                } else {
                    MaskShape::Ellipse
                },
                points: vec![[0.5, 0.5], [0.1, 0.1]],
                track: Some(track(300, i as f64 * 0.01)),
            })
            .collect();
        let mask = ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.0, 0.0], [0.1, 0.1]],
            feather: 0.25,
            invert: true,
            effect: MaskEffect::Blur,
            strength: Some(20.0),
            range_ms: Some([500, 9000]),
            track: Some(track(300, 0.0)),
            regions,
        };
        let block = mask_block_tracked_cmd(
            &mask,
            "in",
            "out",
            "u0",
            1280,
            720,
            30.0,
            10_000,
            "",
            GraphColor::sdr(),
            None,
        )
        .unwrap();
        assert!(!block.contains("if(lt("), "{block}");
        assert_eq!(block.matches("sendcmd=f=<dry-run>").count(), 1, "{block}");
        assert!(block.contains("drawbox@mu0_r0="), "{block}");
        assert!(block.contains("overlay@mu0_r1=format=gbrp"), "{block}");
        assert!(block.contains(",gblur=sigma=180,negate[mmu0]"), "{block}");
        assert!(block.contains(":enable='between(t,0.5,9)'"), "{block}");
    }

    #[test]
    fn released_sizes_centres_and_inclusive_edges() {
        let rect = Region::new(
            MaskShape::Rect,
            &[[0.0, 0.0], [0.25, 0.35]],
            None,
            "r".into(),
            (320.0, 240.0),
        )
        .unwrap();
        // Half sizes 40 x 42, static centre (40, 42): X 0..=80, Y 0..=84.
        assert_eq!(rect.place(40.0, 42.0), [0, 0, 81, 85]);
        assert_eq!(rect.place(40.5, 42.0), [1, 0, 80, 85]);
        assert!(!rect.moving());
        // Ellipse points are [centre, radii]; one track point keeps the static centre.
        let one = [MaskTrackPoint {
            t_ms: 0,
            cx: 0.9,
            cy: 0.9,
        }];
        let ellipse = Region::new(
            MaskShape::Ellipse,
            &[[0.5, 0.5], [0.13, 0.25]],
            Some(&one),
            "e".into(),
            (320.0, 240.0),
        )
        .unwrap();
        assert_eq!((ellipse.cx[0].1, ellipse.cy[0].1), (160.0, 120.0));
        assert_eq!(ellipse.patch_centre(), (43, 61));
        assert_eq!(ellipse.place(160.0, 120.0), [117, 59, 0, 0]);
    }

    #[test]
    fn oversized_moving_mask_block_refuses_before_string_allocation() {
        let two_pts = |cx0: f64, cx1: f64| {
            vec![
                MaskTrackPoint {
                    t_ms: 0,
                    cx: cx0,
                    cy: 0.5,
                },
                MaskTrackPoint {
                    t_ms: 480_000,
                    cx: cx1,
                    cy: 0.5,
                },
            ]
        };
        let regions = (1..256)
            .map(|i| MaskRegion {
                shape: MaskShape::Rect,
                points: vec![[0.1, 0.1], [0.2, 0.2]],
                track: Some(two_pts(0.1 + (i as f64) * 0.001, 0.8 - (i as f64) * 0.001)),
            })
            .collect();
        let mask = ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.1, 0.1], [0.2, 0.2]],
            feather: 0.0,
            invert: false,
            effect: MaskEffect::Blur,
            strength: Some(20.0),
            range_ms: None,
            track: Some(two_pts(0.1, 0.9)),
            regions,
        };
        let err = mask_block_tracked_cmd(
            &mask,
            "in",
            "out",
            "v0region",
            1280,
            720,
            30.0,
            480_000,
            "",
            GraphColor::sdr(),
            None,
        )
        .expect_err("256 moving regions over 8 minutes at 30 fps must exceed the byte budget");
        assert_eq!(err.code, cut_core::error_codes::INVALID_ARGS);
        assert!(
            err.message.starts_with("This export would need ")
                && err
                    .message
                    .contains("MB of movement data; split the clip or reduce the moving regions"),
            "{}",
            err.message
        );
    }
}
