use crate::{
    merge_sealed_event_tracks, ClickPositionQuality, ClickSample, CursorCoordinateSource,
    CursorCorrelation, CursorSample, EventTrack, KeySample, Monitor, MouseButton, ScrollSample,
    SealedEventTrackRun, Settings,
};

fn settings() -> Settings {
    Settings {
        width: 640,
        height: 360,
        fps: 50.0,
        audio_rate: 48_000,
    }
}

fn correlation(exact_clicks: u32) -> CursorCorrelation {
    CursorCorrelation {
        source: CursorCoordinateSource::RdevinAbsolute,
        state: crate::CursorCoordinateState::Exact,
        exact_clicks,
        approximate_clicks: 0,
        unavailable_clicks: 0,
        max_metadata_age_ms: Some(12),
        detail: Some("captured surface coordinates".into()),
    }
}

fn events(duration_ms: u64) -> EventTrack {
    EventTrack {
        duration_ms,
        screen_w: 640,
        screen_h: 360,
        monitors: vec![Monitor {
            id: 7,
            x: 0,
            y: 0,
            w: 640,
            h: 360,
            primary: true,
        }],
        cursor: vec![],
        clicks: vec![],
        scrolls: vec![],
        keys: vec![],
        cursor_correlation: correlation(0),
    }
}

fn run(logical_offset_ms: u64, events: EventTrack) -> SealedEventTrackRun {
    SealedEventTrackRun {
        logical_offset_ms,
        settings: settings(),
        events,
    }
}

#[test]
fn compacts_two_sealed_runs_without_emitting_wall_pause_samples() {
    let mut first = events(100);
    first.cursor.push(CursorSample {
        t_ms: 99,
        x: 10.0,
        y: 20.0,
    });
    let mut second = events(50);
    second.cursor.push(CursorSample {
        t_ms: 0,
        x: 30.0,
        y: 40.0,
    });

    // The source workers may have been paused for hours between these runs. The
    // supplied compact resume offset deliberately excludes that wall-clock gap.
    let merged = merge_sealed_event_tracks(&[run(0, first), run(100, second)]).unwrap();

    assert_eq!(merged.events.duration_ms, 150);
    assert_eq!(
        merged
            .events
            .cursor
            .iter()
            .map(|sample| sample.t_ms)
            .collect::<Vec<_>>(),
        vec![99, 100]
    );
    assert!(
        merged.events.cursor.iter().all(|sample| sample.t_ms <= 100),
        "the merger must not invent cursor samples for a wall-clock pause"
    );
}

#[test]
fn offsets_every_event_kind_and_preserves_half_open_edges() {
    let mut first = events(20);
    first.cursor = vec![
        CursorSample {
            t_ms: 0,
            x: 0.0,
            y: 0.0,
        },
        CursorSample {
            t_ms: 19,
            x: 639.0,
            y: 359.0,
        },
    ];
    first.clicks = vec![ClickSample {
        t_ms: 19,
        x: 639.0,
        y: 359.0,
        button: MouseButton::Left,
        down: true,
        position_quality: ClickPositionQuality::Exact,
    }];
    first.scrolls = vec![ScrollSample {
        t_ms: 0,
        x: 1.0,
        y: 2.0,
        dx: 0.5,
        dy: -1.0,
    }];
    first.keys = vec![KeySample {
        t_ms: 19,
        key: "Enter".into(),
        down: true,
    }];
    first.cursor_correlation = correlation(1);

    let mut second = events(10);
    second.cursor.push(CursorSample {
        t_ms: 0,
        x: 20.0,
        y: 30.0,
    });
    second.clicks.push(ClickSample {
        t_ms: 9,
        x: 20.0,
        y: 30.0,
        button: MouseButton::Right,
        down: false,
        position_quality: ClickPositionQuality::Exact,
    });
    second.scrolls.push(ScrollSample {
        t_ms: 9,
        x: 20.0,
        y: 30.0,
        dx: -2.0,
        dy: 3.0,
    });
    second.keys.push(KeySample {
        t_ms: 0,
        key: "A".into(),
        down: false,
    });
    second.cursor_correlation = correlation(1);

    let merged = merge_sealed_event_tracks(&[run(0, first), run(20, second)]).unwrap();

    assert_eq!(
        merged
            .events
            .cursor
            .iter()
            .map(|sample| sample.t_ms)
            .collect::<Vec<_>>(),
        vec![0, 19, 20]
    );
    assert_eq!(
        merged
            .events
            .clicks
            .iter()
            .map(|sample| sample.t_ms)
            .collect::<Vec<_>>(),
        vec![19, 29]
    );
    assert_eq!(
        merged
            .events
            .scrolls
            .iter()
            .map(|sample| sample.t_ms)
            .collect::<Vec<_>>(),
        vec![0, 29]
    );
    assert_eq!(
        merged
            .events
            .keys
            .iter()
            .map(|sample| sample.t_ms)
            .collect::<Vec<_>>(),
        vec![19, 20]
    );
    assert_eq!(merged.events.cursor_correlation.exact_clicks, 2);
    assert_eq!(merged.events.duration_ms, 30);
}

