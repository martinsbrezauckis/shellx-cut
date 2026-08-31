//! Private ScreenCaptureKit setup for a future macOS display-region stream.
//!
//! This is deliberately not wired to a public verb, picker, schema, or UI.
//! Candidate-private capture transport may reach it only after a later native
//! picker has supplied an exact display identity and native crop. The server
//! must keep its one-use selection ticket private until that native picker
//! exists; window capture cannot create this display-only request.

#![allow(dead_code)] // Private backend seam; public selection wiring remains deliberately gated.

use record_core::{error_codes, RecordError, Result};
use screencapturekit::{cg::CGRect, prelude::*, shareable_content::SCShareableContentInfo};

use crate::{
    macos_region_plan::{completed_region_output_size, MacosRegionCaptureRequest, MacosRegionPlan},
    region_geometry::NativePixelCrop,
    surface_coordinates::CaptureSurface,
    CaptureConfig, CaptureRegion,
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
    let output_size = plan.output_size();
    let stream_config = stream_configuration(plan, fps, capture_cursor);
    Ok(MacosRegionCaptureBackend {
        filter,
        stream_config,
        input_surface: plan.input_surface(),
        output_size,
    })
}

/// Convert only the non-serializable region transport into a native request.
/// There is deliberately no server selection-ticket consumer here: tickets
/// remain unwired until a real macOS visual picker can issue one.
pub(crate) fn prepare_region_capture_from_config(
    config: &CaptureConfig,
    fps: u32,
) -> Result<MacosRegionCaptureBackend> {
    let monitor_id = config.monitor_id.as_ref().ok_or_else(|| {
        cap_err(
            "capture selected region",
            "a region requires one exact display identity and cannot target a window",
        )
    })?;
    let region = config.region.ok_or_else(|| {
        cap_err(
            "capture selected region",
            "a region requires one native-picker-admitted crop",
        )
    })?;
    let crop = NativePixelCrop::from_capture_region(region).ok_or_else(|| {
        cap_err(
            "validate selected region",
            "the region is not an even, contained native-pixel crop",
        )
    })?;
    let request =
        MacosRegionCaptureRequest::new(monitor_id.clone(), crop, config.window.as_deref())
            .map_err(|_| {
                cap_err(
                    "capture selected region",
                    "a region requires one exact display identity and cannot target a window",
                )
            })?;
    prepare_region_capture(&request, fps, config.capture_cursor)
}

/// Consume-time proof for the private server ticket registry. It performs the
/// same exact-display, logical-frame, physical-pixel, and scale checks as the
/// real launch path without attaching an output or starting a stream. The
/// capture worker repeats this immediately before native launch, so a topology
/// change in either gap fails closed.
pub(crate) fn selection_is_current(monitor_id: &str, region: CaptureRegion) -> bool {
    let Some(crop) = NativePixelCrop::from_capture_region(region) else {
        return false;
    };
    let Ok(request) = MacosRegionCaptureRequest::new(monitor_id.to_owned(), crop, None) else {
        return false;
    };
    prepare_region_capture(&request, 30, false).is_ok()
}

/// Configure one pre-output ScreenCaptureKit crop. The source rectangle is in
/// display-local logical points while width/height remain exact native pixels;
/// a destination rectangle would introduce a second composition transform and
/// is intentionally absent.
fn stream_configuration(
    plan: MacosRegionPlan,
    fps: u32,
    capture_cursor: bool,
) -> SCStreamConfiguration {
    let source = plan.source_rect();
    let (width, height) = plan.output_size();
    crate::macos::recording_stream_config(width, height, fps, capture_cursor)
        .with_source_rect(CGRect::new(source.x, source.y, source.width, source.height))
}

/// Preserve the configured output fact only when probing is unavailable. Once
/// a completed file reports dimensions, a region capture must match its exact
/// selected native crop; reporting a mismatch as a successful region would
/// corrupt both source dimensions and pointer mapping.
pub(crate) fn verified_region_output_size(
    expected: (u32, u32),
    observed: Option<(u32, u32)>,
) -> Result<(u32, u32)> {
    completed_region_output_size(expected, observed).ok_or_else(|| {
        let actual = observed.expect("completed output mismatch has observed dimensions");
        cap_err(
            "verify ScreenCaptureKit region output dimensions",
            &format!(
                "the recorded region is {}×{}, but the selected native crop was {}×{}",
                actual.0, actual.1, expected.0, expected.1
            ),
        )
    })
}

fn cap_err(context: &str, detail: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, detail)
        .with_action("choose the display region again after confirming Screen Recording permission")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region_geometry::NativePixelCrop;

    #[test]
    fn region_stream_configuration_binds_source_rect_native_output_and_cursor() {
        let parent = CaptureSurface::new(-1_440.0, 20.0, 2_560.0, 1_440.0).unwrap();
        let crop = NativePixelCrop::new(512, 720, 2_560, 1_440, 5_120, 2_880).unwrap();
        let plan = MacosRegionPlan::from_refreshed_parent(crop, parent, (5_120, 2_880)).unwrap();

        let hidden_cursor = stream_configuration(plan, 30, false);
        let source = hidden_cursor.source_rect();
        assert_eq!(hidden_cursor.width(), 2_560);
        assert_eq!(hidden_cursor.height(), 1_440);
        assert!(!hidden_cursor.shows_cursor());
        assert_eq!(
            (
                source.origin.x,
                source.origin.y,
                source.size.width,
                source.size.height
            ),
            (256.0, 360.0, 1_280.0, 720.0),
            "ScreenCaptureKit receives display-local logical sourceRect points"
        );
        assert!(stream_configuration(plan, 30, true).shows_cursor());
    }
}
