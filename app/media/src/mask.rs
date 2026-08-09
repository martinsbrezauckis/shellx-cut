//! mask.rs — bake a vector/freeform MASK (edit.add_mask) to a feathered GRAY alpha
//! PNG: WHITE inside the shape, BLACK outside, with an optional gaussian-feathered
//! edge. The renderer feeds this PNG as a parallel input, `alphamerge`s it onto the
//! effected frame, then `overlay`s that over the original — so the effect (blur/
//! pixelate/black) is scoped to the region (the proven matte composite pattern, NOT
//! `maskedmerge`, which blends per-plane and would mishandle a gray mask's chroma).
//! Rasterized with resvg/tiny-skia (same engine as titles) so all shapes — including
//! arbitrary polygons/freeform — render uniformly.
//!
//! Dependencies: resvg/usvg/tiny-skia (re-exported via resvg). Caller: render.rs
//! (bakes the PNG into the project cache during graph construction) — content-
//! addressed by `ClipMask::cache_tag`, so an identical mask reuses the file.

use cut_core::{error_codes, ClipMask, CutError, MaskShape};
use std::path::Path;

/// Build the SVG for a mask: a WHITE shape on a BLACK full-frame background at
/// `w`×`h`, with an optional gaussian feather. Points are fractions of the frame
/// (rect/ellipse[centre,radii]/polygon).
/// Draw ONE region (shape + normalized points) as a white SVG element on the alpha.
/// Shared by the primary region and every `ClipMask.regions` face-blur entry.
fn region_svg_el(shape: MaskShape, points: &[[f64; 2]], wf: f64, hf: f64) -> Option<String> {
    let px = |p: &[f64; 2]| (p[0] * wf, p[1] * hf);
    match shape {
        MaskShape::Rect => {
            if points.len() < 2 {
                return None;
            }
            let (x0, y0) = px(&points[0]);
            let (x1, y1) = px(&points[1]);
            let (x, y, rw, rh) = (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
            Some(format!(
                "<rect x='{x:.2}' y='{y:.2}' width='{rw:.2}' height='{rh:.2}' fill='white'/>"
            ))
        }
        MaskShape::Ellipse => {
            if points.len() < 2 {
                return None;
            }
            let (cx, cy) = px(&points[0]);
            let (rx, ry) = ((points[1][0] * wf).abs(), (points[1][1] * hf).abs());
            Some(format!(
                "<ellipse cx='{cx:.2}' cy='{cy:.2}' rx='{rx:.2}' ry='{ry:.2}' fill='white'/>"
            ))
        }
        MaskShape::Polygon => {
            if points.len() < 3 {
                return None;
            }
            let pts: String = points
                .iter()
                .map(|p| {
                    let (x, y) = px(p);
                    format!("{x:.2},{y:.2} ")
                })
                .collect();
            Some(format!("<polygon points='{}' fill='white'/>", pts.trim()))
        }
    }
}

fn build_mask_svg(mask: &ClipMask, w: u32, h: u32) -> String {
    let (wf, hf) = (w as f64, h as f64);
    // Feather is a fraction of frame HEIGHT → gaussian stdDeviation in px.
    let feather_px = (mask.feather * hf).max(0.0);
    // UNION of the primary region + every extra region: all share this alpha,
    // so N faces / boxes bake into one PNG and the one-pass effect+overlay is unchanged.
    let mut shape = region_svg_el(mask.shape, &mask.points, wf, hf).unwrap_or_default();
    for r in &mask.regions {
        if let Some(el) = region_svg_el(r.shape, &r.points, wf, hf) {
            shape.push_str(&el);
        }
    }
    let (filter_def, group) = if feather_px > 0.1 {
        let pad = feather_px * 3.0;
        let fx = -pad;
        let fy = -pad;
        let fw2 = wf + (pad * 2.0);
        let fh2 = hf + (pad * 2.0);
        (
            format!(
                "<filter id='f' filterUnits='userSpaceOnUse' x='{fx:.2}' y='{fy:.2}' width='{fw2:.2}' height='{fh2:.2}'>\
                 <feGaussianBlur stdDeviation='{feather_px:.2}'/></filter>"
            ),
            format!("<g filter='url(#f)'>{shape}</g>"),
        )
    } else {
        (String::new(), shape)
    };
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' \
         viewBox='0 0 {w} {h}'><rect width='{w}' height='{h}' fill='black'/>\
         {filter_def}{group}</svg>"
    )
}

/// Bake the mask to a GRAY-on-PNG file at `out_path` (white inside the shape, black
/// outside, feathered). Errors if the SVG fails to parse or the file cannot be
/// written. `invert` is NOT applied here (the renderer negates the mask filter
/// instead, so one baked alpha serves both inside/outside scoping).
pub fn bake_mask_png(mask: &ClipMask, w: u32, h: u32, out_path: &Path) -> Result<(), CutError> {
    let svg = build_mask_svg(mask, w, h);
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(&svg, &opt)
        .map_err(|e| CutError::new(error_codes::FFMPEG, "mask SVG parse failed", e.to_string()))?;
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| {
        CutError::new(
            error_codes::FFMPEG,
            "mask pixmap alloc failed",
            format!("{w}x{h}"),
        )
    })?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|e| CutError::new(error_codes::FFMPEG, "mask PNG encode failed", e.to_string()))?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                "mask PNG output dir create failed",
                e.to_string(),
            )
        })?;
    }
    std::fs::write(out_path, png)
        .map_err(|e| CutError::new(error_codes::FFMPEG, "mask PNG write failed", e.to_string()))?;
    Ok(())
}

