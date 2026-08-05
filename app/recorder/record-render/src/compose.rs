//! compose.rs — the polish compositor (pure, deterministic, no ffmpeg).
//!
//! `Compositor::new(plan)` precomputes the INVARIANT layers once — background,
//! drop shadow, and the rounded-card alpha mask — into a `base` pixmap. Each
//! `frame(source, t_ms)` then clones that base and draws only the DYNAMIC parts:
//! the eased-zoom source (clipped to the rounded card via the prebuilt mask),
//! click ripples, and the synthetic cursor. Cloning a base pixmap per frame is a
//! cheap memcpy; rebuilding the gradient + shadow + mask every frame is not — so
//! this is the production-grade path. Same inputs ⇒ identical pixels (golden-testable).

use record_core::EditPlan;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, Mask, MaskType, Paint, PathBuilder, Pixmap,
    PixmapPaint, Stroke, Transform,
};

use crate::{
    card_rect, circle_mask, cursor_sample_at, draw_card_shadow, draw_cursor, draw_ripple,
    fill_background, output_mask_for, output_size, rounded_rect_path, CardRect, RIPPLE_MS,
};

/// Webcam-bubble geometry for one rendered frame.
struct WebcamGeom {
    x: f32,
    y: f32,
    d: f32,
    mask: Mask,
}

/// A reusable compositor that caches per-render invariant layers.
pub struct Compositor<'a> {
    plan: &'a EditPlan,
    out_w: u32,
    out_h: u32,
    card: CardRect,
    /// Background + shadow, rendered once; cloned per frame.
    base: Pixmap,
    /// Rounded-card clip mask (output-sized), or None when the frame is disabled.
    mask: Option<Mask>,
    /// Caption lines (loaded once from the plan's transcript), empty if no captions.
    caption_lines: Vec<crate::captions::CaptionLine>,
}

impl<'a> Compositor<'a> {
    /// Build the compositor, precomputing background + shadow + mask.
    pub fn new(plan: &'a EditPlan) -> Self {
        Self::with_bg(plan, None)
    }

    /// Like `new`, but a `BlurScreen` background can blur the given representative
    /// source `bg_frame` into the backdrop (else BlurScreen falls back to a gradient).
    pub fn with_bg(plan: &'a EditPlan, bg_frame: Option<&Pixmap>) -> Self {
        let (out_w, out_h) = output_size(plan);

        // Reframe (9:16 / 1:1) uses COVER fit: the source fills the output and is
        // cropped around the zoom focus (the "action"), full-bleed, no frame. The
        // default (no reframe) uses CONTAIN fit: a padded, rounded, shadowed card.
        let cover = matches!(plan.reframe, record_core::Reframe::Aspect { .. });
        let (card, radius) = if cover {
            let sb = (out_w as f32 / plan.source_w as f32).max(out_h as f32 / plan.source_h as f32);
            (
                CardRect {
                    x: 0.0,
                    y: 0.0,
                    w: out_w as f32,
                    h: out_h as f32,
                    scale_base: sb,
                },
                0.0,
            )
        } else {
            let pad = if plan.frame.enabled {
                plan.frame.padding
            } else {
                0.0
            };
            let c = card_rect(out_w, out_h, plan.source_w, plan.source_h, pad);
            let r = if plan.frame.enabled {
                plan.frame.corner_radius as f32
            } else {
                0.0
            };
            (c, r)
        };

        let mut base = Pixmap::new(out_w, out_h).expect("allocate base pixmap");
        match (&plan.background, bg_frame) {
            (record_core::Background::BlurScreen { .. }, Some(frame)) => {
                crate::fill_blur_bg(&mut base, frame)
            }
            _ => fill_background(&mut base, &plan.background),
        }
        if plan.frame.enabled && !cover {
            if let Some(sh) = &plan.frame.shadow {
                draw_card_shadow(&mut base, &card, radius, sh);
            }
        }
        let mask = if radius > 0.5 {
            output_mask_for(out_w, out_h, &card, radius)
        } else {
            None
        };

        let caption_lines = match &plan.captions {
            Some(c) => crate::captions::load_lines(&c.words_path, 36),
            None => Vec::new(),
        };

        Self {
            plan,
            out_w,
            out_h,
            card,
            base,
            mask,
            caption_lines,
        }
    }

