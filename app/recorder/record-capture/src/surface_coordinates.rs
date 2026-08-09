//! Validated conversion from absolute desktop input to captured-frame pixels.
//!
//! rdevin reports a global desktop point. It becomes exact only after a native
//! backend supplies the selected capture surface's global origin and coordinate size.

use record_core::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState,
    CursorCorrelation, CursorSample, ScrollSample,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureSurface {
    origin_x: f64,
    origin_y: f64,
    coordinate_width: f64,
    coordinate_height: f64,
}

impl CaptureSurface {
    pub(crate) fn new(
        origin_x: f64,
        origin_y: f64,
        coordinate_width: f64,
        coordinate_height: f64,
    ) -> Option<Self> {
        (origin_x.is_finite()
            && origin_y.is_finite()
            && coordinate_width.is_finite()
            && coordinate_height.is_finite()
            && coordinate_width > 0.0
            && coordinate_height > 0.0)
            .then_some(Self {
                origin_x,
                origin_y,
                coordinate_width,
                coordinate_height,
            })
    }

    fn for_output(self, width: u32, height: u32) -> Option<SurfaceTransform> {
        (width > 0 && height > 0).then_some(SurfaceTransform {
            surface: self,
            scale_x: f64::from(width) / self.coordinate_width,
            scale_y: f64::from(height) / self.coordinate_height,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SurfaceTransform {
    surface: CaptureSurface,
    scale_x: f64,
    scale_y: f64,
}

impl SurfaceTransform {
    fn point(self, x: f64, y: f64) -> Option<(f64, f64)> {
        let x = x - self.surface.origin_x;
        let y = y - self.surface.origin_y;
        (x.is_finite()
            && y.is_finite()
            && (0.0..self.surface.coordinate_width).contains(&x)
            && (0.0..self.surface.coordinate_height).contains(&y))
        .then_some((x * self.scale_x, y * self.scale_y))
    }
}

pub(crate) struct AbsoluteInputOutput {
    pub cursor: Vec<CursorSample>,
    pub scrolls: Vec<ScrollSample>,
    pub correlation: CursorCorrelation,
}

/// Map global rdevin input into the selected native surface.
///
/// Click transitions outside a selected window/monitor and every click without a
/// validated transform are `Unavailable`; they cannot become auto-zoom anchors.
/// Raw global cursor/scroll samples are dropped in that case because EventTrack's
/// coordinate contract is captured-frame pixels, never desktop pixels.
pub(crate) fn map_rdevin_input(
    surface: Option<CaptureSurface>,
    output_width: u32,
    output_height: u32,
    cursor: Vec<CursorSample>,
    clicks: &mut [ClickSample],
    scrolls: Vec<ScrollSample>,
) -> AbsoluteInputOutput {
    let Some(transform) =
        surface.and_then(|surface| surface.for_output(output_width, output_height))
    else {
        mark_unavailable(clicks);
        return unavailable_output(
            clicks.len(),
            "the native capture surface could not be validated",
        );
    };

    let cursor = cursor
        .into_iter()
        .filter_map(|sample| {
            transform
                .point(sample.x, sample.y)
                .map(|(x, y)| CursorSample { x, y, ..sample })
        })
        .collect();
    let scrolls = scrolls
        .into_iter()
        .filter_map(|sample| {
            transform
                .point(sample.x, sample.y)
                .map(|(x, y)| ScrollSample { x, y, ..sample })
        })
        .collect();

    let mut exact = 0_u32;
    let mut unavailable = 0_u32;
    let mut no_absolute_position = 0_u32;
    let mut outside_surface = 0_u32;
    for click in clicks {
        if click.position_quality != ClickPositionQuality::Exact {
            click.position_quality = ClickPositionQuality::Unavailable;
            unavailable = unavailable.saturating_add(1);
            no_absolute_position = no_absolute_position.saturating_add(1);
        } else if let Some((x, y)) = transform.point(click.x, click.y) {
            click.x = x;
            click.y = y;
            click.position_quality = ClickPositionQuality::Exact;
            exact = exact.saturating_add(1);
        } else {
            click.position_quality = ClickPositionQuality::Unavailable;
            unavailable = unavailable.saturating_add(1);
            outside_surface = outside_surface.saturating_add(1);
        }
    }
    AbsoluteInputOutput {
        cursor,
        scrolls,
        correlation: CursorCorrelation {
            source: CursorCoordinateSource::RdevinAbsolute,
            state: coordinate_state(exact, unavailable),
            exact_clicks: exact,
            approximate_clicks: 0,
            unavailable_clicks: unavailable,
            max_metadata_age_ms: None,
            detail: unavailable_detail(no_absolute_position, outside_surface),
        },
    }
}

/// Window capture needs timestamped window rectangles to map global input. Neither
/// native backend currently supplies them on the capture clock, so a launch-time
/// rectangle is deliberately never reused after the window may have moved or resized.
#[cfg(any(test, windows, target_os = "macos"))]
pub(crate) fn unavailable_window_rdevin_input(
    _cursor: Vec<CursorSample>,
    clicks: &mut [ClickSample],
    _scrolls: Vec<ScrollSample>,
) -> AbsoluteInputOutput {
    mark_unavailable(clicks);
    unavailable_output(
        clicks.len(),
        "the selected window has no timestamped capture-surface geometry samples",
    )
}

fn unavailable_detail(no_absolute_position: u32, outside_surface: u32) -> Option<String> {
    match (no_absolute_position, outside_surface) {
        (0, 0) => None,
        (count, 0) => Some(format!(
            "{count} button transition(s) arrived before an absolute cursor position"
        )),
        (0, count) => Some(format!(
            "{count} button transition(s) were outside the captured surface"
        )),
        (unknown, outside) => Some(format!(
            "{unknown} button transition(s) arrived before an absolute cursor position; {outside} were outside the captured surface"
        )),
    }
}

fn unavailable_output(click_count: usize, detail: &str) -> AbsoluteInputOutput {
    AbsoluteInputOutput {
        cursor: vec![],
        scrolls: vec![],
        correlation: CursorCorrelation {
            source: CursorCoordinateSource::RdevinAbsolute,
            state: CursorCoordinateState::Unavailable,
            exact_clicks: 0,
            approximate_clicks: 0,
            unavailable_clicks: u32::try_from(click_count).unwrap_or(u32::MAX),
            max_metadata_age_ms: None,
            detail: Some(detail.to_string()),
        },
    }
}

fn mark_unavailable(clicks: &mut [ClickSample]) {
    for click in clicks {
        click.position_quality = ClickPositionQuality::Unavailable;
    }
}

fn coordinate_state(exact: u32, unavailable: u32) -> CursorCoordinateState {
    // This receipt is specifically about click-coordinate provenance. A validated
    // transform alone does not prove a click position when there were no button
    // transitions, so never advertise a vacuous `Exact` state.
    if exact == 0 {
        CursorCoordinateState::Unavailable
    } else if unavailable != 0 {
        CursorCoordinateState::Approximate
    } else {
        CursorCoordinateState::Exact
    }
}