/// Rasterize an ARBITRARY SVG document to a (transparent-background) PNG file at
/// `out_path`. Generic sibling of [`bake_mask_png`] — used by the `stickers` asset
/// provider to render its built-in shape catalog to importable overlay PNGs.
/// The SVG controls its own background (omit a backdrop rect for transparency).
pub fn render_svg_png(svg: &str, w: u32, h: u32, out_path: &Path) -> Result<(), CutError> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opt)
        .map_err(|e| CutError::new(error_codes::FFMPEG, "SVG parse failed", e.to_string()))?;
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| {
        CutError::new(
            error_codes::FFMPEG,
            "pixmap alloc failed",
            format!("{w}x{h}"),
        )
    })?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|e| CutError::new(error_codes::FFMPEG, "PNG encode failed", e.to_string()))?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                "PNG output dir create failed",
                e.to_string(),
            )
        })?;
    }
    std::fs::write(out_path, png)
        .map_err(|e| CutError::new(error_codes::FFMPEG, "PNG write failed", e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::MaskEffect;

    fn mask(shape: MaskShape, points: Vec<[f64; 2]>, feather: f64) -> ClipMask {
        ClipMask {
            shape,
            points,
            feather,
            invert: false,
            effect: MaskEffect::Blur,
            strength: None,
            range_ms: None,
            track: None,
            regions: Vec::new(),
        }
    }

    /// The SVG carries a black background + the white shape (+ feather filter when
    /// requested). Sanity of the emitted markup for each shape.
    #[test]
    fn svg_has_black_bg_and_white_shape() {
        let r = build_mask_svg(
            &mask(MaskShape::Rect, vec![[0.1, 0.1], [0.5, 0.5]], 0.0),
            1000,
            1000,
        );
        assert!(r.contains("fill='black'"), "black bg: {r}");
        assert!(
            r.contains("<rect") && r.contains("fill='white'"),
            "white rect: {r}"
        );
        assert!(!r.contains("feGaussianBlur"), "no feather → no blur filter");

        let e = build_mask_svg(
            &mask(MaskShape::Ellipse, vec![[0.5, 0.5], [0.2, 0.3]], 0.05),
            1000,
            1000,
        );
        assert!(
            e.contains("<ellipse") && e.contains("rx='200"),
            "ellipse radii in px: {e}"
        );
        assert!(
            e.contains("feGaussianBlur stdDeviation='50"),
            "feather 0.05*1000=50px: {e}"
        );

        let p = build_mask_svg(
            &mask(
                MaskShape::Polygon,
                vec![[0.1, 0.1], [0.9, 0.2], [0.5, 0.8]],
                0.0,
            ),
            1000,
            1000,
        );
        assert!(
            p.contains("<polygon points='100.00,100.00 900.00,200.00 500.00,800.00'"),
            "polygon pts: {p}"
        );
    }

    #[test]
    fn svg_skips_malformed_regions_without_panicking() {
        let r = build_mask_svg(&mask(MaskShape::Rect, vec![[0.1, 0.1]], 0.0), 1000, 1000);
        assert!(
            r.contains("fill='black'"),
            "mask still has a black backdrop: {r}"
        );
        assert!(
            !r.contains("fill='white'"),
            "malformed rect region is omitted: {r}"
        );

        let e = build_mask_svg(&mask(MaskShape::Ellipse, vec![[0.5, 0.5]], 0.0), 1000, 1000);
        assert!(
            !e.contains("<ellipse"),
            "malformed ellipse region is omitted: {e}"
        );
    }

    #[test]
    fn feather_filter_expands_in_frame_space_for_large_blurs() {
        let r = build_mask_svg(
            &mask(MaskShape::Rect, vec![[0.4, 0.4], [0.6, 0.6]], 0.5),
            1000,
            1000,
        );
        assert!(
            r.contains("filterUnits='userSpaceOnUse'"),
            "filter should not be limited to the shape bbox: {r}"
        );
        assert!(
            r.contains("x='-1500.00'") && r.contains("width='4000.00'"),
            "filter should expand by 3 sigma around the frame: {r}"
        );
    }

    /// Baking writes a real PNG; the shape region is BRIGHT and the corner is DARK
    /// (decode it back and check pixels). Proves resvg actually rasterized the mask.
    #[test]
    fn bakes_a_png_white_inside_black_outside() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("m.png");
        // A centred rect covering the middle of a 100x100 frame.
        let m = mask(MaskShape::Rect, vec![[0.25, 0.25], [0.75, 0.75]], 0.0);
        bake_mask_png(&m, 100, 100, &out).expect("bake");
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[1..4], b"PNG", "wrote a PNG");
        // Decode with tiny-skia and check centre is white, corner is black.
        let pm = tiny_skia::Pixmap::decode_png(&bytes).unwrap();
        let at = |x: u32, y: u32| pm.pixel(x, y).unwrap().red();
        assert!(
            at(50, 50) > 200,
            "centre is inside the rect (white): {}",
            at(50, 50)
        );
        assert!(
            at(2, 2) < 50,
            "corner is outside the rect (black): {}",
            at(2, 2)
        );
    }

    #[test]
    fn bake_reports_output_dir_creation_failure() {
        let dir = tempfile::tempdir().unwrap();
        let file_parent = dir.path().join("not-a-dir");
        std::fs::write(&file_parent, b"file").unwrap();
        let out = file_parent.join("m.png");
        let m = mask(MaskShape::Rect, vec![[0.25, 0.25], [0.75, 0.75]], 0.0);
        let err = bake_mask_png(&m, 16, 16, &out).unwrap_err();
        assert!(
            err.message.contains("output dir"),
            "directory creation failure should not be hidden as a later write failure: {err:?}"
        );
    }
}
