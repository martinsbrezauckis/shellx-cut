//! text.rs — minimal text rendering (swash + embedded DejaVu Sans Bold).
//!
//! Rasterizes single-line labels and blits them onto a tiny-skia Pixmap. Enough
//! for the short overlays we need (key-cast chips, captions). Complex shaping /
//! word-wrapping / font fallback would call for cosmic-text — noted as the upgrade.
//! The font is embedded (include_bytes!) so the binary is self-contained; DejaVu's
//! license permits embedding (see assets/FONT-NOTICE.txt).

use std::sync::OnceLock;

use record_core::Rgba;
use swash::{
    scale::{image::Content, Render, ScaleContext, Source},
    FontRef,
};
use tiny_skia::Pixmap;

static FONT: OnceLock<FontRef<'static>> = OnceLock::new();

fn font() -> FontRef<'static> {
    *FONT.get_or_init(|| {
        FontRef::from_index(include_bytes!("../assets/font.ttf").as_slice(), 0)
            .expect("embedded font loads")
    })
}

/// Pixel width of `text` at size `px`.
pub fn text_width(text: &str, px: f32) -> f32 {
    let font = font();
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(&[]).scale(px);
    text.chars()
        .map(|ch| metrics.advance_width(charmap.map(ch)))
        .sum()
}

/// Draw `text` with left edge at `x` and baseline at `baseline_y`.
pub fn draw_text(pm: &mut Pixmap, text: &str, x: f32, baseline_y: f32, px: f32, color: Rgba) {
    let font = font();
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(&[]).scale(px);
    let mut scale_context = ScaleContext::new();
    let mut scaler = scale_context.builder(font).size(px).hint(true).build();
    let renderer = Render::new(&[Source::Outline]);
    let pw = pm.width() as i32;
    let ph = pm.height() as i32;
    let data = pm.data_mut();
    let mut pen = x;
    for ch in text.chars() {
        let glyph_id = charmap.map(ch);
        if let Some(image) = renderer.render(&mut scaler, glyph_id) {
            if image.content == Content::Mask {
                let gx = (pen + image.placement.left as f32).round() as i32;
                let gy = (baseline_y - image.placement.top as f32).round() as i32;
                blit(
                    data,
                    pw,
                    ph,
                    &image.data,
                    image.placement.width as i32,
                    image.placement.height as i32,
                    gx,
                    gy,
                    color,
                );
            }
        }
        pen += metrics.advance_width(glyph_id);
    }
}

/// Alpha-blit an 8-bit coverage bitmap in `color` onto premultiplied RGBA `data`.
#[allow(clippy::too_many_arguments)]
fn blit(
    data: &mut [u8],
    pw: i32,
    ph: i32,
    bm: &[u8],
    bw: i32,
    bh: i32,
    ox: i32,
    oy: i32,
    color: Rgba,
) {
    let (cr, cg, cb, ca) = (
        color.r as f32,
        color.g as f32,
        color.b as f32,
        color.a as f32 / 255.0,
    );
    for j in 0..bh {
        let py = oy + j;
        if py < 0 || py >= ph {
            continue;
        }
        for i in 0..bw {
            let cov = bm[(j * bw + i) as usize] as f32 / 255.0;
            if cov <= 0.0 {
                continue;
            }
            let px = ox + i;
            if px < 0 || px >= pw {
                continue;
            }
            let idx = ((py * pw + px) * 4) as usize;
            let a = cov * ca;
            let inv = 1.0 - a;
            data[idx] = (cr * a + data[idx] as f32 * inv).round().clamp(0.0, 255.0) as u8;
            data[idx + 1] = (cg * a + data[idx + 1] as f32 * inv)
                .round()
                .clamp(0.0, 255.0) as u8;
            data[idx + 2] = (cb * a + data[idx + 2] as f32 * inv)
                .round()
                .clamp(0.0, 255.0) as u8;
            let da = data[idx + 3] as f32 / 255.0;
            data[idx + 3] = ((a + da * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Draw a centered rounded pill chip containing `text`, near the bottom of the
/// frame. `from_bottom` = gap (px) between the chip's bottom edge and the output
/// bottom. Returns the chip's top-left y (so callers can stack chips).
pub fn draw_bottom_chip(
    pm: &mut Pixmap,
    text: &str,
    px: f32,
    from_bottom: f32,
    text_color: Rgba,
    bg: Rgba,
) -> f32 {
    use tiny_skia::{Color, FillRule, Paint, Transform};

    let pw = pm.width() as f32;
    let ph = pm.height() as f32;
    let pad_x = px * 0.6;
    let pad_y = px * 0.35;
    // Truncate with an ellipsis so long captions / key-casts never run off-frame.
    let max_text_w = (pw * 0.92 - pad_x * 2.0).max(px);
    let shown = truncate_to_width(text, px, max_text_w);
    let tw = text_width(&shown, px);
    let chip_w = tw + pad_x * 2.0;
    let chip_h = px + pad_y * 2.0;
    let cx = ((pw - chip_w) / 2.0).max(0.0);
    let cy = (ph - from_bottom - chip_h).max(0.0);

    if let Some(path) = crate::rounded_rect_path(cx, cy, chip_w, chip_h, chip_h * 0.5) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));
        paint.anti_alias = true;
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let baseline = cy + pad_y + px * 0.8;
    draw_text(pm, &shown, cx + pad_x, baseline, px, text_color);
    cy
}

/// Truncate `text` (appending "…") so it fits within `max_w` px at size `px`.
fn truncate_to_width(text: &str, px: f32, max_w: f32) -> String {
    if text_width(text, px) <= max_w {
        return text.to_string();
    }
    let ell_w = text_width("…", px);
    let mut out = String::new();
    let mut w = 0.0;
    for ch in text.chars() {
        let cw = text_width(&ch.to_string(), px);
        if w + cw + ell_w > max_w {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Pixmap;

    #[test]
    fn text_width_positive() {
        assert!(text_width("shellx", 32.0) > 0.0);
        assert!(text_width("", 32.0) == 0.0);
    }

    #[test]
    fn draw_text_marks_pixels() {
        let mut pm = Pixmap::new(200, 60).unwrap();
        pm.fill(tiny_skia::Color::BLACK);
        let before = pm.data().to_vec();
        draw_text(&mut pm, "Hi", 10.0, 40.0, 32.0, Rgba::WHITE);
        assert_ne!(before, pm.data(), "drawing text must change pixels");
    }

    #[test]
    fn chip_renders_glyphs_for_symbols() {
        let mut pm = Pixmap::new(300, 120).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(30, 30, 30, 255));
        // includes the special key-cast glyphs
        draw_bottom_chip(
            &mut pm,
            "⏎ ␣ shellx",
            28.0,
            10.0,
            Rgba::WHITE,
            Rgba::new(20, 22, 30, 220),
        );
        let spread = {
            let d = pm.data();
            let (mut mn, mut mx) = (255u8, 0u8);
            for b in d.iter().step_by(4) {
                mn = mn.min(*b);
                mx = mx.max(*b);
            }
            mx - mn
        };
        assert!(spread > 40, "chip+text should add contrast");
    }
}
