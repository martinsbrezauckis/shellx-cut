//! Deterministic Wayland cursor correlation contract tests.

use crate::cursor_correlation::*;
use record_core::{
    ClickPositionQuality, ClickSample, CursorCoordinateSource, CursorCoordinateState, CursorSample,
    MouseButton, ScrollSample,
};

fn wayland_mode() -> SessionInputMode {
    session_input_mode_from(Some("wayland"), true, false, None, None)
}

fn click(t_ms: u64) -> ClickSample {
    ClickSample {
        t_ms,
        x: 11.0,
        y: 22.0,
        button: MouseButton::Left,
        down: true,
        position_quality: ClickPositionQuality::Approximate,
    }
}

fn geometry() -> PortalCursorGeometry {
    PortalCursorGeometry::from_portal(Some((0, 0)), Some((1000, 500))).unwrap()
}

fn capture(samples: Vec<CursorMetadataSample>) -> PipewireCursorCapture {
    PipewireCursorCapture {
        metadata: samples,
        frame_width: 1000,
        frame_height: 500,
        capture_start_ms: 0,
        capture_end_ms: 0,
    }
}

#[test]
fn exact_click_uses_nearest_fresh_absolute_metadata() {
    let mut clicks = [click(105)];
    let output = correlate_clicks(
        wayland_mode(),
        &mut clicks,
        vec![],
        vec![],
        Some(capture(vec![CursorMetadataSample {
            t_ms: 100,
            x: 400.0,
            y: 200.0,
        }])),
        Some(geometry()),
        None,
    );
    assert_eq!((clicks[0].x, clicks[0].y), (400.0, 200.0));
    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
    assert_eq!(output.status.state, CursorCoordinateState::Exact);
}

#[test]
fn late_out_of_order_metadata_selects_the_closest_clock_sample() {
    let mut clicks = [click(100)];
    correlate_clicks(
        wayland_mode(),
        &mut clicks,
        vec![],
        vec![],
        Some(capture(vec![
            CursorMetadataSample {
                t_ms: 160,
                x: 600.0,
                y: 200.0,
            },
            CursorMetadataSample {
                t_ms: 95,
                x: 95.0,
                y: 100.0,
            },
        ])),
        Some(geometry()),
        None,
    );
    assert_eq!((clicks[0].x, clicks[0].y), (95.0, 100.0));
    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
}

#[test]
fn stale_or_missing_metadata_stays_approximate() {
    let mut stale = [click(101)];
    let stale_output = correlate_clicks(
        wayland_mode(),
        &mut stale,
        vec![],
        vec![],
        Some(capture(vec![CursorMetadataSample {
            t_ms: 0,
            x: 400.0,
            y: 200.0,
        }])),
        Some(geometry()),
        None,
    );
    assert_eq!(stale[0].position_quality, ClickPositionQuality::Approximate);
    assert_eq!(
        stale_output.status.state,
        CursorCoordinateState::Approximate
    );

    let mut missing = [click(10)];
    let missing_output = correlate_clicks(
        wayland_mode(),
        &mut missing,
        vec![],
        vec![],
        None,
        None,
        None,
    );
    assert_eq!(
        missing[0].position_quality,
        ClickPositionQuality::Approximate
    );
    assert_eq!(
        missing_output.status.source,
        CursorCoordinateSource::WaylandEvdevRelative
    );
}

#[test]
fn fractional_scale_and_multimonitor_origin_map_to_frame_pixels() {
    let mut clicks = [click(50)];
    correlate_clicks(
        wayland_mode(),
        &mut clicks,
        vec![],
        vec![],
        Some(PipewireCursorCapture {
            metadata: vec![CursorMetadataSample {
                t_ms: 50,
                x: -800.0,
                y: 580.0,
            }],
            frame_width: 3840,
            frame_height: 2160,
            capture_start_ms: 0,
            capture_end_ms: 0,
        }),
        PortalCursorGeometry::from_portal(Some((-1600, 100)), Some((2560, 1440))),
        None,
    );
    assert_eq!((clicks[0].x, clicks[0].y), (1200.0, 720.0));
}

#[test]
fn clicks_before_first_or_after_last_metadata_do_not_clamp_endpoints() {
    let mut clicks = [click(0), click(202)];
    correlate_clicks(
        wayland_mode(),
        &mut clicks,
        vec![],
        vec![],
        Some(capture(vec![CursorMetadataSample {
            t_ms: 101,
            x: 400.0,
            y: 200.0,
        }])),
        Some(geometry()),
        None,
    );
    assert!(clicks
        .iter()
        .all(|click| click.position_quality == ClickPositionQuality::Approximate));
}

