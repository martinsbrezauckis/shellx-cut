//! Private macOS region-capture geometry for a future ScreenCaptureKit consumer.
//!
//! The picker and server keep their selection state private. This module only
//! turns an already-validated native crop into the two backend facts that need
//! to agree: ScreenCaptureKit's display-local logical `sourceRect` and the
//! native-pixel output/input contract.

use crate::{
    region_geometry::{CaptureSubsurface, NativePixelCrop},
    surface_coordinates::CaptureSurface,
};

/// Private display-only input accepted by the future native coordinator.
///
/// Region capture has no window form. Rejecting a supplied window here keeps
/// the unsupported combination out of ScreenCaptureKit configuration and out
/// of the capture lifecycle before it can acquire output state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Constructed only when the still-private selection consumer is wired.
pub(crate) enum MacosRegionRequestError {
    InvalidMonitorIdentity,
    WindowRegionUnsupported,
}

pub(crate) struct MacosRegionCaptureRequest {
    monitor_id: String,
    crop: NativePixelCrop,
}

impl MacosRegionCaptureRequest {
    #[allow(dead_code)] // Private selection wiring is deliberately gated on native proof.
    pub(crate) fn new(
        monitor_id: String,
        crop: NativePixelCrop,
        window_id: Option<&str>,
    ) -> Result<Self, MacosRegionRequestError> {
        if window_id.is_some() {
            return Err(MacosRegionRequestError::WindowRegionUnsupported);
        }
        (!monitor_id.trim().is_empty() && monitor_id.len() <= 1_024)
            .then_some(Self { monitor_id, crop })
            .ok_or(MacosRegionRequestError::InvalidMonitorIdentity)
    }

    pub(crate) fn monitor_id(&self) -> &str {
        &self.monitor_id
    }

    pub(crate) fn crop(&self) -> NativePixelCrop {
        self.crop
    }
}

/// A ScreenCaptureKit source rectangle in display-local logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MacosLogicalSourceRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// Exact private contract for one region stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MacosRegionPlan {
    source_rect: MacosLogicalSourceRect,
    output_size: (u32, u32),
    input: CaptureSubsurface,
}

impl MacosRegionPlan {
    /// Build only from a fresh display's logical frame and pixel dimensions.
    ///
    /// `NativePixelCrop` retains the pixel dimensions it was selected against,
    /// so a mode/scale change is refused even if the old crop happens to fit
    /// inside the new display. No rounding, clamping, primary-display, title,
    /// or ordinal fallback is permitted here.
    pub(crate) fn from_refreshed_parent(
        crop: NativePixelCrop,
        parent_surface: CaptureSurface,
        refreshed_pixel_size: (u32, u32),
    ) -> Option<Self> {
        if crop.parent_size() != refreshed_pixel_size {
            return None;
        }
        let input = parent_surface.subsurface_from_native_crop(
            crop,
            refreshed_pixel_size.0,
            refreshed_pixel_size.1,
        )?;
        let (_, _, logical_width, logical_height) = parent_surface.global_geometry();
        let scale_x = checked_scale(refreshed_pixel_size.0, logical_width)?;
        let scale_y = checked_scale(refreshed_pixel_size.1, logical_height)?;
        let (left, top, width, height) = crop.origin_and_size();
        let right = left.checked_add(width)?;
        let bottom = top.checked_add(height)?;
        let x = logical_pixels(left, scale_x)?;
        let y = logical_pixels(top, scale_y)?;
        let right = logical_pixels(right, scale_x)?;
        let bottom = logical_pixels(bottom, scale_y)?;
        let width = checked_extent(x, right, logical_width)?;
        let height = checked_extent(y, bottom, logical_height)?;
        Some(Self {
            source_rect: MacosLogicalSourceRect {
                x,
                y,
                width,
                height,
            },
            output_size: crop.output_size(),
            input,
        })
    }

    pub(crate) fn source_rect(self) -> MacosLogicalSourceRect {
        self.source_rect
    }

    /// ScreenCaptureKit must be configured to this exact even pixel output.
    pub(crate) fn output_size(self) -> (u32, u32) {
        self.output_size
    }

    /// Global rdevin input is mapped only through this selected sub-surface.
    pub(crate) fn input_surface(self) -> CaptureSurface {
        self.input.capture_surface()
    }
}

fn checked_scale(pixel_size: u32, logical_size: f64) -> Option<f64> {
    let scale = f64::from(pixel_size) / logical_size;
    (pixel_size > 0
        && logical_size.is_finite()
        && logical_size > 0.0
        && scale.is_finite()
        && scale > 0.0)
        .then_some(scale)
}

fn logical_pixels(pixels: u32, scale: f64) -> Option<f64> {
    let points = f64::from(pixels) / scale;
    (scale.is_finite() && scale > 0.0 && points.is_finite() && points >= 0.0).then_some(points)
}

fn checked_extent(start: f64, end: f64, parent_size: f64) -> Option<f64> {
    let extent = end - start;
    (start.is_finite()
        && end.is_finite()
        && parent_size.is_finite()
        && start >= 0.0
        && end <= parent_size
        && extent.is_finite()
        && extent > 0.0)
        .then_some(extent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_crop_has_display_local_logical_source_and_native_output_contract() {
        let parent = CaptureSurface::new(-1_440.0, 20.0, 2_560.0, 1_440.0).unwrap();
        let crop = NativePixelCrop::new(512, 720, 2_560, 1_440, 5_120, 2_880).unwrap();
        let plan = MacosRegionPlan::from_refreshed_parent(crop, parent, (5_120, 2_880)).unwrap();

        assert_eq!(
            plan.source_rect(),
            MacosLogicalSourceRect {
                x: 256.0,
                y: 360.0,
                width: 1_280.0,
                height: 720.0,
            },
            "sourceRect uses display-local logical points, never desktop coordinates"
        );
        assert_eq!(plan.output_size(), (2_560, 1_440));
        assert_eq!(
            plan.input_surface().global_geometry(),
            (-1_184.0, 380.0, 1_280.0, 720.0),
            "the same native crop supplies input mapping"
        );
    }

    #[test]
    fn refreshed_pixel_mode_mismatch_refuses_even_when_old_crop_would_fit() {
        let parent = CaptureSurface::new(0.0, 0.0, 2_560.0, 1_440.0).unwrap();
        let crop = NativePixelCrop::new(0, 0, 2_560, 1_440, 5_120, 2_880).unwrap();

        assert!(
            MacosRegionPlan::from_refreshed_parent(crop, parent, (5_120, 2_160)).is_none(),
            "a fresh display mode/scale must exactly match the selected native frame"
        );
    }

    #[test]
    fn window_and_region_are_refused_before_backend_setup() {
        let crop = NativePixelCrop::new(0, 0, 640, 480, 1_920, 1_080).unwrap();
        assert!(matches!(
            MacosRegionCaptureRequest::new(
                "shellx-monitor-v1:macos:opaque".to_string(),
                crop,
                Some("window:opaque"),
            ),
            Err(MacosRegionRequestError::WindowRegionUnsupported)
        ));
        let request = MacosRegionCaptureRequest::new(
            "shellx-monitor-v1:macos:opaque".to_string(),
            crop,
            None,
        )
        .unwrap();
        assert_eq!(request.monitor_id(), "shellx-monitor-v1:macos:opaque");
        assert_eq!(request.crop(), crop);
    }
}
