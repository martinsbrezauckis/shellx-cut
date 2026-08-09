//! source.rs — synthetic "fake desktop" source video generator.
//!
//! Live screen capture can't run headless (WSL/CI), so to prove the render
//! pipeline end-to-end we synthesize a desktop-like source whose layout has a
//! distinct UI card at every click anchor — so auto-zoom has something real to
//! zoom INTO. This is a TEST/DEMO aid; the production source comes from the
//! capture layer. The same tiny-skia primitives feed the polish compositor.

use record_core::{EventTrack, Result};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::{ffmpeg, rounded_rect_path};

/// Draw a static fake-desktop frame sized to the EventTrack's screen.
pub fn draw_desktop(events: &EventTrack) -> Pixmap {
    let w = events.screen_w.max(2);
    let h = events.screen_h.max(2);
    let wf = w as f32;
    let hf = h as f32;
    let mut pm = Pixmap::new(w, h).expect("desktop pixmap");
    pm.fill(Color::from_rgba8(28, 30, 38, 255));

    // Top window-chrome bar.
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, wf, hf * 0.06) {
        let mut p = Paint::default();
        p.set_color(Color::from_rgba8(44, 47, 58, 255));
        pm.fill_rect(rect, &p, Transform::identity(), None);
    }
    // Traffic-light dots.
    for (i, c) in [(255u8, 95u8, 86u8), (255, 189, 46), (39, 201, 63)]
        .iter()
        .enumerate()
    {
        let mut pb = PathBuilder::new();
        pb.push_circle(hf * 0.03 + i as f32 * hf * 0.028, hf * 0.03, hf * 0.009);
        if let Some(path) = pb.finish() {
            let mut p = Paint::default();
            p.set_color(Color::from_rgba8(c.0, c.1, c.2, 255));
            p.anti_alias = true;
            pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
    }
    // Faint grid so zoom/pan is perceptible. 1px lines MUST be non-AA: tiny-skia
    // routes a full-width anti-aliased 1px fill_rect through its hairline scan
    // converter, which panics (hairline_aa assertion). Sharp lines are correct here.
    let mut gp = Paint {
        anti_alias: false,
        ..Default::default()
    };
    gp.set_color(Color::from_rgba8(255, 255, 255, 10));
    let step = hf * 0.08;
    let mut y = hf * 0.06;
    while y < hf {
        if let Some(r) = Rect::from_xywh(0.0, y, wf, 1.0) {
            pm.fill_rect(r, &gp, Transform::identity(), None);
        }
        y += step;
    }
    let mut x = 0.0;
    while x < wf {
        if let Some(r) = Rect::from_xywh(x, hf * 0.06, 1.0, hf) {
            pm.fill_rect(r, &gp, Transform::identity(), None);
        }
        x += step;
    }

    // A distinct card at each click anchor.
    let accents = [
        (99u8, 179u8, 237u8),
        (246, 173, 85),
        (159, 122, 234),
        (72, 187, 120),
        (237, 100, 166),
        (56, 178, 172),
    ];
    for (i, c) in events.click_downs().enumerate() {
        let cw = wf * 0.18;
        let ch = hf * 0.16;
        let cxp = (c.x as f32 - cw / 2.0).clamp(0.0, wf - cw);
        let cyp = (c.y as f32 - ch / 2.0).clamp(hf * 0.06, hf - ch);
        if let Some(path) = rounded_rect_path(cxp, cyp, cw, ch, hf * 0.02) {
            let mut p = Paint::default();
            p.set_color(Color::from_rgba8(54, 57, 70, 255));
            p.anti_alias = true;
            pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
        let ac = accents[i % accents.len()];
        if let Some(path) = rounded_rect_path(
            cxp + cw * 0.08,
            cyp + ch * 0.18,
            cw * 0.84,
            ch * 0.16,
            hf * 0.008,
        ) {
            let mut p = Paint::default();
            p.set_color(Color::from_rgba8(ac.0, ac.1, ac.2, 255));
            p.anti_alias = true;
            pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
        for k in 0..2 {
            if let Some(path) = rounded_rect_path(
                cxp + cw * 0.08,
                cyp + ch * 0.5 + k as f32 * ch * 0.18,
                cw * 0.7,
                ch * 0.1,
                hf * 0.006,
            ) {
                let mut p = Paint::default();
                p.set_color(Color::from_rgba8(120, 126, 140, 255));
                p.anti_alias = true;
                pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
            }
        }
    }
    pm
}

/// Encode a static synthetic desktop as an MP4 of the right length (the source
/// the render pipeline then decodes + polishes).
pub fn generate_source(events: &EventTrack, out_path: &str, fps: f64) -> Result<u64> {
    let desktop = draw_desktop(events);
    let w = desktop.width();
    let h = desktop.height();
    let nframes = ((events.duration_ms as f64) * fps / 1000.0).round() as u64;
    let frame_bytes = desktop.data().to_vec();
    ffmpeg::encode_frames(out_path, w, h, fps, nframes.max(1), |_idx| {
        frame_bytes.clone()
    })
}
