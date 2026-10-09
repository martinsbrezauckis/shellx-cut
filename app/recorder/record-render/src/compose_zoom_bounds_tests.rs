//! Regression pixels for generated off-center one-times zoom keys.
use super::*;
use record_core::{Background, ClickFx, CursorSample, Ease, Rgba, ZoomKey};

fn plan(frame: bool) -> EditPlan {
    let mut p = EditPlan::empty(320, 180, 7791, 24.0);
    p.frame.enabled = frame;
    p.frame.corner_radius = 0.0;
    p.frame.shadow = None;
    p.background = Background::Solid {
        color: Rgba {
            r: 0,
            g: 255,
            b: 0,
            a: 255,
        },
    };
    // Same times/foci as the actual native automatic two-click plan.
    p.zoom.keys = [
        (0, 1.0, 0.5, 0.5),
        (742, 1.0, 0.25, 380.0 / 1144.0),
        (1292, 2.0, 0.25, 380.0 / 1144.0),
        (1792, 2.0, 0.25, 380.0 / 1144.0),
        (3955, 2.0, 0.75, 640.0 / 1144.0),
        (4455, 2.0, 0.75, 640.0 / 1144.0),
        (4955, 1.0, 0.75, 640.0 / 1144.0),
        (7791, 1.0, 0.5, 0.5),
    ]
    .into_iter()
    .map(|(t_ms, scale, cx, cy)| ZoomKey {
        t_ms,
        scale,
        cx,
        cy,
        ease: Ease::EaseInOut,
    })
    .collect();
    p
}
fn landmarks() -> Pixmap {
    let mut s = Pixmap::new(320, 180).unwrap();
    // Every edge and corner has independently identifiable source pixels.
    for y in 0..180 {
        for x in 0..320 {
            let i = ((y * 320 + x) * 4) as usize;
            s.data_mut()[i..i + 4].copy_from_slice(&[
                (40 + x % 200) as u8,
                (60 + y % 170) as u8,
                30,
                255,
            ]);
        }
    }
    s
}
fn pixel(p: &Pixmap, x: u32, y: u32) -> &[u8] {
    let i = ((y * p.width() + x) * 4) as usize;
    &p.data()[i..i + 4]
}
#[test]
fn one_x_preposition_and_return_preserve_every_edge_and_corner() {
    let source = landmarks();
    for framed in [true, false] {
        let p = plan(framed);
        let mut neutral = p.clone();
        neutral.zoom.keys.clear();
        let expected = Compositor::new(&neutral).unwrap().frame(&source, 0);
        let c = Compositor::new(&p).unwrap();
        for t in [0, 500, 742, 4955, 6500, 7791] {
            assert_eq!(p.zoom.eval(t).0, 1.0);
            assert!(
                c.frame(&source, t).data() == expected.data(),
                "one-x frame shifted/cropped at {t}ms framed={framed}"
            );
        }
    }
}
#[test]
fn zoom_ramps_never_expose_background_inside_the_screen_card() {
    let mut source = Pixmap::new(320, 180).unwrap();
    source.fill(Color::from_rgba8(210, 70, 60, 255));
    for framed in [true, false] {
        let p = plan(framed);
        let c = Compositor::new(&p).unwrap();
        let card = c.card;
        for t in [
            0, 100, 500, 742, 850, 1000, 1292, 2000, 3500, 3955, 4455, 4600, 4800, 4955, 6000, 7791,
        ] {
            let out = c.frame(&source, t);
            for y in (card.y as u32 + 2)..((card.y + card.h) as u32 - 2) {
                for x in (card.x as u32 + 2)..((card.x + card.w) as u32 - 2) {
                    let px = pixel(&out, x, y);
                    assert!(
                        px[0] > 180 && px[1] < 110,
                        "exposed card at {t}ms ({x},{y}) framed={framed}: {px:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn ripple_and_cursor_follow_the_same_corrected_source_transform() {
    let mut source = Pixmap::new(320, 180).unwrap();
    source.fill(Color::from_rgba8(15, 15, 20, 255));
    let mut p = plan(false);
    p.zoom.keys = vec![ZoomKey {
        t_ms: 0,
        scale: 1.0,
        cx: 0.25,
        cy: 0.33,
        ease: Ease::Linear,
    }];
    p.clicks = vec![ClickFx {
        t_ms: 0,
        x: 0.25,
        y: 1.0 / 3.0,
    }];
    p.cursor.smoothed = vec![CursorSample {
        t_ms: 0,
        x: 80.0,
        y: 60.0,
    }];
    p.cursor.scale = 6.0;
    let mut neutral = p.clone();
    neutral.zoom.keys.clear();
    let actual = Compositor::new(&p).unwrap().frame(&source, 100);
    let expected = Compositor::new(&neutral).unwrap().frame(&source, 100);
    assert!(
        actual.data() == expected.data(),
        "pixels, pointer and ring must share the neutral transform at one-x"
    );
    assert!(
        pixel(&actual, 84, 72)[0] > 200,
        "cursor interior must be at actual source point"
    );
    let blue = (35..86)
        .flat_map(|y| (50..110).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = pixel(&actual, x, y);
            u16::from(p[2]) > u16::from(p[0]) + 20 && p[2] > p[1]
        })
        .count();
    assert!(blue > 50, "actual ring missing at corrected point");
}
#[test]
fn explicit_portrait_cover_keeps_its_deliberate_focus_crop() {
    let source = landmarks();
    let mut p = plan(false);
    p.reframe = record_core::Reframe::Aspect { w: 9, h: 16 };
    p.zoom.keys = vec![ZoomKey {
        t_ms: 0,
        scale: 1.0,
        cx: 0.25,
        cy: 0.33,
        ease: Ease::Linear,
    }];
    let c = Compositor::new(&p).unwrap();
    let actual = c.frame(&source, 0);
    // Independent legacy COVER reference: required crop around requested focus.
    let mut expected = c.base.clone();
    let a = c.card.scale_base;
    expected.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            opacity: 1.0,
            blend_mode: BlendMode::SourceOver,
            quality: FilterQuality::Bilinear,
        },
        Transform::from_row(
            a,
            0.0,
            0.0,
            a,
            c.card.w / 2.0 - a * 0.25 * 320.0,
            c.card.h / 2.0 - a * 0.33 * 180.0,
        ),
        None,
    );
    assert_eq!(actual.data(), expected.data());
}

#[test]
fn peak_two_x_pixels_ring_and_cursor_keep_existing_mapping() {
    let source = landmarks();
    let mut p = plan(false);
    p.zoom.keys = vec![ZoomKey {
        t_ms: 0,
        scale: 2.0,
        cx: 0.25,
        cy: 0.5,
        ease: Ease::Linear,
    }];
    p.clicks = vec![ClickFx {
        t_ms: 0,
        x: 0.25,
        y: 0.5,
    }];
    p.cursor.smoothed = vec![CursorSample {
        t_ms: 0,
        x: 80.0,
        y: 90.0,
    }];
    p.cursor.scale = 6.0;
    let c = Compositor::new(&p).unwrap();
    let actual = c.frame(&source, 100);
    let mut expected = c.base.clone();
    expected.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            opacity: 1.0,
            blend_mode: BlendMode::SourceOver,
            quality: FilterQuality::Bilinear,
        },
        Transform::from_row(2.0, 0.0, 0.0, 2.0, 0.0, -90.0),
        None,
    );
    crate::draw_ripple(
        &mut expected,
        160.0,
        90.0,
        100.0 / crate::RIPPLE_MS as f32,
        92.0,
    );
    crate::draw_cursor(&mut expected, 160.0, 90.0, 3.0);
    assert!(
        actual.data() == expected.data(),
        "peak pixels and both overlays changed"
    );
}
