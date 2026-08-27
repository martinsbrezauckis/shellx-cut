//! Deterministic contracts for future region-capture geometry.

use crate::{
    region_geometry::{NativePixelCrop, NormalizedCaptureBounds},
    surface_coordinates::{map_rdevin_input, CaptureSurface},
};
use record_core::{
    ClickPositionQuality, ClickSample, CursorCoordinateState, CursorSample, MouseButton,
    ScrollSample,
};

fn click(t_ms: u64, x: f64, y: f64, down: bool) -> ClickSample {
    ClickSample {
        t_ms,
        x,
        y,
        button: MouseButton::Left,
        down,
        position_quality: ClickPositionQuality::Exact,
    }
}

#[test]
fn normalized_subsurface_snaps_to_native_even_pixels_and_maps_exact_input() {
    let surface = CaptureSurface::new(-1920.0, 0.0, 1920.0, 1080.0).unwrap();
    let bounds = NormalizedCaptureBounds::new(0.251, 0.249, 0.749, 0.751).unwrap();
    let region = surface
        .subsurface_from_normalized(bounds, 3840, 2160)
        .expect("contained normalized region with an encodable output");

    assert_eq!(
        region.native_crop(),
        NativePixelCrop::new(962, 536, 1916, 1088, 3840, 2160).unwrap(),
        "the bounds snap outward to even native-pixel edges"
    );
    assert_eq!(region.native_crop().output_size(), (1916, 1088));

    let mut clicks = [
        click(10, -960.0, 540.0, true),
        click(11, -480.0, 540.0, false),
    ];
    let output = map_rdevin_input(
        Some(region.capture_surface()),
        region.native_crop().output_size().0,
        region.native_crop().output_size().1,
        vec![CursorSample {
            t_ms: 9,
            x: -960.0,
            y: 540.0,
        }],
        &mut clicks,
        vec![ScrollSample {
            t_ms: 12,
            x: -960.0,
            y: 540.0,
            dx: 0.0,
            dy: -1.0,
        }],
    );
    assert_eq!(
        (clicks[0].x, clicks[0].y),
        (958.0, 544.0),
        "the effective global origin and extent are derived after native snapping"
    );
    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
    assert_eq!(
        clicks[1].position_quality,
        ClickPositionQuality::Unavailable,
        "a point beyond the crop's half-open right edge is never clamped"
    );
    assert_eq!((output.cursor[0].x, output.cursor[0].y), (958.0, 544.0));
    assert_eq!((output.scrolls[0].x, output.scrolls[0].y), (958.0, 544.0));
    assert!(output.cursor.iter().all(|sample| sample.x >= 0.0
        && sample.y >= 0.0
        && sample.x < f64::from(region.native_crop().output_size().0)
        && sample.y < f64::from(region.native_crop().output_size().1)));
    assert_eq!(output.correlation.state, CursorCoordinateState::Approximate);
    assert_eq!(output.correlation.exact_clicks, 1);
    assert_eq!(output.correlation.unavailable_clicks, 1);
}

#[test]
fn malformed_normalized_and_stale_native_bounds_fail_without_clamping() {
    for bounds in [
        NormalizedCaptureBounds::new(f64::NAN, 0.0, 1.0, 1.0),
        NormalizedCaptureBounds::new(-0.01, 0.0, 1.0, 1.0),
        NormalizedCaptureBounds::new(0.0, 0.0, 1.01, 1.0),
        NormalizedCaptureBounds::new(0.5, 0.0, 0.5, 1.0),
        NormalizedCaptureBounds::new(0.0, 0.5, 1.0, 0.5),
    ] {
        assert!(bounds.is_none());
    }

    let surface = CaptureSurface::new(100.0, 200.0, 800.0, 600.0).unwrap();
    let too_small = NormalizedCaptureBounds::new(0.999, 0.0, 1.0, 1.0).unwrap();
    assert!(
        surface
            .subsurface_from_normalized(too_small, 1001, 1000)
            .is_none(),
        "a non-encodable crop is rejected instead of expanded"
    );

    let crop = NativePixelCrop::new(200, 100, 600, 400, 1000, 800).unwrap();
    assert!(
        surface
            .subsurface_from_native_crop(crop, 799, 600)
            .is_none(),
        "a crop paired with stale native output geometry is rejected"
    );
    assert!(NativePixelCrop::new(201, 100, 600, 400, 1000, 800).is_none());
    assert!(NativePixelCrop::new(200, 100, 599, 400, 1000, 800).is_none());
}

#[test]
fn region_mapping_preserves_half_open_output_edges_and_input_quality() {
    let surface = CaptureSurface::new(0.0, 0.0, 1000.0, 1000.0).unwrap();
    let region = surface
        .subsurface_from_normalized(
            NormalizedCaptureBounds::new(0.25, 0.25, 0.75, 0.75).unwrap(),
            1000,
            1000,
        )
        .unwrap();
    let mut clicks = [
        click(10, 749.999, 749.999, true),
        click(11, 750.0, 750.0, false),
        ClickSample {
            position_quality: ClickPositionQuality::Approximate,
            ..click(12, 500.0, 500.0, true)
        },
    ];
    let output = map_rdevin_input(
        Some(region.capture_surface()),
        region.native_crop().output_size().0,
        region.native_crop().output_size().1,
        vec![CursorSample {
            t_ms: 9,
            x: 750.0,
            y: 750.0,
        }],
        &mut clicks,
        vec![],
    );

    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
    assert!(clicks[0].x < 500.0 && clicks[0].y < 500.0);
    assert_eq!(
        clicks[1].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert_eq!(
        clicks[2].position_quality,
        ClickPositionQuality::Unavailable,
        "approximate input can never become exact because a crop was valid"
    );
    assert!(
        output.cursor.is_empty(),
        "the right/bottom edge is outside [0, 500)"
    );
    assert_eq!(output.correlation.exact_clicks, 1);
    assert_eq!(output.correlation.unavailable_clicks, 2);
}
