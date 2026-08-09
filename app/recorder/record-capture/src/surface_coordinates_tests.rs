//! Deterministic native capture-surface coordinate contracts.

use crate::surface_coordinates::{
    map_rdevin_input, unavailable_window_rdevin_input, CaptureSurface,
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
fn nonzero_origin_and_scaled_monitor_map_global_input_to_frame_pixels() {
    let mut clicks = [
        click(10, -960.0, 540.0, true),
        click(11, -960.0, 540.0, false),
    ];
    let output = map_rdevin_input(
        CaptureSurface::new(-1920.0, 0.0, 1920.0, 1080.0),
        3840,
        2160,
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
    assert_eq!((clicks[0].x, clicks[0].y), (1920.0, 1080.0));
    assert_eq!((output.cursor[0].x, output.cursor[0].y), (1920.0, 1080.0));
    assert_eq!((output.scrolls[0].x, output.scrolls[0].y), (1920.0, 1080.0));
    assert_eq!(output.correlation.state, CursorCoordinateState::Exact);
    assert_eq!(
        output.correlation.exact_clicks, 2,
        "press and release both count"
    );
}

#[test]
fn scaled_window_frame_maps_global_input_to_the_recorded_window() {
    let mut clicks = [click(10, 500.0, 500.0, true)];
    let output = map_rdevin_input(
        CaptureSurface::new(100.0, 200.0, 800.0, 600.0),
        1600,
        1200,
        vec![],
        &mut clicks,
        vec![],
    );
    assert_eq!((clicks[0].x, clicks[0].y), (800.0, 600.0));
    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
    assert_eq!(output.correlation.exact_clicks, 1);
}

#[test]
fn right_bottom_boundary_and_outside_window_are_unavailable() {
    let mut clicks = [
        click(10, 900.0, 800.0, true),
        click(11, 901.0, 801.0, false),
    ];
    let output = map_rdevin_input(
        CaptureSurface::new(100.0, 200.0, 800.0, 600.0),
        1600,
        1200,
        vec![CursorSample {
            t_ms: 9,
            x: 900.0,
            y: 800.0,
        }],
        &mut clicks,
        vec![ScrollSample {
            t_ms: 12,
            x: 901.0,
            y: 801.0,
            dx: 0.0,
            dy: -1.0,
        }],
    );
    assert!(output.cursor.is_empty());
    assert!(output.scrolls.is_empty());
    assert!(clicks
        .iter()
        .all(|click| click.position_quality == ClickPositionQuality::Unavailable));
    assert_eq!(output.correlation.state, CursorCoordinateState::Unavailable);
    assert_eq!(
        output.correlation.unavailable_clicks, 2,
        "press and release both count"
    );
}

#[test]
fn mixed_window_transitions_report_exact_and_unavailable_counts() {
    let mut clicks = [
        click(10, 500.0, 500.0, true),
        click(11, 900.0, 800.0, false),
    ];
    let output = map_rdevin_input(
        CaptureSurface::new(100.0, 200.0, 800.0, 600.0),
        1600,
        1200,
        vec![],
        &mut clicks,
        vec![],
    );
    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
    assert_eq!(
        clicks[1].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert_eq!(output.correlation.state, CursorCoordinateState::Approximate);
    assert_eq!(output.correlation.exact_clicks, 1);
    assert_eq!(output.correlation.unavailable_clicks, 1);
}

#[test]
fn absent_surface_never_promotes_global_coordinates_to_exact() {
    let mut clicks = [click(10, 50.0, 50.0, true)];
    let output = map_rdevin_input(None, 1920, 1080, vec![], &mut clicks, vec![]);
    assert_eq!(
        clicks[0].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert_eq!(output.correlation.state, CursorCoordinateState::Unavailable);
    assert_eq!(output.correlation.unavailable_clicks, 1);
}

#[test]
fn unknown_rdevin_position_is_not_promoted_by_a_valid_surface() {
    let mut clicks = [ClickSample {
        position_quality: ClickPositionQuality::Unavailable,
        ..click(10, 500.0, 500.0, true)
    }];
    let output = map_rdevin_input(
        CaptureSurface::new(100.0, 200.0, 800.0, 600.0),
        1600,
        1200,
        vec![],
        &mut clicks,
        vec![],
    );
    assert_eq!(
        clicks[0].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert_eq!(output.correlation.exact_clicks, 0);
    assert_eq!(output.correlation.unavailable_clicks, 1);
    assert!(output
        .correlation
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("before an absolute cursor position")));
}

#[test]
fn windows_and_macos_window_move_resize_and_stale_input_is_unavailable() {
    // These points model an initial window location, a later move, and a resize
    // after the startup rectangle is stale. Native window capture currently has
    // no capture-clock rectangle samples, so none may be mapped as frame pixels.
    let mut clicks = [
        click(10, 500.0, 500.0, true),
        click(150, 1300.0, 600.0, false),
        click(310, 1700.0, 900.0, true),
    ];
    let output = unavailable_window_rdevin_input(
        vec![
            CursorSample {
                t_ms: 10,
                x: 500.0,
                y: 500.0,
            },
            CursorSample {
                t_ms: 150,
                x: 1300.0,
                y: 600.0,
            },
            CursorSample {
                t_ms: 310,
                x: 1700.0,
                y: 900.0,
            },
        ],
        &mut clicks,
        vec![ScrollSample {
            t_ms: 310,
            x: 1700.0,
            y: 900.0,
            dx: 0.0,
            dy: -1.0,
        }],
    );
    assert!(clicks
        .iter()
        .all(|click| click.position_quality == ClickPositionQuality::Unavailable));
    assert!(output.cursor.is_empty());
    assert!(output.scrolls.is_empty());
    assert_eq!(output.correlation.state, CursorCoordinateState::Unavailable);
    assert_eq!(output.correlation.unavailable_clicks, 3);
    assert!(output
        .correlation
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("timestamped capture-surface geometry")));
}
