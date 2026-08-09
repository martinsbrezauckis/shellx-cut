//! record-render — the polish compositor + ffmpeg I/O.
//!
//! Role: turn an `EditPlan` (+ a source frame) into the polished output, frame by
//! frame, on the CPU via tiny-skia, then mux with ffmpeg. This is the headless
//! proof that "record data → auto-infer edit → re-render polished" actually
//! produces a real video. Two layers:
//!
//! - `compose_frame` (PURE, unit-testable, NO ffmpeg): background → eased-zoom
//!   source (rounded + shadowed framed card) → synthetic cursor + click ripples.
//!   Deterministic: same (source, plan, t) ⇒ same pixels.
//! - `ffmpeg` glue + `generate_source` + `render_video`: decode a source MP4 to
//!   rawvideo frames, compose each, encode back to MP4 (libx264). At integration
//!   time this compositor is swapped for cutd's GPU path; the plan is the contract.
//!
//! Dependencies: tiny-skia (CPU 2D), ffmpeg (external binary on PATH).
//! Primary callers: record-cli `gen-source` / `render` / `render-frame`.

use record_core::{Background, EditPlan, Rgba};
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, MaskType, Paint,
    PathBuilder, Pixmap, PixmapPaint, Point, Rect, SpreadMode, Stroke, Transform,
};

pub mod captions;
mod compose;
pub mod ffmpeg;
mod render;
mod source;
pub mod text;

pub use compose::{compose_frame, Compositor};
pub use render::{
    render_frame_png, render_video, render_video_audio, render_video_audio_with_control,
};
pub use source::{draw_desktop, generate_source};

/// How long a click ripple animates (ms).
pub const RIPPLE_MS: u64 = 520;

/// Convert a core `Rgba` to a tiny-skia `Color`.
pub(crate) fn color(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// The geometry of the framed "screen card" inside the output, preserving the
/// source aspect ratio and centered within the padded area.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CardRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Base scale that maps source pixels → card pixels at zoom 1.0.
    pub scale_base: f32,
}

/// Compute the framed card rect for a given output size + source size + padding.
pub(crate) fn card_rect(out_w: u32, out_h: u32, src_w: u32, src_h: u32, padding: f64) -> CardRect {
    let ow = out_w as f32;
    let oh = out_h as f32;
    let pad = (padding as f32) * ow.min(oh);
    let avail_w = (ow - 2.0 * pad).max(1.0);
    let avail_h = (oh - 2.0 * pad).max(1.0);
    let src_aspect = src_w as f32 / src_h as f32;
    let (w, h) = if avail_w / avail_h > src_aspect {
        let h = avail_h;
        (h * src_aspect, h)
    } else {
        let w = avail_w;
        (w, w / src_aspect)
    };
    CardRect {
        x: (ow - w) / 2.0,
        y: (oh - h) / 2.0,
        w,
        h,
        scale_base: w / src_w as f32,
    }
}

