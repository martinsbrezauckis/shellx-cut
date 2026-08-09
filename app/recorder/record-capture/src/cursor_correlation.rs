//! Wayland click-to-cursor correlation in one explicit capture clock.
//!
//! evdev reports button timing but only relative pointer deltas. PipeWire's
//! `SPA_META_Cursor` reports the absolute compositor position. This module maps
//! that compositor-space metadata to the captured frame pixels and only promotes a
//! click to `Exact` when its nearest sample is fresh enough.

use record_core::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState,
    CursorCorrelation, CursorSample, ScrollSample,
};

use crate::surface_coordinates::{self, CaptureSurface};

/// A click and metadata callback may arrive on different threads, but both stamp the
/// same capture-start `Instant`. Beyond this distance we refuse to reuse a position.
pub(crate) const MAX_CURSOR_METADATA_AGE_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorMetadataSample {
    pub t_ms: u64,
    /// `SPA_META_Cursor.position` in compositor coordinate space.
    pub x: f64,
    pub y: f64,
}

/// The portal's monitor geometry in compositor coordinates. The portal explicitly
/// says this can differ from the PipeWire frame's physical-pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PortalCursorGeometry {
    origin_x: f64,
    origin_y: f64,
    logical_width: f64,
    logical_height: f64,
}

impl PortalCursorGeometry {
    pub(crate) fn from_portal(
        position: Option<(i32, i32)>,
        size: Option<(i32, i32)>,
    ) -> Option<Self> {
        let ((origin_x, origin_y), (width, height)) = position.zip(size)?;
        let (logical_width, logical_height) = (f64::from(width), f64::from(height));
        (logical_width > 0.0 && logical_height > 0.0).then_some(Self {
            origin_x: f64::from(origin_x),
            origin_y: f64::from(origin_y),
            logical_width,
            logical_height,
        })
    }

    fn frame_transform(self, frame_width: u32, frame_height: u32) -> Option<FrameTransform> {
        (frame_width > 0 && frame_height > 0).then_some(FrameTransform {
            origin_x: self.origin_x,
            origin_y: self.origin_y,
            logical_width: self.logical_width,
            logical_height: self.logical_height,
            scale_x: f64::from(frame_width) / self.logical_width,
            scale_y: f64::from(frame_height) / self.logical_height,
        })
    }

