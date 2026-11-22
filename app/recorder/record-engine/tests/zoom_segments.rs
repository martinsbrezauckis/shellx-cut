use record_core::zoom_segments::{keys_to_segments, segments_to_keys};
use record_core::{fixtures, EditPlan};
use record_engine::{build_zoom, EngineConfig};

#[test]
fn automatic_zoom_segments_roundtrip_exact_keys() {
    for fixture in ["click-walkthrough", "scroll-read"] {
        let original = fixtures::generate(fixture).unwrap();
        for count in 0..=original.clicks.len() {
            let mut events = original.clone();
            events.clicks.truncate(count);
            for (gap, scale) in [
                (0, 1.0),
                (2500, 1.0),
                (100_000, 1.0),
                (0, 2.0),
                (2500, 2.0),
                (100_000, 8.0),
            ] {
                let cfg = EngineConfig {
                    stay_zoomed_gap_ms: gap,
                    max_zoom: scale,
                    ..Default::default()
                };
                let keys = build_zoom(&events, &cfg);
                let segments = keys_to_segments(&keys).unwrap();
                let rebuilt = segments_to_keys(&segments, events.duration_ms).unwrap();
                assert_eq!(keys, rebuilt, "{fixture}, gap={gap}, count={count}");
            }
        }
    }
}

#[test]
fn manual_segments_stay_zoomed_and_pan_across_short_gaps() {
    use record_core::zoom_segments::{ZoomOrigin, ZoomSegment};
    let segments = vec![
        ZoomSegment {
            id: "z1".into(),
            start_ms: 1000,
            end_ms: 3000,
            scale: 2.0,
            cx: 0.25,
            cy: 0.5,
            ramp_in_ms: 550,
            ramp_out_ms: 500,
            origin: ZoomOrigin::Manual,
        },
        ZoomSegment {
            id: "z2".into(),
            start_ms: 4000,
            end_ms: 6000,
            scale: 2.0,
            cx: 0.75,
            cy: 0.5,
            ramp_in_ms: 550,
            ramp_out_ms: 500,
            origin: ZoomOrigin::Manual,
        },
    ];
    let keys = segments_to_keys(&segments, 8000).unwrap();
    let (scale, cx, _) = keys.eval(3500);
    assert_eq!(scale, 2.0);
    assert!(cx > 0.25 && cx < 0.75);
}

#[test]
fn plan_rejects_invalid_zoom_keys() {
    let mut plan = EditPlan::empty(1920, 1080, 20_000, 30.0);
    for (scale, cx, cy) in [
        (0.5, 0.5, 0.5),
        (9.0, 0.5, 0.5),
        (2.0, -0.1, 0.5),
        (2.0, 0.5, f64::NAN),
    ] {
        plan.zoom.keys = vec![record_core::ZoomKey {
            t_ms: 0,
            scale,
            cx,
            cy,
            ease: Default::default(),
        }];
        assert!(plan.validate().is_err());
    }
    plan.zoom.keys = vec![
        record_core::ZoomKey {
            t_ms: 20,
            scale: 2.0,
            cx: 0.5,
            cy: 0.5,
            ease: Default::default()
        };
        2
    ];
    assert!(plan.validate().is_err());
}