/// Build a rounded-rectangle path.
pub(crate) fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Fill the background of `pm` per the plan's `Background`. Image and
/// BlurScreen use a neutral fallback when no decoded source frame is supplied.
pub(crate) fn fill_background(pm: &mut Pixmap, bg: &Background) {
    let w = pm.width() as f32;
    let h = pm.height() as f32;
    match bg {
        Background::Solid { color: c } => pm.fill(color(*c)),
        Background::LinearGradient {
            from,
            to,
            angle_deg,
        } => {
            let a = (*angle_deg).to_radians() as f32;
            let (dx, dy) = (a.cos(), a.sin());
            let start = Point::from_xy(w * (0.5 - 0.5 * dx), h * (0.5 - 0.5 * dy));
            let end = Point::from_xy(w * (0.5 + 0.5 * dx), h * (0.5 + 0.5 * dy));
            let shader = LinearGradient::new(
                start,
                end,
                vec![
                    GradientStop::new(0.0, color(*from)),
                    GradientStop::new(1.0, color(*to)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            );
            match shader {
                Some(shader) => {
                    let paint = Paint {
                        shader,
                        anti_alias: false,
                        ..Default::default()
                    };
                    if let Some(rect) = Rect::from_xywh(0.0, 0.0, w, h) {
                        pm.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
                None => pm.fill(color(*from)),
            }
        }
        // Image and blur-screen backgrounds use a neutral fallback at this layer.
        Background::Image { .. } | Background::BlurScreen { .. } => {
            pm.fill(Color::from_rgba8(24, 26, 34, 255));
        }
    }
}

/// Fill `base` with a blurred, darkened copy of a representative source `frame`
/// — a blurred-screen backdrop. Cheap blur via downscale→upscale
/// (bilinear); a static backdrop (one frame), so it stays cacheable in the base.
pub(crate) fn fill_blur_bg(base: &mut Pixmap, frame: &Pixmap) {
    let (ow, oh) = (base.width(), base.height());
    let (sw, sh) = ((ow / 10).max(1), (oh / 10).max(1));
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        ..Default::default()
    };
    if let Some(mut small) = Pixmap::new(sw, sh) {
        small.draw_pixmap(
            0,
            0,
            frame.as_ref(),
            &paint,
            Transform::from_scale(
                sw as f32 / frame.width() as f32,
                sh as f32 / frame.height() as f32,
            ),
            None,
        );
        base.draw_pixmap(
            0,
            0,
            small.as_ref(),
            &paint,
            Transform::from_scale(ow as f32 / sw as f32, oh as f32 / sh as f32),
            None,
        );
    }
    // Darken so the framed card pops.
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(0, 0, 0, 96));
    p.anti_alias = false;
    if let Some(r) = Rect::from_xywh(0.0, 0.0, ow as f32, oh as f32) {
        base.fill_rect(r, &p, Transform::identity(), None);
    }
}

/// Draw a soft drop shadow under the card by stacking translucent rounded rects.
/// (tiny-skia has no gaussian blur; this fake-blur is cheap and reads as a shadow.)
pub(crate) fn draw_card_shadow(
    pm: &mut Pixmap,
    card: &CardRect,
    radius: f32,
    shadow: &record_core::Shadow,
) {
    let layers = 6;
    for i in 0..layers {
        let t = i as f32 / (layers - 1) as f32; // 0..1
        let grow = shadow.blur as f32 * t;
        let alpha = (shadow.opacity as f32) * (1.0 - t) / layers as f32;
        let a8 = (alpha * 255.0).clamp(0.0, 255.0) as u8;
        if a8 == 0 {
            continue;
        }
        if let Some(path) = rounded_rect_path(
            card.x - grow,
            card.y - grow + shadow.offset_y as f32,
            card.w + 2.0 * grow,
            card.h + 2.0 * grow,
            radius + grow,
        ) {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(
                shadow.color.r,
                shadow.color.g,
                shadow.color.b,
                a8,
            ));
            paint.anti_alias = true;
            pm.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

/// Build an output-sized alpha `Mask` shaped like the rounded card rect, so the
/// zoomed source can be drawn straight onto the output (clipped to rounded
/// corners) without an intermediate per-frame pixmap. Built ONCE per render.
pub(crate) fn output_mask_for(out_w: u32, out_h: u32, card: &CardRect, r: f32) -> Option<Mask> {
    let mut tmp = Pixmap::new(out_w, out_h)?;
    let path = rounded_rect_path(card.x, card.y, card.w, card.h, r)?;
    let mut paint = Paint::default();
    paint.set_color(Color::WHITE);
    paint.anti_alias = true;
    tmp.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    Some(Mask::from_pixmap(tmp.as_ref(), MaskType::Alpha))
}

/// Build an output-sized alpha `Mask` shaped like a filled circle (the webcam
/// bubble). Built once per render.
pub(crate) fn circle_mask(out_w: u32, out_h: u32, cx: f32, cy: f32, r: f32) -> Option<Mask> {
    let mut tmp = Pixmap::new(out_w, out_h)?;
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    let path = pb.finish()?;
    let mut paint = Paint::default();
    paint.set_color(Color::WHITE);
    paint.anti_alias = true;
    tmp.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    Some(Mask::from_pixmap(tmp.as_ref(), MaskType::Alpha))
}

/// Draw a pointer-style synthetic cursor centered at (cx, cy) in output coords.
pub(crate) fn draw_cursor(pm: &mut Pixmap, cx: f32, cy: f32, scale: f32) {
    // Arrow polygon in a ~0..18 x 0..26 box; tip at origin.
    let s = scale;
    let pts = [
        (0.0, 0.0),
        (0.0, 18.0),
        (4.6, 13.8),
        (7.6, 20.6),
        (10.4, 19.4),
        (7.4, 12.8),
        (13.0, 12.8),
    ];
    let mut pb = PathBuilder::new();
    pb.move_to(cx + pts[0].0 * s, cy + pts[0].1 * s);
    for p in &pts[1..] {
        pb.line_to(cx + p.0 * s, cy + p.1 * s);
    }
    pb.close();
    if let Some(path) = pb.finish() {
        // White fill + dark outline for contrast on any background.
        let mut fill = Paint::default();
        fill.set_color(Color::WHITE);
        fill.anti_alias = true;
        pm.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
        let mut stroke_paint = Paint::default();
        stroke_paint.set_color(Color::from_rgba8(20, 20, 24, 235));
        stroke_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.5 * s,
            ..Default::default()
        };
        pm.stroke_path(&path, &stroke_paint, &stroke, Transform::identity(), None);
    }
}

/// Draw an expanding click ripple at (cx, cy) with progress `p` in [0,1].
pub(crate) fn draw_ripple(pm: &mut Pixmap, cx: f32, cy: f32, p: f32, base_r: f32) {
    let r = base_r * (0.3 + 0.7 * p);
    let alpha = ((1.0 - p) * 200.0).clamp(0.0, 255.0) as u8;
    if alpha == 0 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.push_circle(cx, cy, r);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(120, 190, 255, alpha));
        paint.anti_alias = true;
        let stroke = Stroke {
            width: (base_r * 0.12).max(1.5),
            ..Default::default()
        };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Linear-interpolate a cursor sample list at time `t_ms` (screen pixels).
pub(crate) fn cursor_sample_at(
    samples: &[record_core::CursorSample],
    t_ms: u64,
) -> Option<(f64, f64)> {
    let n = samples.len();
    if n == 0 {
        return None;
    }
    if t_ms <= samples[0].t_ms {
        return Some((samples[0].x, samples[0].y));
    }
    let last = samples[n - 1];
    if t_ms >= last.t_ms {
        return Some((last.x, last.y));
    }
    let mut i = 0;
    while i + 1 < n && samples[i + 1].t_ms <= t_ms {
        i += 1;
    }
    let a = samples[i];
    let b = samples[i + 1];
    let span = (b.t_ms - a.t_ms).max(1) as f64;
    let f = (t_ms - a.t_ms) as f64 / span;
    Some((a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f))
}

/// Output settings derived from an `EditPlan` (the polished output is the source
/// resolution by default; an explicit reframe aspect changes it).
pub fn output_size(plan: &EditPlan) -> (u32, u32) {
    match plan.reframe {
        record_core::Reframe::Aspect { w, h } => {
            // Fit the requested aspect to the source height, even dims.
            let target_aspect = w as f32 / h as f32;
            let oh = plan.source_h;
            let ow = ((oh as f32 * target_aspect).round() as u32) & !1;
            (ow.max(2), oh & !1)
        }
        record_core::Reframe::None => (plan.source_w & !1, plan.source_h & !1),
    }
}