#[test]
fn rejects_a_logical_gap_that_would_retain_wall_pause_time() {
    let mut first = events(10);
    first.keys.push(KeySample {
        t_ms: 9,
        key: "A".into(),
        down: true,
    });
    let mut second = events(10);
    second.keys.push(KeySample {
        t_ms: 0,
        key: "B".into(),
        down: true,
    });

    assert!(merge_sealed_event_tracks(&[run(0, first), run(200, second)]).is_err());
}

#[test]
fn rejects_contract_mismatch_and_geometry_drift() {
    let first = run(0, events(10));
    let mut mismatched = run(10, events(10));
    mismatched.settings.fps = 25.0;
    assert!(merge_sealed_event_tracks(&[first.clone(), mismatched]).is_err());

    let mut monitor_drift = run(10, events(10));
    monitor_drift.events.monitors[0].x = 1;
    assert!(merge_sealed_event_tracks(&[first.clone(), monitor_drift]).is_err());

    let mut correlation_drift = run(10, events(10));
    correlation_drift
        .events
        .cursor_correlation
        .max_metadata_age_ms = Some(13);
    assert!(merge_sealed_event_tracks(&[first, correlation_drift]).is_err());
}

#[test]
fn rejects_out_of_range_or_unordered_samples() {
    assert!(merge_sealed_event_tracks(&[run(0, events(0))]).is_err());

    let mut out_of_range = events(10);
    out_of_range.cursor.push(CursorSample {
        t_ms: 10,
        x: 1.0,
        y: 1.0,
    });
    assert!(merge_sealed_event_tracks(&[run(0, out_of_range)]).is_err());

    let mut geometry = events(10);
    geometry.clicks.push(ClickSample {
        t_ms: 1,
        x: 640.0,
        y: 1.0,
        button: MouseButton::Middle,
        down: true,
        position_quality: ClickPositionQuality::Exact,
    });
    assert!(merge_sealed_event_tracks(&[run(0, geometry)]).is_err());

    let mut unordered = events(10);
    unordered.keys = vec![
        KeySample {
            t_ms: 5,
            key: "A".into(),
            down: true,
        },
        KeySample {
            t_ms: 4,
            key: "A".into(),
            down: false,
        },
    ];
    assert!(merge_sealed_event_tracks(&[run(0, unordered)]).is_err());
}

#[test]
fn rejects_run_ordering_and_checked_timestamp_overflow() {
    let first = run(0, events(10));
    let overlapping = run(9, events(10));
    assert!(merge_sealed_event_tracks(&[first.clone(), overlapping]).is_err());

    let overflowing = run(u64::MAX - 1, events(2));
    assert!(merge_sealed_event_tracks(&[first, overflowing]).is_err());
}
