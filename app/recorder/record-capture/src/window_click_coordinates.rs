//! Window click coordinates use actual button payloads and the accepted full-window frame fit.
//! Native cursor pixels stay in the video; this module never creates a cursor track.

use record_core::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState,
    CursorCorrelation, MouseButton,
};

use crate::window_frame_fit::WindowFrameFit;

pub(crate) const MAX_EVENT_AGE_MS: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowRect {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

struct PendingClick {
    sample: ClickSample,
    local: Option<(f64, f64)>,
    content: Option<(u32, u32)>,
    age_ms: u32,
    refusal: Option<&'static str>,
}

pub(crate) struct WindowClicks {
    output: (u32, u32),
    content: Option<WindowFrameFit>,
    closed: bool,
    clicks: Vec<PendingClick>,
    pending: Vec<usize>,
}

impl WindowClicks {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self {
            output: (width, height),
            content: None,
            closed: false,
            clicks: vec![],
            pending: vec![],
        }
    }

    /// Called only after the encoder accepts a native frame whose visible bounds
    /// agree with its content dimensions. A static accepted frame can be held:
    /// moving a window changes its desktop origin, not its content pixel origin.
    #[cfg(test)]
    pub(crate) fn frame(&mut self, t_ms: u64, content: Option<(u32, u32)>) {
        self.accepted_fit(
            t_ms,
            content.and_then(|size| WindowFrameFit::new(size, self.output)),
        );
    }

    pub(crate) fn accepted_fit(&mut self, t_ms: u64, fit: Option<WindowFrameFit>) {
        self.content = if self.closed {
            None
        } else {
            fit.filter(|fit| fit.output_size() == self.output)
        };
        let output = self.output;
        let pending = std::mem::take(&mut self.pending);
        for index in pending {
            let click = &mut self.clicks[index];
            if t_ms >= click.sample.t_ms && t_ms - click.sample.t_ms <= u64::from(MAX_EVENT_AGE_MS)
            {
                admit(click, self.content, output);
                if click.sample.position_quality == ClickPositionQuality::Unavailable {
                    self.pending.push(index);
                }
            }
        }
    }

    pub(crate) fn close(&mut self) {
        self.pending.clear();
        self.closed = true;
        self.content = None;
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(crate) fn queue_click(
        &mut self,
        t_ms: u64,
        point: Option<(f64, f64, u32)>,
        button: MouseButton,
        down: bool,
    ) -> usize {
        let (x, y, age_ms) = point.unwrap_or((0.0, 0.0, u32::MAX));
        let index = self.clicks.len();
        self.clicks.push(PendingClick {
            sample: ClickSample {
                t_ms,
                x: if x.is_finite() { x } else { 0.0 },
                y: if y.is_finite() { y } else { 0.0 },
                button,
                down,
                position_quality: ClickPositionQuality::Unavailable,
            },
            local: None,
            content: None,
            age_ms,
            refusal: Some(if self.closed {
                "selected source already closed"
            } else {
                "geometry observation pending"
            }),
        });
        index
    }

    pub(crate) fn refuse(&mut self, index: usize, reason: &'static str) {
        if !self.closed {
            if let Some(click) = self.clicks.get_mut(index) {
                click.refusal = Some(reason);
            }
        }
    }

    pub(crate) fn resolve_click(
        &mut self,
        index: usize,
        point: Option<(f64, f64, u32)>,
        rect: Option<WindowRect>,
    ) {
        if self.closed {
            return;
        }
        let Some(t_ms) = self.clicks.get(index).map(|click| click.sample.t_ms) else {
            return;
        };
        // Expired resize candidates cannot later be admitted. Keep frame work
        // bounded to the current freshness interval, not the whole take history.
        self.pending.retain(|index| {
            t_ms.saturating_sub(self.clicks[*index].sample.t_ms) <= u64::from(MAX_EVENT_AGE_MS)
        });
        let click = &mut self.clicks[index];
        let (x, y, age_ms) = point.unwrap_or((0.0, 0.0, u32::MAX));
        click.age_ms = age_ms;
        click.refusal = Some(if age_ms > MAX_EVENT_AGE_MS {
            "native event/queue/query age exceeded 100ms"
        } else {
            "native geometry or ownership unavailable"
        });
        if age_ms <= MAX_EVENT_AGE_MS && x.is_finite() && y.is_finite() {
            if let Some(rect) = rect.filter(|r| r.width > 0 && r.height > 0) {
                let local = (x - f64::from(rect.left), y - f64::from(rect.top));
                if local.0 >= 0.0
                    && local.1 >= 0.0
                    && local.0 < f64::from(rect.width)
                    && local.1 < f64::from(rect.height)
                {
                    click.local = Some(local);
                    click.content = Some((rect.width, rect.height));
                }
            }
        }
        admit(click, self.content, self.output);
        if click.local.is_some()
            && click.sample.position_quality == ClickPositionQuality::Unavailable
        {
            self.pending.push(index);
        }
    }

    #[cfg(test)]
    pub(crate) fn click(
        &mut self,
        t_ms: u64,
        point: Option<(f64, f64, u32)>,
        rect: Option<WindowRect>,
        button: MouseButton,
        down: bool,
    ) {
        let index = self.queue_click(t_ms, point, button, down);
        self.resolve_click(index, point, rect);
    }

    pub(crate) fn finish(self, duration_ms: u64) -> (Vec<ClickSample>, CursorCorrelation) {
        let mut exact = 0u32;
        let mut unavailable = 0u32;
        let mut age = None;
        let mut refusals = std::collections::BTreeMap::new();
        let clicks = self
            .clicks
            .into_iter()
            .filter(|c| c.sample.t_ms < duration_ms)
            .map(|c| {
                if c.sample.position_quality == ClickPositionQuality::Exact {
                    exact = exact.saturating_add(1);
                    age = Some(age.unwrap_or(0).max(u64::from(c.age_ms)));
                } else {
                    unavailable = unavailable.saturating_add(1);
                    *refusals
                        .entry(c.refusal.unwrap_or("matching encoded content unavailable"))
                        .or_insert(0u32) += 1;
                }
                c.sample
            })
            .collect();
        let correlation = CursorCorrelation {
            source: CursorCoordinateSource::RdevinAbsolute,
            state: if exact == 0 {
                CursorCoordinateState::Unavailable
            } else if unavailable > 0 {
                CursorCoordinateState::Approximate
            } else {
                CursorCoordinateState::Exact
            },
            exact_clicks: exact,
            approximate_clicks: 0,
            unavailable_clicks: unavailable,
            max_metadata_age_ms: age,
            detail: (unavailable > 0).then(|| {
                format!("{unavailable} selected-window transition(s) unavailable: {refusals:?}")
            }),
        };
        (clicks, correlation)
    }
}

fn admit(click: &mut PendingClick, fit: Option<WindowFrameFit>, output: (u32, u32)) {
    let Some(fit) =
        fit.filter(|fit| fit.output_size() == output && click.content == Some(fit.source_size()))
    else {
        return;
    };
    if let Some((x, y)) = click.local.and_then(|(x, y)| fit.map_point(x, y)) {
        click.sample.x = x;
        click.sample.y = y;
        click.sample.position_quality = ClickPositionQuality::Exact;
        click.refusal = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(left: i32, top: i32, width: u32, height: u32) -> WindowRect {
        WindowRect {
            left,
            top,
            width,
            height,
        }
    }
    #[test]
    fn mapped_window_click_drives_zoom_and_visible_blue_ripple_without_synthetic_cursor() {
        let mut mapped = WindowClicks::new(800, 600);
        mapped.frame(0, Some((800, 600)));
        mapped.click(
            1_000,
            Some((1040.0, 620.0, 1)),
            Some(rect(640, 320, 800, 600)),
            MouseButton::Left,
            true,
        );
        let (clicks, correlation) = mapped.finish(3_000);
        assert_eq!(correlation.exact_clicks, 1);
        let mut events = record_core::fixtures::generate("click-walkthrough").unwrap();
        events.screen_w = 800;
        events.screen_h = 600;
        events.duration_ms = 3_000;
        events.cursor.clear();
        events.clicks = clicks;
        events.cursor_correlation = correlation;
        let mut plan = record_engine::autoedit(&events, &record_engine::EngineConfig::default());
        plan.frame.enabled = false;
        plan.background = record_core::Background::Transparent;
        assert!(plan.cursor.smoothed.is_empty());
        assert_eq!(plan.clicks.len(), 1);
        assert!(plan.zoom.eval(1_100).0 > 1.5);
        let mut source = tiny_skia::Pixmap::new(800, 600).unwrap();
        source.fill(tiny_skia::Color::BLACK);
        // A real source landmark makes the rendered zoom observable in pixels.
        for y in 20..100usize {
            for x in 20..100usize {
                source.data_mut()[(y * 800 + x) * 4] = 255;
            }
        }
        let composed = record_render::compose_frame(&source, &plan, 1_100).unwrap();
        let blue_pixels = |pixels: &tiny_skia::Pixmap| {
            pixels
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[2] > 100 && p[1] > p[0] && p[2] > p[1])
                .count()
        };
        assert!(
            blue_pixels(&composed) > 50,
            "polish must visibly render the click's blue ripple"
        );
        let mut neutral = plan.clone();
        neutral.clicks.clear();
        neutral.zoom.keys.clear();
        let baseline = record_render::compose_frame(&source, &neutral, 1_100).unwrap();
        assert_eq!(blue_pixels(&baseline), 0);
        let landmark = (30 * 800 + 30) * 4;
        assert_eq!(baseline.data()[landmark], 255);
        assert_eq!(
            composed.data()[landmark],
            0,
            "the source landmark must leave the crop under actual zoom"
        );
    }

    #[test]
    fn actual_click_point_maps_negative_origin_and_move_without_last_mouse_guess() {
        let mut clicks = WindowClicks::new(800, 600);
        clicks.frame(0, Some((800, 600)));
        clicks.click(
            20,
            Some((-100.0, 250.0, 1)),
            Some(rect(-400, 100, 800, 600)),
            MouseButton::Left,
            true,
        );
        clicks.click(
            40,
            Some((500.0, 350.0, 1)),
            Some(rect(200, 200, 800, 600)),
            MouseButton::Left,
            false,
        );
        let (samples, proof) = clicks.finish(100);
        assert_eq!((samples[0].x, samples[0].y), (300.0, 150.0));
        assert_eq!((samples[1].x, samples[1].y), (300.0, 150.0));
        assert_eq!(proof.exact_clicks, 2);
    }
    #[test]
    fn physical_dpi_pixels_use_accepted_fit_and_grown_edges_are_preserved() {
        let mut clicks = WindowClicks::new(1000, 700);
        clicks.frame(0, Some((1600, 1200)));
        clicks.click(
            10,
            Some((1100.0, 600.0, 0)),
            Some(rect(200, 100, 1600, 1200)),
            MouseButton::Left,
            true,
        );
        clicks.click(
            20,
            Some((1400.0, 600.0, 0)),
            Some(rect(200, 100, 1600, 1200)),
            MouseButton::Left,
            true,
        );
        let (samples, proof) = clicks.finish(100);
        let fit = WindowFrameFit::new((1600, 1200), (1000, 700)).unwrap();
        assert_eq!(
            (samples[0].x, samples[0].y),
            fit.map_point(900.0, 500.0).unwrap()
        );
        assert_eq!(
            (samples[1].x, samples[1].y),
            fit.map_point(1200.0, 500.0).unwrap()
        );
        assert_eq!((proof.exact_clicks, proof.unavailable_clicks), (2, 0));
    }
    #[test]
    fn resizing_waits_for_matching_content_and_never_reuses_old_size() {
        let mut clicks = WindowClicks::new(800, 600);
        clicks.frame(0, Some((800, 600)));
        clicks.click(
            10,
            Some((300.0, 200.0, 2)),
            Some(rect(100, 100, 1000, 700)),
            MouseButton::Left,
            true,
        );
        clicks.frame(30, Some((1000, 700)));
        clicks.click(
            40,
            Some((300.0, 200.0, 0)),
            Some(rect(100, 100, 1200, 800)),
            MouseButton::Left,
            true,
        );
        clicks.frame(200, Some((1200, 800)));
        let (samples, proof) = clicks.finish(300);
        assert_eq!(samples[0].position_quality, ClickPositionQuality::Exact);
        assert_eq!(
            samples[1].position_quality,
            ClickPositionQuality::Unavailable
        );
        assert_eq!(proof.exact_clicks, 1);
    }
    #[test]
    fn stale_missing_foreign_closed_and_outside_points_remain_unavailable() {
        let mut clicks = WindowClicks::new(800, 600);
        clicks.frame(0, Some((800, 600)));
        clicks.click(
            10,
            Some((20.0, 20.0, 101)),
            Some(rect(0, 0, 800, 600)),
            MouseButton::Left,
            true,
        );
        clicks.click(
            20,
            None,
            Some(rect(0, 0, 800, 600)),
            MouseButton::Left,
            true,
        );
        clicks.click(30, Some((20.0, 20.0, 0)), None, MouseButton::Left, true);
        clicks.click(
            40,
            Some((800.0, 20.0, 0)),
            Some(rect(0, 0, 800, 600)),
            MouseButton::Left,
            true,
        );
        clicks.close();
        clicks.click(
            50,
            Some((20.0, 20.0, 0)),
            Some(rect(0, 0, 800, 600)),
            MouseButton::Left,
            true,
        );
        clicks.frame(60, Some((800, 600)));
        let (samples, proof) = clicks.finish(50);
        assert_eq!(samples.len(), 4);
        assert_eq!(proof.unavailable_clicks, 4);
        assert_eq!(proof.state, CursorCoordinateState::Unavailable);
    }
    #[test]
    fn grown_right_bottom_click_drives_aligned_visible_ripple_and_zoom() {
        let mut mapped = WindowClicks::new(800, 600);
        let fit = WindowFrameFit::new((900, 650), (800, 600)).unwrap();
        mapped.accepted_fit(0, Some(fit));
        // Both coordinates exceeded the old800x600 prefix and now map into
        // the exact accepted full-window destination.
        mapped.click(
            1000,
            Some((1050.0, 710.0, 1)),
            Some(rect(200, 100, 900, 650)),
            MouseButton::Left,
            true,
        );
        let (clicks, correlation) = mapped.finish(3000);
        let expected = fit.map_point(850.0, 610.0).unwrap();
        assert_eq!((clicks[0].x, clicks[0].y), expected);
        assert_eq!(correlation.exact_clicks, 1);
        let mut events = record_core::fixtures::generate("click-walkthrough").unwrap();
        events.screen_w = 800;
        events.screen_h = 600;
        events.duration_ms = 3000;
        events.cursor.clear();
        events.clicks = clicks;
        events.cursor_correlation = correlation;
        let mut plan = record_engine::autoedit(&events, &record_engine::EngineConfig::default());
        plan.frame.enabled = false;
        plan.background = record_core::Background::Transparent;
        assert!(plan.cursor.smoothed.is_empty());
        assert_eq!(plan.clicks.len(), 1);
        let (zoom, cx, cy) = plan.zoom.eval(1100);
        assert!(zoom > 1.5);
        let center = (
            400.0 + (expected.0 - cx * 800.0) * zoom,
            300.0 + (expected.1 - cy * 600.0) * zoom,
        );
        let mut source = tiny_skia::Pixmap::new(800, 600).unwrap();
        source.fill(tiny_skia::Color::BLACK);
        let polished = record_render::compose_frame(&source, &plan, 1100).unwrap();
        let blue = polished
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, p)| p[2] > 100 && p[1] > p[0] && p[2] > p[1])
            .map(|(i, _)| ((i % 800) as f64, (i / 800) as f64))
            .collect::<Vec<_>>();
        assert!(
            blue.len() > 50,
            "grown-edge click must visibly render blue pixels"
        );
        let mean = (
            blue.iter().map(|p| p.0).sum::<f64>() / blue.len() as f64,
            blue.iter().map(|p| p.1).sum::<f64>() / blue.len() as f64,
        );
        assert!(
            (mean.0 - center.0).abs() < 2.0 && (mean.1 - center.1).abs() < 2.0,
            "ripple pixels must align with accepted fit and actual zoom: {mean:?} != {center:?}"
        );
    }

    #[test]
    fn fit_for_different_output_cannot_admit_click() {
        let mut mapped = WindowClicks::new(800, 600);
        mapped.accepted_fit(0, WindowFrameFit::new((900, 650), (1000, 700)));
        mapped.click(
            20,
            Some((300.0, 300.0, 1)),
            Some(rect(0, 0, 900, 650)),
            MouseButton::Left,
            true,
        );
        let (_, proof) = mapped.finish(100);
        assert_eq!(proof.exact_clicks, 0);
        assert_eq!(proof.unavailable_clicks, 1);
    }
}