#[test]
fn x11_uses_finalized_video_dimensions_not_portal_logical_size() {
    let x11 = session_input_mode_from(Some("x11"), false, true, None, None);
    assert_eq!(
        x11,
        SessionInputMode {
            wayland: false,
            use_evdev: false,
            use_pipewire_metadata: false
        }
    );
    let mut clicks = [click(10)];
    clicks[0].position_quality = ClickPositionQuality::Exact;
    clicks[0].x = -960.0;
    clicks[0].y = 540.0;
    let output = correlate_clicks(
        x11,
        &mut clicks,
        vec![CursorSample {
            t_ms: 9,
            x: -960.0,
            y: 540.0,
        }],
        vec![ScrollSample {
            t_ms: 11,
            x: -960.0,
            y: 540.0,
            dx: 0.0,
            dy: -1.0,
        }],
        None,
        PortalCursorGeometry::from_portal(Some((-1920, 0)), Some((1920, 1080))),
        Some((3840, 2160)),
    );
    assert_eq!((clicks[0].x, clicks[0].y), (1920.0, 1080.0));
    assert_eq!((output.cursor[0].x, output.cursor[0].y), (1920.0, 1080.0));
    assert_eq!((output.scrolls[0].x, output.scrolls[0].y), (1920.0, 1080.0));
    assert_eq!(clicks[0].position_quality, ClickPositionQuality::Exact);
    assert_eq!(output.status.source, CursorCoordinateSource::RdevinAbsolute);
    assert_eq!(output.status.state, CursorCoordinateState::Exact);
}

#[test]
fn x11_unknown_finalized_dimensions_keep_global_input_unavailable() {
    let x11 = session_input_mode_from(Some("x11"), false, true, None, None);
    let mut clicks = [click(10)];
    clicks[0].position_quality = ClickPositionQuality::Exact;
    let output = correlate_clicks(
        x11,
        &mut clicks,
        vec![CursorSample {
            t_ms: 9,
            x: -960.0,
            y: 540.0,
        }],
        vec![ScrollSample {
            t_ms: 11,
            x: -960.0,
            y: 540.0,
            dx: 0.0,
            dy: -1.0,
        }],
        None,
        PortalCursorGeometry::from_portal(Some((-1920, 0)), Some((1920, 1080))),
        None,
    );
    assert_eq!(
        clicks[0].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert!(output.cursor.is_empty());
    assert!(output.scrolls.is_empty());
    assert_eq!(output.status.state, CursorCoordinateState::Unavailable);
    assert!(output
        .status
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("could not be validated")));
}

#[test]
fn x11_window_geometry_maps_global_input_to_the_negotiated_frame() {
    let x11 = session_input_mode_from(Some("x11"), false, true, None, None);
    let mut clicks = [click(10)];
    clicks[0].position_quality = ClickPositionQuality::Exact;
    clicks[0].x = 500.0;
    clicks[0].y = 500.0;
    let output = correlate_clicks(
        x11,
        &mut clicks,
        vec![],
        vec![],
        None,
        PortalCursorGeometry::from_portal(Some((100, 200)), Some((800, 600))),
        Some((1600, 1200)),
    );
    assert_eq!((clicks[0].x, clicks[0].y), (800.0, 600.0));
    assert_eq!(output.status.state, CursorCoordinateState::Exact);
}

#[test]
fn x11_absent_portal_geometry_makes_raw_input_unavailable() {
    let x11 = session_input_mode_from(Some("x11"), false, true, None, None);
    let mut clicks = [click(10)];
    clicks[0].position_quality = ClickPositionQuality::Exact;
    let output = correlate_clicks(
        x11,
        &mut clicks,
        vec![CursorSample {
            t_ms: 9,
            x: 500.0,
            y: 500.0,
        }],
        vec![ScrollSample {
            t_ms: 11,
            x: 500.0,
            y: 500.0,
            dx: 0.0,
            dy: -1.0,
        }],
        None,
        None,
        Some((1600, 1200)),
    );
    assert_eq!(
        clicks[0].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert!(output.cursor.is_empty());
    assert!(output.scrolls.is_empty());
    assert_eq!(output.status.state, CursorCoordinateState::Unavailable);
}

#[test]
fn no_click_transitions_never_claims_vacuous_exactness() {
    let x11 = session_input_mode_from(Some("x11"), false, true, None, None);
    let output = correlate_clicks(
        x11,
        &mut [],
        vec![CursorSample {
            t_ms: 9,
            x: 500.0,
            y: 500.0,
        }],
        vec![],
        None,
        PortalCursorGeometry::from_portal(Some((100, 200)), Some((800, 600))),
        Some((1600, 1200)),
    );
    assert_eq!(output.status.state, CursorCoordinateState::Unavailable);
    assert_eq!(output.status.exact_clicks, 0);
    assert_eq!(output.status.unavailable_clicks, 0);
    assert_eq!(
        output.cursor.len(),
        1,
        "cursor samples remain valid evidence"
    );
}
