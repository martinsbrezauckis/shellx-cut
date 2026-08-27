//! Private ScreenCaptureKit setup for a future macOS display-region stream.
//!
//! This is deliberately not wired to a public verb, picker, schema, UI, or
//! `CaptureConfig`. A later private coordinator must first consume its one-use
//! selection, then construct this request from the exact monitor identity and
//! native crop. Window capture cannot create this display-only request.

#![allow(dead_code)] // Private backend seam; public selection wiring remains deliberately gated.

use record_core::{error_codes, RecordError, Result};
use screencapturekit::{cg::CGRect, prelude::*, shareable_content::SCShareableContentInfo};

use crate::{
    macos_region_plan::{MacosRegionCaptureRequest, MacosRegionPlan},
    surface_coordinates::CaptureSurface,
};

/// Ready-to-start ScreenCaptureKit configuration plus its matching input map.
///
/// `destinationRect` is intentionally never set: ScreenCaptureKit emits the
/// selected crop at `output_size`, without an additional composition/scaling
/// target.
pub(crate) struct MacosRegionCaptureBackend {
    pub(crate) filter: SCContentFilter,
    pub(crate) stream_config: SCStreamConfiguration,
    pub(crate) input_surface: CaptureSurface,
    pub(crate) output_size: (u32, u32),
}

/// Refresh the exact native display and build a direct region stream.
///
/// No presentation field is consulted. A missing/replaced display, invalid
/// logical frame, unavailable fresh pixel size, or changed pixel mode fails
/// before an output is attached or a capture reservation can be consumed.
pub(crate) fn prepare_region_capture(
    request: &MacosRegionCaptureRequest,
    fps: u32,
    capture_cursor: bool,
) -> Result<MacosRegionCaptureBackend> {
    let display =
        crate::macos_monitor_target::resolve_monitor(request.monitor_id()).ok_or_else(|| {
            cap_err(
                "find selected display for region capture",
                "the selected display is no longer available; choose the region again",
            )
        })?;
    let frame = display.frame();
    let parent_surface = CaptureSurface::new(
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
    .ok_or_else(|| {
        cap_err(
            "validate selected display for region capture",
            "the selected display has invalid logical dimensions; choose the region again",
        )
    })?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();
    let pixels = SCShareableContentInfo::for_filter(&filter)
        .map(|info| info.pixel_size())
        .filter(|(width, height)| *width > 0 && *height > 0)
        .ok_or_else(|| {
            cap_err(
                "refresh selected display for region capture",
                "the selected display has no current native pixel dimensions; choose the region again",
            )
        })?;
    let plan = MacosRegionPlan::from_refreshed_parent(request.crop(), parent_surface, pixels)
        .ok_or_else(|| {
            cap_err(
                "validate selected region for capture",
                "the selected display changed size or scale; choose the region again",
            )
        })?;
    let source = plan.source_rect();
    let output_size = plan.output_size();
    let stream_config =
        crate::macos::recording_stream_config(output_size.0, output_size.1, fps, capture_cursor)
            .with_source_rect(CGRect::new(source.x, source.y, source.width, source.height));
    Ok(MacosRegionCaptureBackend {
        filter,
        stream_config,
        input_surface: plan.input_surface(),
        output_size,
    })
}

fn cap_err(context: &str, detail: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, detail)
        .with_action("choose the display region again after confirming Screen Recording permission")
}
