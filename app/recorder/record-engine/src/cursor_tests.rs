use super::{autoedit, smooth_cursor, EngineConfig};
use record_core::{fixtures, ClickPositionQuality, ClickSample, CursorSample, MouseButton};

fn cfg() -> EngineConfig {
    EngineConfig::default()
}

#[test]
fn sparse_native_clicks_keep_the_stationary_seed_and_exact_button_anchors() {
    let mut events = fixtures::generate("click-walkthrough").unwrap();
    events.screen_w = 3840;
    events.screen_h = 2160;
    events.duration_ms = 8935;
    events.cursor = vec![
        CursorSample {
            t_ms: 18,
            x: 1395.2,
            y: 1284.6,
        },
        CursorSample {
            t_ms: 2420,
            x: 720.0,
            y: 1233.6,
        },
        CursorSample {
            t_ms: 2549,
            x: 720.0,
            y: 1233.6,
        },
        CursorSample {
            t_ms: 5403,
            x: 1680.0,
            y: 1462.4,
        },
        CursorSample {
            t_ms: 5536,
            x: 1680.0,
            y: 1462.4,
        },
    ];
    events.clicks = [(2420, 720.0, 1233.6), (5403, 1680.0, 1462.4)]
        .into_iter()
        .map(|(t_ms, x, y)| ClickSample {
            t_ms,
            x,
            y,
            button: MouseButton::Left,
            down: true,
            position_quality: ClickPositionQuality::Exact,
        })
        .collect();
    let plan = autoedit(&events, &cfg());
    assert_eq!(plan.cursor.smoothed, events.cursor);
    assert_eq!(plan.clicks.len(), 2);
    for click in &events.clicks {
        let point = plan
            .cursor
            .smoothed
            .iter()
            .find(|point| point.t_ms == click.t_ms)
            .unwrap();
        assert_eq!((point.x, point.y), (click.x, click.y));
    }
}

#[test]
fn dense_motion_is_smoothed_but_an_exact_click_between_samples_is_pinned() {
    let mut events = fixtures::generate("click-walkthrough").unwrap();
    events.cursor = vec![
        CursorSample {
            t_ms: 0,
            x: 100.0,
            y: 100.0,
        },
        CursorSample {
            t_ms: 16,
            x: 110.0,
            y: 98.0,
        },
        CursorSample {
            t_ms: 32,
            x: 90.0,
            y: 102.0,
        },
        CursorSample {
            t_ms: 48,
            x: 110.0,
            y: 98.0,
        },
        CursorSample {
            t_ms: 64,
            x: 100.0,
            y: 100.0,
        },
    ];
    events.clicks = vec![ClickSample {
        t_ms: 40,
        x: 101.0,
        y: 99.0,
        button: MouseButton::Left,
        down: true,
        position_quality: ClickPositionQuality::Exact,
    }];
    let plan = autoedit(&events, &cfg());
    assert_eq!(plan.cursor.smoothed[0], events.cursor[0]);
    assert_eq!(
        plan.cursor.smoothed.last().copied(),
        events.cursor.last().copied()
    );
    assert!((plan.cursor.smoothed[2].x - 90.0).abs() > 1.0);
    let click = plan
        .cursor
        .smoothed
        .iter()
        .find(|point| point.t_ms == 40)
        .unwrap();
    assert_eq!((click.x, click.y), (101.0, 99.0));
}

#[test]
fn dense_segment_endpoint_before_long_gap_keeps_its_observed_coordinate() {
    let cursor = [
        CursorSample {
            t_ms: 0,
            x: 100.0,
            y: 100.0,
        },
        CursorSample {
            t_ms: 16,
            x: 110.0,
            y: 98.0,
        },
        CursorSample {
            t_ms: 32,
            x: 90.0,
            y: 102.0,
        },
        CursorSample {
            t_ms: 2000,
            x: 400.0,
            y: 300.0,
        },
        CursorSample {
            t_ms: 2016,
            x: 410.0,
            y: 302.0,
        },
        CursorSample {
            t_ms: 2032,
            x: 390.0,
            y: 298.0,
        },
    ];
    let smoothed = smooth_cursor(&cursor, 5);
    assert_eq!(smoothed[2], cursor[2]);
    assert_eq!(smoothed[3], cursor[3]);
    assert_ne!(smoothed[1], cursor[1]);
    assert_ne!(smoothed[4], cursor[4]);
}

#[test]
fn exact_click_replaces_final_duplicate_timestamp_used_by_renderer() {
    let mut events = fixtures::generate("click-walkthrough").unwrap();
    events.cursor = vec![
        CursorSample {
            t_ms: 0,
            x: 10.0,
            y: 10.0,
        },
        CursorSample {
            t_ms: 40,
            x: 20.0,
            y: 20.0,
        },
        CursorSample {
            t_ms: 40,
            x: 21.0,
            y: 21.0,
        },
        CursorSample {
            t_ms: 80,
            x: 30.0,
            y: 30.0,
        },
    ];
    events.clicks = vec![ClickSample {
        t_ms: 40,
        x: 22.0,
        y: 23.0,
        button: MouseButton::Left,
        down: true,
        position_quality: ClickPositionQuality::Exact,
    }];
    let path = autoedit(&events, &cfg()).cursor.smoothed;
    assert_eq!(path.len(), events.cursor.len());
    let last_equal = path.iter().rfind(|point| point.t_ms == 40).unwrap();
    assert_eq!((last_equal.x, last_equal.y), (22.0, 23.0));
}

#[test]
fn approximate_and_unavailable_clicks_do_not_anchor_cursor() {
    let mut events = fixtures::generate("click-walkthrough").unwrap();
    events.cursor = vec![
        CursorSample {
            t_ms: 0,
            x: 10.0,
            y: 10.0,
        },
        CursorSample {
            t_ms: 40,
            x: 20.0,
            y: 20.0,
        },
    ];
    events.clicks = [
        ClickPositionQuality::Approximate,
        ClickPositionQuality::Unavailable,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, position_quality)| ClickSample {
        t_ms: 10 + i as u64 * 10,
        x: 999.0,
        y: 999.0,
        button: MouseButton::Left,
        down: true,
        position_quality,
    })
    .collect();
    assert_eq!(autoedit(&events, &cfg()).cursor.smoothed, events.cursor);
}

#[test]
fn clicks_without_a_cursor_path_do_not_create_a_synthetic_pointer() {
    let mut events = fixtures::generate("click-walkthrough").unwrap();
    events.cursor.clear();
    assert!(autoedit(&events, &cfg()).cursor.smoothed.is_empty());
}