    /// Compose a frame that also overlays a webcam bubble (a square `webcam` frame
    /// drawn into the circular bubble). Falls back to `frame` when there is no
    /// webcam geometry or no webcam frame for this instant.
    pub fn frame_webcam(&self, source: &Pixmap, webcam: Option<&Pixmap>, t_ms: u64) -> Pixmap {
        let mut out = self.frame(source, t_ms);
        if let (Some(wc), Some(cam)) = (&self.plan.webcam, webcam) {
            let placement = wc.placement_at(t_ms, self.out_w, self.out_h);
            if !placement.visible {
                return out;
            }
            let d = (placement.size as f32 * self.out_h as f32).round().max(2.0);
            let x = (placement.x as f32 * self.out_w as f32)
                .clamp(0.0, (self.out_w as f32 - d).max(0.0));
            let y = (placement.y as f32 * self.out_h as f32)
                .clamp(0.0, (self.out_h as f32 - d).max(0.0));
            let Some(mask) = webcam_mask(self.out_w, self.out_h, x, y, d, placement.shape) else {
                return out;
            };
            let g = WebcamGeom { x, y, d, mask };
            let sx = g.d / cam.width() as f32;
            let paint = PixmapPaint {
                opacity: 1.0,
                blend_mode: BlendMode::SourceOver,
                quality: FilterQuality::Bilinear,
            };
            out.draw_pixmap(
                0,
                0,
                cam.as_ref(),
                &paint,
                Transform::from_row(sx, 0.0, 0.0, sx, g.x, g.y),
                Some(&g.mask),
            );
            // White ring around the bubble.
            if let Some(path) = webcam_outline(g.x, g.y, g.d, placement.shape) {
                let mut p = Paint::default();
                p.set_color(Color::from_rgba8(255, 255, 255, 230));
                p.anti_alias = true;
                let stroke = Stroke {
                    width: (g.d * 0.02).max(2.0),
                    ..Default::default()
                };
                out.stroke_path(&path, &p, &stroke, Transform::identity(), None);
            }
        }
        out
    }

    /// Compose one polished output frame from a decoded `source` frame at `t_ms`.
    pub fn frame(&self, source: &Pixmap, t_ms: u64) -> Pixmap {
        let plan = self.plan;
        let card = self.card;
        let mut out = self.base.clone();

        // Eased zoom → transform mapping source px → output px (incl. card offset).
        let (z, cx, cy) = plan.zoom.eval(t_ms);
        let a = card.scale_base * z as f32;
        let tx = card.w / 2.0 - a * (cx as f32) * plan.source_w as f32;
        let ty = card.h / 2.0 - a * (cy as f32) * plan.source_h as f32;

        let paint = PixmapPaint {
            opacity: 1.0,
            blend_mode: BlendMode::SourceOver,
            quality: FilterQuality::Bilinear,
        };
        out.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &paint,
            Transform::from_row(a, 0.0, 0.0, a, card.x + tx, card.y + ty),
            self.mask.as_ref(),
        );

        // Map a source pixel → output coords through the same zoom transform.
        let map = |sx: f64, sy: f64| -> (f32, f32, bool) {
            let scx = a * sx as f32 + tx;
            let scy = a * sy as f32 + ty;
            let inside = scx >= 0.0 && scx <= card.w && scy >= 0.0 && scy <= card.h;
            (card.x + scx, card.y + scy, inside)
        };

        // Click ripples (under the cursor).
        for fx in &plan.clicks {
            if t_ms >= fx.t_ms && t_ms < fx.t_ms + RIPPLE_MS {
                let p = (t_ms - fx.t_ms) as f32 / RIPPLE_MS as f32;
                let (ox, oy, inside) =
                    map(fx.x * plan.source_w as f64, fx.y * plan.source_h as f64);
                if inside {
                    draw_ripple(
                        &mut out,
                        ox,
                        oy,
                        p,
                        (card.scale_base * 46.0 * z as f32).max(8.0),
                    );
                }
            }
        }

        // Synthetic cursor — constant on-screen size (scales with output
        // resolution, NOT with zoom).
        if let Some((sx, sy)) = cursor_sample_at(&plan.cursor.smoothed, t_ms) {
            let (ox, oy, inside) = map(sx, sy);
            if inside {
                let cur_scale = (plan.cursor.scale as f32) * (self.out_h as f32 / 1080.0).max(0.5);
                draw_cursor(&mut out, ox, oy, cur_scale);
            }
        }

        // Captions — the active transcript line (above the key-cast row).
        if let Some(line) = crate::captions::active(&self.caption_lines, t_ms) {
            let px = (self.out_h as f32 * 0.044).max(16.0);
            let (fg, bg) = plan
                .captions
                .as_ref()
                .map(|c| (c.color, c.box_color))
                .unwrap_or((
                    record_core::Rgba::WHITE,
                    record_core::Rgba::new(0, 0, 0, 150),
                ));
            crate::text::draw_bottom_chip(&mut out, line, px, self.out_h as f32 * 0.16, fg, bg);
        }

