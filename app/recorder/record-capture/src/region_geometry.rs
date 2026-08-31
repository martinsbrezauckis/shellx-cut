//! Capture-region bounds, native crop alignment, and global-coordinate derivation.
//!
//! This module is deliberately separate from `surface_coordinates`: it owns the
//! future region-capture geometry, while that module owns event mapping into a
//! validated output frame.

use crate::{surface_coordinates::CaptureSurface, CaptureRegion};

/// A strictly contained crop in normalized parent-frame coordinates.
///
/// Bounds are half-open so `right == 1.0` and `bottom == 1.0` denote the final
/// parent-frame edges, but no point at either edge belongs to the region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NormalizedCaptureBounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl NormalizedCaptureBounds {
    pub(crate) fn new(left: f64, top: f64, right: f64, bottom: f64) -> Option<Self> {
        (left.is_finite()
            && top.is_finite()
            && right.is_finite()
            && bottom.is_finite()
            && (0.0..=1.0).contains(&left)
            && (0.0..=1.0).contains(&top)
            && (0.0..=1.0).contains(&right)
            && (0.0..=1.0).contains(&bottom)
            && left < right
            && top < bottom)
            .then_some(Self {
                left,
                top,
                right,
                bottom,
            })
    }
}

/// A native frame crop with the even offsets and dimensions required for the
/// recorder's YUV420/H.264 output path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativePixelCrop {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    // A native crop is valid only for the exact pixel frame it was selected
    // against. Keeping that frame here makes a later backend fail closed if a
    // display changes scale or mode between selection and capture.
    parent_width: u32,
    parent_height: u32,
}

impl NativePixelCrop {
    pub(crate) fn from_capture_region(region: CaptureRegion) -> Option<Self> {
        let (left, top, width, height, parent_width, parent_height) = region.native_parts();
        Self::new(left, top, width, height, parent_width, parent_height)
    }

    /// Validate a native crop that is already expressed in parent-frame pixels.
    /// This is the equivalent of a normalized region once its edges have been
    /// snapped by the native capture surface.
    pub(crate) fn new(
        left: u32,
        top: u32,
        width: u32,
        height: u32,
        parent_width: u32,
        parent_height: u32,
    ) -> Option<Self> {
        let crop = Self {
            left,
            top,
            width,
            height,
            parent_width,
            parent_height,
        };
        crop.is_contained_in(parent_width, parent_height)
            .then_some(crop)
    }

    fn from_normalized(
        bounds: NormalizedCaptureBounds,
        parent_width: u32,
        parent_height: u32,
    ) -> Option<Self> {
        let (left, right) = snap_native_range(bounds.left, bounds.right, parent_width)?;
        let (top, bottom) = snap_native_range(bounds.top, bounds.bottom, parent_height)?;
        Self::new(
            left,
            top,
            right.checked_sub(left)?,
            bottom.checked_sub(top)?,
            parent_width,
            parent_height,
        )
    }

    fn is_contained_in(self, parent_width: u32, parent_height: u32) -> bool {
        let Some(right) = self.left.checked_add(self.width) else {
            return false;
        };
        let Some(bottom) = self.top.checked_add(self.height) else {
            return false;
        };
        self.parent_size() == (parent_width, parent_height)
            && self.left.is_multiple_of(2)
            && self.top.is_multiple_of(2)
            && self.width >= 2
            && self.height >= 2
            && self.width.is_multiple_of(2)
            && self.height.is_multiple_of(2)
            && right <= parent_width
            && bottom <= parent_height
    }

    pub(crate) fn output_size(self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Native-pixel origin and size. This stays crate-private so native
    /// backends can configure an exact crop without exposing coordinates to a
    /// picker, public verb, receipt, or UI.
    pub(crate) fn origin_and_size(self) -> (u32, u32, u32, u32) {
        (self.left, self.top, self.width, self.height)
    }

    /// The exact parent pixel dimensions used to validate this crop.
    pub(crate) fn parent_size(self) -> (u32, u32) {
        (self.parent_width, self.parent_height)
    }
}

/// A native crop and the matching global input surface.
///
/// Future region capture can give `native_crop` directly to a native frame path
/// and use `surface` for exact global input mapping without assuming that display
/// coordinates and physical frame pixels have the same scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureSubsurface {
    surface: CaptureSurface,
    native_crop: NativePixelCrop,
}

impl CaptureSubsurface {
    pub(crate) fn capture_surface(self) -> CaptureSurface {
        self.surface
    }

    pub(crate) fn native_crop(self) -> NativePixelCrop {
        self.native_crop
    }
}

impl CaptureSurface {
    /// Derive a native-pixel-aligned sub-surface from a normalized region.
    ///
    /// The normalized rectangle is half-open (`[left, right) x [top, bottom)`).
    /// Its native edges are snapped outward to even pixels so the requested region
    /// remains contained in an H.264-compatible crop. Invalid bounds and a crop
    /// that cannot be represented inside this exact native frame are refused; this
    /// method deliberately does not clamp either one.
    pub(crate) fn subsurface_from_normalized(
        self,
        bounds: NormalizedCaptureBounds,
        native_width: u32,
        native_height: u32,
    ) -> Option<CaptureSubsurface> {
        let crop = NativePixelCrop::from_normalized(bounds, native_width, native_height)?;
        self.subsurface_from_native_crop(crop, native_width, native_height)
    }

    /// Derive a sub-surface from an already validated native crop.
    ///
    /// Revalidating containment against the supplied native frame makes a crop
    /// descriptor fail closed if it is later paired with stale output geometry.
    pub(crate) fn subsurface_from_native_crop(
        self,
        crop: NativePixelCrop,
        native_width: u32,
        native_height: u32,
    ) -> Option<CaptureSubsurface> {
        if !crop.is_contained_in(native_width, native_height) {
            return None;
        }

        let (origin_x, origin_y, coordinate_width, coordinate_height) = self.global_geometry();
        let native_width = f64::from(native_width);
        let native_height = f64::from(native_height);
        let surface = Self::new(
            origin_x + f64::from(crop.left) * coordinate_width / native_width,
            origin_y + f64::from(crop.top) * coordinate_height / native_height,
            f64::from(crop.width) * coordinate_width / native_width,
            f64::from(crop.height) * coordinate_height / native_height,
        )?;
        Some(CaptureSubsurface {
            surface,
            native_crop: crop,
        })
    }
}

fn snap_native_range(start: f64, end: f64, limit: u32) -> Option<(u32, u32)> {
    // NormalizedCaptureBounds already checked finiteness, containment, and order.
    // Snap outward rather than silently shrinking the requested half-open region.
    let start = native_pixel(start * f64::from(limit), false)? & !1;
    let end = native_pixel(end * f64::from(limit), true)?;
    let end = if end % 2 == 0 {
        end
    } else {
        end.checked_add(1)?
    };
    (start < end && end <= limit).then_some((start, end))
}

fn native_pixel(value: f64, round_up: bool) -> Option<u32> {
    let value = if round_up {
        value.ceil()
    } else {
        value.floor()
    };
    (value.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&value)).then_some(value as u32)
}