    fn capture_surface(self) -> Option<CaptureSurface> {
        CaptureSurface::new(
            self.origin_x,
            self.origin_y,
            self.logical_width,
            self.logical_height,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FrameTransform {
    origin_x: f64,
    origin_y: f64,
    logical_width: f64,
    logical_height: f64,
    scale_x: f64,
    scale_y: f64,
}

impl FrameTransform {
    fn apply(self, sample: CursorMetadataSample) -> Option<CursorSample> {
        let local_x = sample.x - self.origin_x;
        let local_y = sample.y - self.origin_y;
        if !(local_x.is_finite()
            && local_y.is_finite()
            && (0.0..self.logical_width).contains(&local_x)
            && (0.0..self.logical_height).contains(&local_y))
        {
            return None;
        }
        Some(CursorSample {
            t_ms: sample.t_ms,
            x: local_x * self.scale_x,
            y: local_y * self.scale_y,
        })
    }
}

/// The negotiated PipeWire video dimensions are physical frame pixels, while the
/// portal geometry is compositor-space. Keeping both values makes fractional scale
/// and non-zero monitor origins explicit rather than silently assuming 1:1 pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct PipewireCursorCapture {
    pub metadata: Vec<CursorMetadataSample>,
    pub frame_width: u32,
    pub frame_height: u32,
    /// Shared capture-clock bounds measured from actual PipeWire frames.
    pub capture_start_ms: u64,
    pub capture_end_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionInputMode {
    pub wayland: bool,
    pub use_evdev: bool,
    pub use_pipewire_metadata: bool,
}

pub(crate) fn session_input_mode() -> SessionInputMode {
    session_input_mode_from(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
        std::env::var("SHELLX_RECORD_INPUT").ok().as_deref(),
        std::env::var("SHELLX_RECORD_WAYLAND_CAPTURE")
            .ok()
            .as_deref(),
    )
}

pub(crate) fn session_input_mode_from(
    session_type: Option<&str>,
    wayland_display: bool,
    display: bool,
    input_override: Option<&str>,
    capture_override: Option<&str>,
) -> SessionInputMode {
    let wayland = session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
        || (wayland_display && !display);
    let use_evdev = match input_override {
        Some("evdev") => true,
        Some("rdevin") => false,
        _ => wayland,
    };
    let use_pipewire_metadata = match capture_override {
        Some("gst") => false,
        Some("pipewire") => true,
        _ => wayland,
    };
    SessionInputMode {
        wayland,
        use_evdev,
        use_pipewire_metadata,
    }
}

pub(crate) struct CorrelationOutput {
    pub cursor: Vec<CursorSample>,
    pub scrolls: Vec<ScrollSample>,
    pub status: CursorCorrelation,
}

pub(crate) fn correlate_clicks(
    mode: SessionInputMode,
    clicks: &mut [ClickSample],
    fallback_cursor: Vec<CursorSample>,
    fallback_scrolls: Vec<ScrollSample>,
    pipewire: Option<PipewireCursorCapture>,
    portal_geometry: Option<PortalCursorGeometry>,
    rdevin_frame_size: Option<(u32, u32)>,
) -> CorrelationOutput {
    if !mode.use_evdev {
        let surface = portal_geometry.and_then(PortalCursorGeometry::capture_surface);
        let (frame_width, frame_height) = rdevin_frame_size.unwrap_or_default();
        let mapped = surface_coordinates::map_rdevin_input(
            surface,
            frame_width,
            frame_height,
            fallback_cursor,
            clicks,
            fallback_scrolls,
        );
        return CorrelationOutput {
            cursor: mapped.cursor,
            scrolls: mapped.scrolls,
            status: mapped.correlation,
        };
    }
    if !mode.wayland || !mode.use_pipewire_metadata {
        return approximate_output(
            clicks,
            fallback_cursor,
            fallback_scrolls,
            "Wayland absolute PipeWire cursor metadata is not active",
        );
    }
    let Some(pipewire) = pipewire else {
        return approximate_output(
            clicks,
            fallback_cursor,
            fallback_scrolls,
            "PipeWire cursor metadata was unavailable for this capture",
        );
    };
    let Some(transform) = portal_geometry
        .and_then(|geometry| geometry.frame_transform(pipewire.frame_width, pipewire.frame_height))
    else {
        return approximate_output(
            clicks,
            fallback_cursor,
            fallback_scrolls,
            "the portal did not provide a usable monitor coordinate transform",
        );
    };
    let mut cursor: Vec<_> = pipewire
        .metadata
        .into_iter()
        .filter_map(|sample| transform.apply(sample))
        .collect();
    cursor.sort_unstable_by_key(|sample| sample.t_ms);
    if cursor.is_empty() {
        return approximate_output(
            clicks,
            fallback_cursor,
            fallback_scrolls,
            "PipeWire supplied no cursor metadata inside the captured monitor",
        );
    }

    let mut exact = 0_u32;
    let mut approximate = 0_u32;
    for click in clicks {
        if let Some(sample) = nearest_fresh_sample(&cursor, click.t_ms) {
            click.x = sample.x;
            click.y = sample.y;
            click.position_quality = ClickPositionQuality::Exact;
            exact = exact.saturating_add(1);
        } else {
            // Retain the evdev location for a visible approximate marker, but the
            // engine filters it out of auto-zoom anchors through `position_quality`.
            click.position_quality = ClickPositionQuality::Approximate;
            approximate = approximate.saturating_add(1);
        }
    }
    let state = if approximate != 0 {
        CursorCoordinateState::Approximate
    } else if exact != 0 {
        CursorCoordinateState::Exact
    } else {
        CursorCoordinateState::Unavailable
    };
    let detail = if approximate != 0 {
        Some(format!(
            "{approximate} click position(s) had no PipeWire cursor sample within {MAX_CURSOR_METADATA_AGE_MS}ms and remain approximate"
        ))
    } else if exact == 0 {
        Some("no button transitions carried cursor-coordinate evidence".to_string())
    } else {
        None
    };
    CorrelationOutput {
        cursor,
        scrolls: fallback_scrolls,
        status: CursorCorrelation {
            source: CursorCoordinateSource::WaylandPipewireMetadata,
            state,
            exact_clicks: exact,
            approximate_clicks: approximate,
            unavailable_clicks: 0,
            max_metadata_age_ms: Some(MAX_CURSOR_METADATA_AGE_MS),
            detail,
        },
    }
}

fn approximate_output(
    clicks: &mut [ClickSample],
    fallback_cursor: Vec<CursorSample>,
    fallback_scrolls: Vec<ScrollSample>,
    detail: &str,
) -> CorrelationOutput {
    mark_clicks(clicks, ClickPositionQuality::Approximate);
    CorrelationOutput {
        cursor: fallback_cursor,
        scrolls: fallback_scrolls,
        status: CursorCorrelation {
            source: CursorCoordinateSource::WaylandEvdevRelative,
            state: if clicks.is_empty() {
                CursorCoordinateState::Unavailable
            } else {
                CursorCoordinateState::Approximate
            },
            exact_clicks: 0,
            approximate_clicks: u32::try_from(clicks.len()).unwrap_or(u32::MAX),
            unavailable_clicks: 0,
            max_metadata_age_ms: Some(MAX_CURSOR_METADATA_AGE_MS),
            detail: Some(detail.to_string()),
        },
    }
}

fn mark_clicks(clicks: &mut [ClickSample], quality: ClickPositionQuality) {
    for click in clicks {
        click.position_quality = quality;
    }
}

fn nearest_fresh_sample(samples: &[CursorSample], click_t_ms: u64) -> Option<CursorSample> {
    samples
        .iter()
        .min_by_key(|sample| (sample.t_ms.abs_diff(click_t_ms), sample.t_ms))
        .copied()
        .filter(|sample| sample.t_ms.abs_diff(click_t_ms) <= MAX_CURSOR_METADATA_AGE_MS)
}
