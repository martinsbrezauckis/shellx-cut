use super::*;

fn native_quiet_capture() -> PipewireCursorCapture {
    PipewireCursorCapture {
        metadata: vec![
            CursorMetadataSample {
                t_ms: 222,
                x: 2511.0,
                y: 791.0,
            },
            CursorMetadataSample {
                t_ms: 328,
                x: 2515.0,
                y: 791.0,
            },
            CursorMetadataSample {
                t_ms: 418,
                x: 2522.0,
                y: 791.0,
            },
        ],
        invalidations: vec![],
        segments: vec![CaptureSegment {
            start_ms: 203,
            end_ms: 972,
        }],
        frame_width: 3840,
        frame_height: 2160,
        capture_start_ms: 203,
        capture_end_ms: 972,
    }
}

fn native_evidence() -> EvdevEvidence {
    EvdevEvidence {
        complete: true,
        motion: vec![
            ObservedMotion {
                observed_ms: 311,
                kernel_time: kernel_ms(311),
            },
            ObservedMotion {
                observed_ms: 412,
                kernel_time: kernel_ms(412),
            },
        ],
        button_times: vec![kernel_ms(810), kernel_ms(931)],
    }
}

fn native_clicks(
    capture: PipewireCursorCapture,
    evidence: &EvdevEvidence,
) -> (Vec<ClickSample>, CorrelationOutput) {
    let mut clicks = vec![click(810), click(931)];
    clicks[1].down = false;
    let output = correlate_clicks(
        wayland_mode(),
        &mut clicks,
        vec![],
        vec![],
        Some(capture),
        PortalCursorGeometry::from_portal(Some((0, 0)), Some((3072, 1728))),
        None,
        Some(evidence),
    );
    (clicks, output)
}

#[test]
fn native_418_to_810ms_quiet_click_uses_distinct_validated_held_quality() {
    let (clicks, output) = native_clicks(native_quiet_capture(), &native_evidence());
    assert_eq!(
        clicks
            .iter()
            .map(|click| click.position_quality)
            .collect::<Vec<_>>(),
        [ClickPositionQuality::Held, ClickPositionQuality::Held]
    );
    assert!(clicks
        .iter()
        .all(|click| (click.x, click.y) == (2522.0, 791.0)));
    assert_eq!(output.status.state, CursorCoordinateState::Held);
    assert_eq!(output.status.held_clicks, 2);
    assert_eq!(output.status.exact_clicks, 0);
}

#[test]
fn first_frame_cursor_sample_at_segment_origin_remains_eligible() {
    let mut capture = native_quiet_capture();
    capture.metadata.retain(|sample| sample.t_ms >= 418);
    capture.segments = vec![CaptureSegment {
        start_ms: 418,
        end_ms: 972,
    }];
    let (clicks, output) = native_clicks(capture, &native_evidence());
    assert!(clicks
        .iter()
        .all(|click| click.position_quality == ClickPositionQuality::Held));
    assert_eq!(output.status.held_clicks, 2);
}

#[test]
fn held_position_refuses_motion_hidden_outside_lost_reader_or_segment_change() {
    let mut cases = vec![];
    let mut moved = native_evidence();
    moved.motion.push(ObservedMotion {
        observed_ms: 600,
        kernel_time: kernel_ms(600),
    });
    cases.push((native_quiet_capture(), moved));
    let mut late_read_motion = native_evidence();
    late_read_motion.motion.push(ObservedMotion {
        observed_ms: 950,
        kernel_time: kernel_ms(600),
    });
    cases.push((native_quiet_capture(), late_read_motion));
    let mut hidden = native_quiet_capture();
    hidden.invalidations.push(600);
    cases.push((hidden, native_evidence()));
    let mut outside = native_quiet_capture();
    outside.invalidations.push(600);
    cases.push((outside, native_evidence()));
    let mut tied_invalidation = native_quiet_capture();
    tied_invalidation.invalidations.push(418);
    cases.push((tied_invalidation, native_evidence()));
    let mut lost_reader = native_evidence();
    lost_reader.complete = false;
    cases.push((native_quiet_capture(), lost_reader));
    let mut rotated = native_quiet_capture();
    rotated.segments = vec![
        CaptureSegment {
            start_ms: 203,
            end_ms: 600,
        },
        CaptureSegment {
            start_ms: 700,
            end_ms: 972,
        },
    ];
    cases.push((rotated, native_evidence()));
    for (capture, evidence) in cases {
        let (clicks, output) = native_clicks(capture, &evidence);
        assert!(clicks
            .iter()
            .all(|click| click.position_quality == ClickPositionQuality::Approximate));
        assert_eq!(output.status.held_clicks, 0);
    }
}
