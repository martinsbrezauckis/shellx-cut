use record_core::{
    CaptureCadence, CursorCoordinateSource, CursorCoordinateState, CursorCorrelation, EventTrack,
    FrameRate, Monitor, Settings,
};
use record_recovery::{
    CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournal, RecordingSessionState, RecordingStream, SealedRun, SessionTerminal,
    StreamFragment, StreamFragmentFacts, TerminalDisposition,
};

use super::super::pause_projection::SealedLegacyProjectionRun;
use super::plan_frame_grid_legacy_root_projection;

fn settings(fps: f32) -> Settings {
    Settings {
        width: 640,
        height: 360,
        fps,
        audio_rate: 48_000,
    }
}

fn events(duration_ms: u64) -> EventTrack {
    EventTrack {
        duration_ms,
        screen_w: 640,
        screen_h: 360,
        monitors: vec![Monitor {
            id: 1,
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
        cursor_correlation: CursorCorrelation {
            source: CursorCoordinateSource::RdevinAbsolute,
            state: CursorCoordinateState::Exact,
            exact_clicks: 0,
            approximate_clicks: 0,
            unavailable_clicks: 0,
            max_metadata_age_ms: Some(16),
            detail: Some("frame-grid fixture".into()),
        },
    }
}

fn transition(
    sequence: u64,
    state: RecordingSessionState,
    logical_offset_ms: u64,
    observed_unix_ms: u64,
) -> DurableStateTransition {
    DurableStateTransition {
        sequence,
        state,
        logical_offset_ms,
        observed_unix_ms,
    }
}

fn fragment(
    checkpoint: u64,
    avg_frame_rate: Option<FrameRate>,
    r_frame_rate: Option<FrameRate>,
    decoded_video_frames: u64,
) -> StreamFragment {
    StreamFragment {
        stream: RecordingStream::ScreenVideo,
        checkpoint_sequence: Some(checkpoint),
        stream_sequence: 0,
        artifact: format!("screen/run-{checkpoint}.mp4"),
        bytes: 1,
        sha256: "a".repeat(64),
        facts: StreamFragmentFacts {
            start_offset_ms: 0,
            end_offset_ms: 100,
            media_duration_ms: 100,
            decoded_video_frames: Some(decoded_video_frames),
            avg_frame_rate,
            r_frame_rate,
        },
    }
}

fn run(
    sequence: u64,
    start_ms: u64,
    checkpoint: u64,
    avg_frame_rate: Option<FrameRate>,
    r_frame_rate: Option<FrameRate>,
    decoded_video_frames: u64,
) -> SealedRun {
    SealedRun {
        sequence,
        observed_start_ms: start_ms + 10,
        observed_end_ms: start_ms + 110,
        logical_start_ms: start_ms,
        logical_end_ms: start_ms + 100,
        checkpoints: CheckpointSequenceRange {
            first: checkpoint,
            last: checkpoint,
        },
        fragments: vec![fragment(
            checkpoint,
            avg_frame_rate,
            r_frame_rate,
            decoded_video_frames,
        )],
    }
}

fn fixture(
    cadence: Option<CaptureCadence>,
    first_rate: (Option<FrameRate>, Option<FrameRate>),
    second_rate: (Option<FrameRate>, Option<FrameRate>),
    first_frames: u64,
    second_frames: u64,
) -> RecordingSessionJournal {
    let mut intent = RecordingSessionIntent::new(
        "frame-grid-pause",
        1,
        100,
        30.0,
        None,
        false,
        "opaque-target",
        vec![RecordingStream::ScreenVideo],
    );
    if let Some(cadence) = cadence {
        intent = intent.with_capture_cadence(cadence);
    }
    let mut journal = RecordingSessionJournal::new(intent).unwrap();
    journal
        .append_transition(transition(0, RecordingSessionState::Started, 0, 1))
        .unwrap();
    journal
        .seal_run(run(0, 0, 0, first_rate.0, first_rate.1, first_frames))
        .unwrap();
    journal
        .append_transition(transition(1, RecordingSessionState::Paused, 100, 200))
        .unwrap();
    journal
        .append_transition(transition(2, RecordingSessionState::Resumed, 100, 1_200))
        .unwrap();
    journal
        .seal_run(run(1, 100, 1, second_rate.0, second_rate.1, second_frames))
        .unwrap();
    journal
        .append_transition(transition(3, RecordingSessionState::Stopping, 200, 1_300))
        .unwrap();
    journal
        .seal_terminal(SessionTerminal {
            disposition: TerminalDisposition::Completed,
            logical_end_ms: 200,
            observed_unix_ms: 1_301,
        })
        .unwrap();
    journal
}

fn inputs(fps: f32) -> Vec<SealedLegacyProjectionRun> {
    vec![
        SealedLegacyProjectionRun::new(0, 0, 100, settings(fps), events(100)),
        SealedLegacyProjectionRun::new(1, 100, 200, settings(fps), events(100)),
    ]
}

fn rate() -> FrameRate {
    FrameRate::new(30, 1).unwrap()
}

#[test]
fn strict_grid_carries_output_policy_frame_count_and_exact_duration() {
    let journal = fixture(
        Some(CaptureCadence::from_server_fps(30.0).unwrap()),
        (Some(rate()), Some(rate())),
        (Some(rate()), Some(rate())),
        3,
        3,
    );
    let qualified = plan_frame_grid_legacy_root_projection(&journal, &inputs(30.0)).unwrap();
    let grid = qualified.source_grid();
    assert_eq!(grid.output_frame_rate, rate());
    assert_eq!(grid.output_frame_count, 6);
    assert_eq!(grid.expected_duration_ms.numerator, 200);
    assert_eq!(grid.expected_duration_ms.denominator, 1);
    assert_eq!(qualified.legacy().merged_events().events.duration_ms, 200);
}

#[test]
fn strict_grid_rejects_missing_fragment_probe_cadence() {
    let cadence = Some(CaptureCadence::from_server_fps(30.0).unwrap());
    let cases = [
        ((None, None), (Some(rate()), Some(rate()))),
        ((Some(rate()), None), (Some(rate()), Some(rate()))),
    ];
    for (first_rate, second_rate) in cases {
        let journal = fixture(cadence.clone(), first_rate, second_rate, 3, 3);
        assert!(plan_frame_grid_legacy_root_projection(&journal, &inputs(30.0)).is_err());
    }
}

#[test]
fn strict_grid_requires_policy_and_accepts_independent_source_count() {
    let matching = (Some(rate()), Some(rate()));
    let no_cadence = fixture(None, matching, matching, 3, 3);
    assert!(plan_frame_grid_legacy_root_projection(&no_cadence, &inputs(30.0)).is_err());

    let independent_frames = fixture(
        Some(CaptureCadence::from_server_fps(30.0).unwrap()),
        matching,
        matching,
        2,
        3,
    );
    assert!(plan_frame_grid_legacy_root_projection(&independent_frames, &inputs(30.0)).is_ok());

    let policy_drift = fixture(
        Some(CaptureCadence::from_server_fps(30.0).unwrap()),
        matching,
        matching,
        3,
        3,
    );
    assert!(plan_frame_grid_legacy_root_projection(&policy_drift, &inputs(25.0)).is_err());
}

#[test]
fn native_vfr_run_gets_five_source_and_five_explicit_gap_frames() {
    let intent = RecordingSessionIntent::new(
        "native-vfr-fixture",
        1,
        15000,
        24.0,
        None,
        false,
        "opaque-target",
        vec![RecordingStream::ScreenVideo],
    )
    .with_capture_cadence(CaptureCadence::from_server_fps(24.0).unwrap());
    let mut journal = RecordingSessionJournal::new(intent).unwrap();
    journal
        .append_transition(transition(0, RecordingSessionState::Started, 0, 1))
        .unwrap();
    // Native source facts; synthetic events do not recover original UI evidence.
    let mut sealed = run(
        0,
        0,
        0,
        Some(FrameRate::new(150, 7).unwrap()),
        Some(FrameRate::new(60, 1).unwrap()),
        5,
    );
    sealed.observed_start_ms = 0;
    sealed.observed_end_ms = 715;
    sealed.logical_end_ms = 425;
    sealed.fragments[0].facts.end_offset_ms = 425;
    sealed.fragments[0].facts.media_duration_ms = 200;
    journal.seal_run(sealed).unwrap();
    journal
        .append_transition(transition(1, RecordingSessionState::Paused, 425, 716))
        .unwrap();
    journal
        .append_transition(transition(2, RecordingSessionState::Stopping, 425, 861))
        .unwrap();
    journal
        .seal_terminal(SessionTerminal {
            disposition: TerminalDisposition::Completed,
            logical_end_ms: 425,
            observed_unix_ms: 862,
        })
        .unwrap();
    let input = SealedLegacyProjectionRun::new(0, 0, 425, settings(24.0), events(425));
    let qualified = plan_frame_grid_legacy_root_projection(&journal, &[input]).unwrap();
    assert_eq!(
        qualified.source_grid().output_frame_rate,
        FrameRate::new(24, 1).unwrap()
    );
    assert_eq!(qualified.source_grid().output_frame_count, 10);
    assert_eq!(qualified.source_grid().spans.len(), 2);
    assert_eq!(qualified.source_grid().spans[0].frame_count(), 5);
    assert_eq!(qualified.source_grid().spans[1].frame_count(), 5);
    assert!(matches!(
        qualified.source_grid().spans[1].span,
        record_recovery::RunAwareStitchSpan::EncoderGapPadding { .. }
    ));
}