        // Key-cast chip — the latest currently-active typed-keys chip (bottom center).
        if let Some(kc) = plan
            .keycast
            .iter()
            .rev()
            .find(|k| t_ms >= k.t_ms && t_ms < k.t_ms + k.hold_ms)
        {
            let px = (self.out_h as f32 * 0.038).max(15.0);
            crate::text::draw_bottom_chip(
                &mut out,
                &kc.text,
                px,
                self.out_h as f32 * 0.08,
                record_core::Rgba::WHITE,
                record_core::Rgba::new(18, 20, 28, 215),
            );
        }

        out
    }
}

fn webcam_mask(
    out_w: u32,
    out_h: u32,
    x: f32,
    y: f32,
    d: f32,
    shape: record_core::WebcamShape,
) -> Option<Mask> {
    match shape {
        record_core::WebcamShape::Circle => {
            circle_mask(out_w, out_h, x + d / 2.0, y + d / 2.0, d / 2.0)
        }
        record_core::WebcamShape::RoundedRect { radius } => {
            let mut tmp = Pixmap::new(out_w, out_h)?;
            let path = rounded_rect_path(x, y, d, d, radius as f32)?;
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
    }
}

fn webcam_outline(
    x: f32,
    y: f32,
    d: f32,
    shape: record_core::WebcamShape,
) -> Option<tiny_skia::Path> {
    match shape {
        record_core::WebcamShape::Circle => {
            let mut pb = PathBuilder::new();
            pb.push_circle(x + d / 2.0, y + d / 2.0, d / 2.0);
            pb.finish()
        }
        record_core::WebcamShape::RoundedRect { radius } => {
            rounded_rect_path(x, y, d, d, radius as f32)
        }
    }
}

/// Compose a SINGLE frame (builds a transient `Compositor`) — for one-offs/tests.
pub fn compose_frame(source: &Pixmap, plan: &EditPlan, t_ms: u64) -> Pixmap {
    Compositor::new(plan).frame(source, t_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_core::{
        Anchor, Ease, EditPlan, WebcamKeyframe, WebcamOverlay, WebcamShape, ZoomKey,
    };
    use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

    /// A 4-colored-quadrant source so we can see compositing is non-trivial.
    fn quad_source(w: u32, h: u32) -> Pixmap {
        let mut pm = Pixmap::new(w, h).unwrap();
        let colors = [
            Color::from_rgba8(220, 60, 60, 255),
            Color::from_rgba8(60, 220, 60, 255),
            Color::from_rgba8(60, 60, 220, 255),
            Color::from_rgba8(220, 220, 60, 255),
        ];
        let hw = w as f32 / 2.0;
        let hh = h as f32 / 2.0;
        for (i, (x, y)) in [(0.0, 0.0), (hw, 0.0), (0.0, hh), (hw, hh)]
            .iter()
            .enumerate()
        {
            let mut p = Paint::default();
            p.set_color(colors[i]);
            pm.fill_rect(
                Rect::from_xywh(*x, *y, hw, hh).unwrap(),
                &p,
                Transform::identity(),
                None,
            );
        }
        pm
    }

    fn channel_spread(pm: &Pixmap) -> u8 {
        let data = pm.data();
        let (mut min, mut max) = (255u8, 0u8);
        for b in data.iter().step_by(4) {
            min = min.min(*b);
            max = max.max(*b);
        }
        max - min
    }

    fn solid_source(w: u32, h: u32, color: Color) -> Pixmap {
        let mut pm = Pixmap::new(w, h).unwrap();
        let mut p = Paint::default();
        p.set_color(color);
        pm.fill_rect(
            Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
            &p,
            Transform::identity(),
            None,
        );
        pm
    }

    fn pixel_rgba(pm: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * pm.width() + x) * 4) as usize;
        [
            pm.data()[i],
            pm.data()[i + 1],
            pm.data()[i + 2],
            pm.data()[i + 3],
        ]
    }

    fn is_red(px: [u8; 4]) -> bool {
        px[0] > 180 && px[1] < 90 && px[2] < 90 && px[3] > 200
    }

    #[test]
    fn compose_outputs_correct_size_and_nonblank() {
        let src = quad_source(320, 180);
        let mut plan = EditPlan::empty(320, 180, 2000, 30.0);
        plan.zoom.keys.push(ZoomKey {
            t_ms: 0,
            scale: 1.4,
            cx: 0.5,
            cy: 0.5,
            ease: Ease::EaseInOut,
        });
        let out = compose_frame(&src, &plan, 0);
        assert_eq!((out.width(), out.height()), output_size(&plan));
        assert!(channel_spread(&out) > 30, "output looks blank");
    }

    #[test]
    fn compose_is_deterministic() {
        let src = quad_source(160, 90);
        let plan = EditPlan::empty(160, 90, 1000, 30.0);
        let a = compose_frame(&src, &plan, 500);
        let b = compose_frame(&src, &plan, 500);
        assert_eq!(a.data(), b.data(), "same inputs must give identical pixels");
    }

    #[test]
    fn compositor_reuse_matches_oneshot() {
        let src = quad_source(200, 120);
        let mut plan = EditPlan::empty(200, 120, 3000, 30.0);
        plan.zoom.keys.push(ZoomKey {
            t_ms: 0,
            scale: 1.0,
            cx: 0.5,
            cy: 0.5,
            ease: Ease::EaseInOut,
        });
        plan.zoom.keys.push(ZoomKey {
            t_ms: 1500,
            scale: 2.0,
            cx: 0.3,
            cy: 0.3,
            ease: Ease::EaseInOut,
        });
        let comp = Compositor::new(&plan);
        for t in [0u64, 750, 1500, 2999] {
            let reused = comp.frame(&src, t);
            let oneshot = compose_frame(&src, &plan, t);
            assert_eq!(
                reused.data(),
                oneshot.data(),
                "cached compositor must match one-shot at t={t}"
            );
        }
    }

    #[test]
    fn reframe_changes_output_aspect() {
        let src = quad_source(320, 180);
        let mut plan = EditPlan::empty(320, 180, 1000, 30.0);
        plan.reframe = record_core::Reframe::Aspect { w: 9, h: 16 };
        let (ow, oh) = output_size(&plan);
        assert!(ow < oh, "9:16 should be portrait, got {ow}x{oh}");
        let out = compose_frame(&src, &plan, 0);
        assert_eq!((out.width(), out.height()), (ow, oh));
    }

    #[test]
    fn webcam_timeline_moves_overlay_for_frame_time() {
        let src = solid_source(320, 180, Color::from_rgba8(40, 80, 180, 255));
        let cam = solid_source(64, 64, Color::from_rgba8(240, 20, 20, 255));
        let mut plan = EditPlan::empty(320, 180, 3000, 30.0);
        plan.webcam = Some(WebcamOverlay {
            source: "cam.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.20,
            timeline: vec![WebcamKeyframe {
                t_ms: 1000,
                visible: Some(true),
                x: Some(0.10),
                y: Some(0.20),
                size: Some(0.30),
                shape: Some(WebcamShape::Circle),
            }],
        });

        let comp = Compositor::new(&plan);
        let before = comp.frame_webcam(&src, Some(&cam), 0);
        assert!(
            !is_red(pixel_rgba(&before, 59, 63)),
            "camera should not start at the moved position"
        );

        let moved = comp.frame_webcam(&src, Some(&cam), 1500);
        assert!(
            is_red(pixel_rgba(&moved, 59, 63)),
            "camera should be drawn at the moved Studio position"
        );
    }

    #[test]
    fn webcam_timeline_visibility_hides_overlay() {
        let src = solid_source(320, 180, Color::from_rgba8(40, 80, 180, 255));
        let cam = solid_source(64, 64, Color::from_rgba8(240, 20, 20, 255));
        let mut plan = EditPlan::empty(320, 180, 3000, 30.0);
        plan.webcam = Some(WebcamOverlay {
            source: "cam.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.30,
            timeline: vec![
                WebcamKeyframe {
                    t_ms: 0,
                    visible: Some(true),
                    x: Some(0.10),
                    y: Some(0.20),
                    size: Some(0.30),
                    shape: None,
                },
                WebcamKeyframe {
                    t_ms: 1000,
                    visible: Some(false),
                    x: None,
                    y: None,
                    size: None,
                    shape: None,
                },
            ],
        });

        let comp = Compositor::new(&plan);
        let visible = comp.frame_webcam(&src, Some(&cam), 500);
        assert!(is_red(pixel_rgba(&visible, 59, 63)));

        let hidden = comp.frame_webcam(&src, Some(&cam), 1500);
        assert!(
            !is_red(pixel_rgba(&hidden, 59, 63)),
            "hidden camera event should remove the overlay from polished frames"
        );
    }
}
