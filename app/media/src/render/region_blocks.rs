//! Region-scoped composite blocks for masks, tracked masks, power windows, and
//! adjustment layers. Kept together so `render.rs` stays within its size budget
//! while sharing one 8-bit SDR vs 10-bit Keep HDR pixel-format contract.

use super::hdr::{adjustment_sdr_only_features, mask_black_filter, wrap_sdr_filter_if, GraphColor};
use super::{effect_filter, fnum, grade_filter};

/// The ffmpeg filter (NO leading comma) that produces the EFFECTED copy of a frame
/// for a mask region (edit.add_mask). Applied to a full-frame split copy; the
/// `alphamerge`+`overlay` then scopes it to the shape. `strength`: blur → gaussian
/// sigma px (default 15); pixelate → mosaic block px (default 16, even); black
/// ignores it (a solid black fill — a hard censor).
pub(super) fn mask_effect_filter(mask: &cut_core::ClipMask, keep_hdr: bool) -> String {
    use cut_core::MaskEffect as ME;
    match mask.effect {
        ME::Blur => {
            let sigma = mask.strength.unwrap_or(15.0).clamp(0.5, 200.0);
            format!("gblur=sigma={}", fnum(sigma))
        }
        ME::Pixelate => {
            let b = ((mask.strength.unwrap_or(16.0).clamp(2.0, 256.0) as u32) & !1u32).max(2);
            format!("pixelize=width={b}:height={b}")
        }
        ME::Black => mask_black_filter(keep_hdr).to_string(),
    }
}

/// The masked-region COMPOSITE block (edit.add_mask) for a base/overlay segment.
/// Returns the multi-line filtergraph that splits `in_label` into base + effected
/// copies, scopes the effect to the baked shape alpha (white = inside; `,negate`
/// when `invert`), and `overlay`s it back — producing `out_label`. `mask_idx` is the
/// parallel mask-PNG input. Uses the proven matte `alphamerge`+`overlay` pattern
/// (NOT `maskedmerge`, which blends per-plane and would mishandle a gray mask's
/// chroma). `uniq` makes every intermediate label distinct. Empty when no mask.
#[allow(clippy::too_many_arguments)]
pub(super) fn mask_block(
    mask: &cut_core::ClipMask,
    mask_idx: usize,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    vfade: &str,
    gc: GraphColor,
) -> String {
    let neg = if mask.invert { ",negate" } else { "" };
    let fx = mask_effect_filter(mask, gc.keep_hdr());
    let enable = match mask.range_ms {
        Some([a, b]) => format!(
            ":enable='between(t,{},{})'",
            fnum(a as f64 / 1000.0),
            fnum(b as f64 / 1000.0)
        ),
        None => String::new(),
    };
    let ma_fmt = if gc.keep_hdr() {
        ",format=yuva420p10le"
    } else {
        ""
    };
    format!(
        "[{in_label}]split=2[mb{uniq}][mf{uniq}];\n\
         [mf{uniq}]{fx}[mx{uniq}];\n\
         [{mask_idx}:v]scale={w}:{h}:flags=bilinear,format=gray{neg}[mm{uniq}];\n\
         [mx{uniq}][mm{uniq}]alphamerge{ma_fmt}[ma{uniq}];\n\
         [mb{uniq}][ma{uniq}]overlay=shortest=1{enable}{ov},format={pix}{vfade}[{out_label}];",
        ov = gc.ov,
        pix = gc.pix,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(super) fn grade_window_block(
    gw: &cut_core::GradeWindow,
    window_idx: usize,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    vfade: &str,
) -> String {
    grade_window_block_for(
        gw,
        window_idx,
        in_label,
        out_label,
        uniq,
        w,
        h,
        vfade,
        GraphColor::sdr(),
    )
}

/// The POWER-WINDOW composite block (edit.grade_window) — a REGION-scoped grade on the
/// running per-clip frame `in_label`, producing `out_label`.
#[allow(clippy::too_many_arguments)]
pub(super) fn grade_window_block_for(
    gw: &cut_core::GradeWindow,
    window_idx: usize,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    vfade: &str,
    gc: GraphColor,
) -> String {
    let neg = if gw.window.invert { ",negate" } else { "" };
    let grade = wrap_sdr_filter_if(gc.hdr, &grade_filter(Some(&gw.grade)), true);
    let ga_fmt = if gc.keep_hdr() {
        ",format=yuva420p10le"
    } else {
        ""
    };
    format!(
        "[{in_label}]split=2[gb{uniq}][gf{uniq}];\n\
         [gf{uniq}]null{grade},format={pix}[gx{uniq}];\n\
         [{window_idx}:v]scale={w}:{h}:flags=bilinear,format=gray{neg}[gm{uniq}];\n\
         [gx{uniq}][gm{uniq}]alphamerge{ga_fmt}[ga{uniq}];\n\
         [gb{uniq}][ga{uniq}]overlay=shortest=1{ov},format={pix}{vfade}[{out_label}];",
        pix = gc.pix,
        ov = gc.ov,
    )
}

/// The ADJUSTMENT-LAYER composite block (edit.adjustment) — a TIME-GATED non-
/// destructive grade/effect pass on the running composite `in_label`, producing
/// `out_label`.
pub(super) fn adjustment_block(
    adj: &cut_core::EdlAdjustment,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    gc: GraphColor,
) -> String {
    let raw = format!(
        "{}{}",
        grade_filter(adj.grade.as_ref()),
        effect_filter(&adj.effects, false)
    );
    let sdr_only = !adjustment_sdr_only_features(adj.grade.as_ref(), &adj.effects).is_empty();
    let chain = wrap_sdr_filter_if(gc.hdr, &raw, sdr_only);
    let [a, b] = adj.range_ms;
    let enable = format!(
        ":enable='between(t,{},{})'",
        fnum(a as f64 / 1000.0),
        fnum(b as f64 / 1000.0)
    );
    let jg_fmt = if gc.keep_hdr() {
        format!(",format={}", gc.pix)
    } else {
        String::new()
    };
    format!(
        "[{in_label}]split=2[jb{uniq}][jf{uniq}];\n\
         [jf{uniq}]null{chain},format={pix}[jg{uniq}];\n\
         [jb{uniq}][jg{uniq}]overlay=eof_action=pass{enable}{ov}{jg_fmt}[{out_label}];",
        pix = gc.pix,
        ov = gc.ov,
    )
}
