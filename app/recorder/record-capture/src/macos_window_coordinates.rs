//! Point-to-pixel correlation for a single SCK window. Native cursor pixels
//! stay in the video. Two agreeing, fresh frames must bracket each button.
use record_core::{ClickPositionQuality, ClickSample};

pub(crate) const MAX_AGE_MS: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn valid(self) -> bool {
        [self.x, self.y, self.w, self.h]
            .into_iter()
            .all(f64::is_finite)
            && self.w > 0.0
            && self.h > 0.0
    }
    fn agrees(self, other: Self) -> bool {
        [
            self.x - other.x,
            self.y - other.y,
            self.w - other.w,
            self.h - other.h,
        ]
        .into_iter()
        .all(|v| v.abs() < 0.01)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub t_ms: u64,
    pub window: Rect,
    pub content: Rect,
    pub scale: f64,
    pub output: (u32, u32),
}

impl Frame {
    /// Apple defines contentScale as backing-surface scaling, scaleFactor as
    /// points-to-backing pixels. SDK SCStream.h defines contentRect and
    /// boundingRect in surface points, while screenRect is the onscreen content.
    /// For this exact desktop-independent single-window filter, require the
    /// smallest captured-window surface bounding box to agree with contentRect.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn verified(
        t_ms: u64,
        live: Rect,
        screen: Rect,
        bounding: Rect,
        content: Rect,
        scale_factor: f64,
        content_scale: f64,
        output: (u32, u32),
    ) -> Option<Self> {
        let scale = scale_factor * content_scale;
        if !live.valid()
            || !screen.valid()
            || !bounding.valid()
            || !content.valid()
            || !live.agrees(screen)
            || !bounding.agrees(content)
            || !scale_factor.is_finite()
            || scale_factor <= 0.0
            || !content_scale.is_finite()
            || content_scale <= 0.0
            || !scale.is_finite()
            || scale <= 0.0
        {
            return None;
        }
        // Normalize surface point placement exactly once. contentScale affects
        // source-to-content sizing, not this surface-point-to-pixel conversion.
        let content = Rect {
            x: content.x * scale_factor,
            y: content.y * scale_factor,
            w: content.w * scale_factor,
            h: content.h * scale_factor,
        };
        if !content.valid()
            || (live.w * scale - content.w).abs() > 1.0
            || (live.h * scale - content.h).abs() > 1.0
            || content.x < 0.0
            || content.y < 0.0
            || content.x + content.w > f64::from(output.0) + 0.01
            || content.y + content.h > f64::from(output.1) + 0.01
        {
            return None;
        }
        Some(Self {
            t_ms,
            window: live,
            content,
            scale,
            output,
        })
    }
    pub(crate) fn same_mapping(self, other: Self) -> bool {
        self.window.agrees(other.window)
            && self.content.agrees(other.content)
            && (self.scale - other.scale).abs() < 0.0001
            && self.output == other.output
    }
}

pub(crate) fn map_click(
    click: &mut ClickSample,
    age_ms: u32,
    frames: &[Option<Frame>],
    output: (u32, u32),
) -> &'static str {
    click.position_quality = ClickPositionQuality::Unavailable;
    if u64::from(age_ms) > MAX_AGE_MS || !click.x.is_finite() || !click.y.is_finite() {
        return "native button point missing, invalid or stale";
    }
    // Missing/invalid frame observations remain barriers, never skipped to
    // join old geometry across a move, resize, suspension or source replacement.
    let after_index = frames
        .iter()
        .position(|f| f.is_some_and(|f| f.t_ms >= click.t_ms));
    let Some(index) = after_index else {
        return "no fresh following frame or Idle observation";
    };
    let Some(before) = index.checked_sub(1).and_then(|i| frames[i]) else {
        return "no fresh preceding pixel-attested observation";
    };
    let Some(after) = frames[index] else {
        return "following frame metadata unavailable";
    };
    if before.t_ms > click.t_ms
        || click.t_ms - before.t_ms > MAX_AGE_MS
        || after.t_ms - click.t_ms > MAX_AGE_MS
        || !before.same_mapping(after)
        || before.output != output
    {
        return "stale, moved, resized or output-mismatched frame bracket";
    }
    let local = (click.x - before.window.x, click.y - before.window.y);
    if local.0 < 0.0 || local.1 < 0.0 || local.0 >= before.window.w || local.1 >= before.window.h {
        return "native button point outside selected content";
    }
    // Both CGEvent and screenRect use desktop coordinates with a top-left
    // origin; no AppKit bottom-left flip is introduced here.
    let x = before.content.x + local.0 * before.scale;
    let y = before.content.y + local.1 * before.scale;
    if x < f64::from(output.0) && y < f64::from(output.1) {
        click.x = x;
        click.y = y;
        click.position_quality = ClickPositionQuality::Exact;
        return "exact";
    }
    "native button point outside encoded output"
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_core::MouseButton;
    fn frame(t_ms: u64, x: f64, scale: f64) -> Frame {
        let live = Rect {
            x,
            y: -200.0,
            w: 400.0,
            h: 300.0,
        };
        let surface_content = Rect {
            x: 5.0,
            y: 10.0,
            w: 400.0 * scale / 2.0,
            h: 300.0 * scale / 2.0,
        };
        Frame::verified(
            t_ms,
            live,
            live,
            surface_content,
            surface_content,
            2.0,
            scale / 2.0,
            (1000, 800),
        )
        .unwrap()
    }
    fn click() -> ClickSample {
        ClickSample {
            t_ms: 50,
            x: -150.0,
            y: -125.0,
            button: MouseButton::Left,
            down: true,
            position_quality: ClickPositionQuality::Unavailable,
        }
    }
    #[test]
    fn negative_origin_retina_padding_and_top_left_axes() {
        let mut click = click();
        map_click(
            &mut click,
            3,
            &[Some(frame(20, -250.0, 2.0)), Some(frame(80, -250.0, 2.0))],
            (1000, 800),
        );
        assert_eq!((click.x, click.y), (210.0, 170.0));
        assert_eq!(click.position_quality, ClickPositionQuality::Exact);
    }
    #[test]
    fn scaled_resize_maps_only_after_two_agreeing_frames() {
        let mut click = click();
        map_click(
            &mut click,
            0,
            &[Some(frame(20, -250.0, 1.25)), Some(frame(80, -250.0, 1.25))],
            (1000, 800),
        );
        assert_eq!((click.x, click.y), (135.0, 113.75));
        assert_eq!(click.position_quality, ClickPositionQuality::Exact);
    }
    #[test]
    fn move_resize_missing_stale_and_replacement_intervals_refuse() {
        for frames in [
            vec![Some(frame(20, -250.0, 2.0)), Some(frame(80, -240.0, 2.0))],
            vec![Some(frame(20, -250.0, 2.0)), Some(frame(80, -250.0, 1.0))],
            vec![
                Some(frame(20, -250.0, 2.0)),
                None,
                Some(frame(80, -250.0, 2.0)),
            ],
            vec![Some(frame(20, -250.0, 2.0)), Some(frame(151, -250.0, 2.0))],
            vec![],
        ] {
            let mut c = click();
            map_click(&mut c, 0, &frames, (1000, 800));
            assert_eq!(c.position_quality, ClickPositionQuality::Unavailable);
        }
    }
    #[test]
    fn delayed_events_foreign_points_and_final_output_mismatch_refuse() {
        for (age, x, output) in [
            (101, -150.0, (1000, 800)),
            (0, 150.0, (1000, 800)),
            (0, -150.0, (998, 800)),
        ] {
            let mut c = click();
            c.x = x;
            map_click(
                &mut c,
                age,
                &[Some(frame(20, -250.0, 2.0)), Some(frame(80, -250.0, 2.0))],
                output,
            );
            assert_eq!(c.position_quality, ClickPositionQuality::Unavailable);
        }
    }

    #[test]
    fn attachment_domain_scale_crop_and_nonfinite_metadata_refuse() {
        let live = Rect {
            x: -250.0,
            y: -200.0,
            w: 400.0,
            h: 300.0,
        };
        let content = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        assert!(Frame::verified(20, live, live, content, content, 2.0, 1.0, (1000, 800)).is_some());
        for (screen, bounding, content, scale) in [
            (Rect { y: 200.0, ..live }, content, content, 2.0),
            // Desktop origin must never be mistaken for a surface origin.
            (live, live, content, 2.0),
            // Pixel-domain attachment values must not be accepted as points.
            (
                live,
                Rect {
                    w: 800.0,
                    h: 600.0,
                    ..content
                },
                Rect {
                    w: 800.0,
                    h: 600.0,
                    ..content
                },
                2.0,
            ),
            (
                live,
                Rect {
                    x: 300.0,
                    ..content
                },
                Rect {
                    x: 300.0,
                    ..content
                },
                2.0,
            ),
            (live, content, content, f64::NAN),
        ] {
            assert!(
                Frame::verified(20, live, screen, bounding, content, scale, 1.0, (1000, 800))
                    .is_none()
            );
        }
    }

    #[test]
    fn nonretina_surface_points_and_scaled_retina_surface_points_share_pixels() {
        let live = Rect {
            x: 250.0,
            y: 300.0,
            w: 400.0,
            h: 300.0,
        };
        let content = Rect {
            x: 5.0,
            y: 10.0,
            w: 200.0,
            h: 150.0,
        };
        let frame =
            Frame::verified(20, live, live, content, content, 2.0, 0.5, (600, 400)).unwrap();
        assert_eq!(
            frame.content,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 400.0,
                h: 300.0
            }
        );
        let content = Rect {
            x: 10.0,
            y: 20.0,
            w: 400.0,
            h: 300.0,
        };
        let one_x =
            Frame::verified(20, live, live, content, content, 1.0, 1.0, (600, 400)).unwrap();
        assert_eq!(one_x.content, frame.content);
        assert_eq!(one_x.scale, frame.scale);
    }
}
