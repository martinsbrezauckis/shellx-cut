use super::*;
use record_core::{
    CursorCoordinateSource, CursorCoordinateState, CursorCorrelation, EventTrack, Monitor,
};
use record_recovery::{
    CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournalEntry, RecordingSessionState, RecordingStream, SealedRun,
    SessionTerminal, StreamFragment, StreamFragmentFacts,
};

fn settings() -> Settings {
    settings_with_fps(30.0)
}

fn settings_with_fps(fps: f32) -> Settings {
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
            detail: Some("surface pixels".into()),
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

fn screen_fragment(
    checkpoint_sequence: u64,
    stream_sequence: u64,
    start_offset_ms: u64,
    end_offset_ms: u64,
    media_duration_ms: u64,
) -> StreamFragment {
    StreamFragment {
        stream: RecordingStream::ScreenVideo,
        checkpoint_sequence: Some(checkpoint_sequence),
        stream_sequence,
        artifact: format!("screen/run-{checkpoint_sequence}.mp4"),
        bytes: 1,
        sha256: "a".repeat(64),
        facts: StreamFragmentFacts {
            start_offset_ms,
            end_offset_ms,
            media_duration_ms,
            decoded_video_frames: Some(3),
            avg_frame_rate: None,
            r_frame_rate: None,
        },
    }
}

fn sealed_run(
    sequence: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    checkpoint: u64,
    fragment_end_ms: u64,
) -> SealedRun {
    SealedRun {
        sequence,
        observed_start_ms: logical_start_ms + 1,
        observed_end_ms: logical_end_ms + 1,
        logical_start_ms,
        logical_end_ms,
        checkpoints: CheckpointSequenceRange {
            first: checkpoint,
            last: checkpoint,
        },
        fragments: vec![screen_fragment(
            checkpoint,
            0,
            0,
            fragment_end_ms,
            fragment_end_ms,
        )],
    }
}

fn journal(disposition: Option<TerminalDisposition>) -> RecordingSessionJournal {
    journal_with_fps_and_cadence(disposition, 30.0, None)
}

fn journal_with_fps(disposition: Option<TerminalDisposition>, fps: f64) -> RecordingSessionJournal {
    journal_with_fps_and_cadence(disposition, fps, None)
}

fn journal_with_fps_and_cadence(
    disposition: Option<TerminalDisposition>,
    fps: f64,
    capture_cadence: Option<CaptureCadence>,
) -> RecordingSessionJournal {
    let mut intent = RecordingSessionIntent::new(
        "pause-projection",
        1,
        100,
        fps,
        None,
        false,
        "opaque-target",
        vec![RecordingStream::ScreenVideo],
    );
    if let Some(capture_cadence) = capture_cadence {
        intent = intent.with_capture_cadence(capture_cadence);
    }
    let mut journal = RecordingSessionJournal::new(intent).unwrap();
    journal
        .append_transition(transition(0, RecordingSessionState::Started, 0, 1))
        .unwrap();
    journal.seal_run(sealed_run(0, 0, 100, 0, 50)).unwrap();
    journal
        .append_transition(transition(1, RecordingSessionState::Paused, 100, 200))
        .unwrap();
    journal
        .append_transition(transition(2, RecordingSessionState::Resumed, 100, 9_200))
        .unwrap();
    journal.seal_run(sealed_run(1, 100, 200, 1, 100)).unwrap();
    if let Some(disposition) = disposition {
        journal
            .append_transition(transition(3, RecordingSessionState::Stopping, 200, 9_300))
            .unwrap();
        journal
            .seal_terminal(SessionTerminal {
                disposition,
                logical_end_ms: 200,
                observed_unix_ms: 9_400,
            })
            .unwrap();
    }
    journal
}

fn inputs() -> Vec<SealedLegacyProjectionRun> {
    inputs_with_fps(30.0)
}

fn inputs_with_fps(fps: f32) -> Vec<SealedLegacyProjectionRun> {
    vec![
        SealedLegacyProjectionRun::new(0, 0, 100, settings_with_fps(fps), events(100)),
        SealedLegacyProjectionRun::new(1, 100, 200, settings_with_fps(fps), events(100)),
    ]
}

#[test]
fn projects_completed_runs_to_fixed_legacy_outputs_without_wall_pause_time() {
    let plan =
        plan_legacy_root_projection(&journal(Some(TerminalDisposition::Completed)), &inputs())
            .unwrap();

    assert_eq!(plan.source_output(), LEGACY_ROOT_SOURCE_OUTPUT);
    assert_eq!(plan.events_output(), LEGACY_ROOT_EVENTS_OUTPUT);
    assert_eq!(plan.source_stitch().duration_ms, 200);
    assert_eq!(plan.merged_events().events.duration_ms, 200);
    assert!(plan
        .source_stitch()
        .spans
        .contains(&RunAwareStitchSpan::EncoderGapPadding {
            run_sequence: 0,
            logical_start_ms: 50,
            logical_end_ms: 100,
        }));
    assert!(plan.source_stitch().spans.iter().all(|span| {
        !matches!(
            span,
            RunAwareStitchSpan::EncoderGapPadding {
                logical_start_ms: 100,
                ..
            }
        )
    }));
}

#[test]
fn rejects_nonterminal_interrupted_and_discarded_sessions() {
    for disposition in [
        None,
        Some(TerminalDisposition::Interrupted),
        Some(TerminalDisposition::Discarded),
    ] {
        let error = plan_legacy_root_projection(&journal(disposition), &inputs()).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }
}

#[test]
fn rejects_missing_extra_and_duplicate_run_inputs() {
    let completed = journal(Some(TerminalDisposition::Completed));
    let all = inputs();
    assert!(plan_legacy_root_projection(&completed, &all[..1]).is_err());

    let mut extra = all.clone();
    extra.push(all[0].clone());
    assert!(plan_legacy_root_projection(&completed, &extra).is_err());

    let duplicate = vec![all[0].clone(), all[0].clone()];
    assert!(plan_legacy_root_projection(&completed, &duplicate).is_err());
}

#[test]
fn rejects_interval_duration_settings_geometry_and_correlation_drift() {
    let completed = journal(Some(TerminalDisposition::Completed));
    let mut duration_drift = inputs();
    duration_drift[1] = SealedLegacyProjectionRun::new(1, 100, 200, settings(), events(99));
    assert!(plan_legacy_root_projection(&completed, &duration_drift).is_err());

    let mut interval_drift = inputs();
    interval_drift[1] = SealedLegacyProjectionRun::new(1, 99, 199, settings(), events(100));
    assert!(plan_legacy_root_projection(&completed, &interval_drift).is_err());

    let mut settings_drift = inputs();
    let mut alternate_settings = settings();
    alternate_settings.audio_rate = 44_100;
    settings_drift[1] =
        SealedLegacyProjectionRun::new(1, 100, 200, alternate_settings, events(100));
    assert!(plan_legacy_root_projection(&completed, &settings_drift).is_err());

    let mut geometry_drift = inputs();
    let mut alternate_events = events(100);
    alternate_events.screen_w = 641;
    geometry_drift[1] = SealedLegacyProjectionRun::new(1, 100, 200, settings(), alternate_events);
    assert!(plan_legacy_root_projection(&completed, &geometry_drift).is_err());

    let mut correlation_drift = inputs();
    let mut alternate_events = events(100);
    alternate_events.cursor_correlation.max_metadata_age_ms = Some(17);
    correlation_drift[1] =
        SealedLegacyProjectionRun::new(1, 100, 200, settings(), alternate_events);
    assert!(plan_legacy_root_projection(&completed, &correlation_drift).is_err());
}

#[test]
fn fractional_intent_does_not_use_a_legacy_f32_equality_gate() {
    let requested_fps = 29.97_f64;
    let narrowed_settings_fps = requested_fps as f32;
    assert_ne!(f64::from(narrowed_settings_fps), requested_fps);
    assert!(plan_legacy_root_projection(
        &journal_with_fps(Some(TerminalDisposition::Completed), requested_fps),
        &inputs_with_fps(narrowed_settings_fps),
    )
    .is_ok());
}

#[test]
fn forwards_optional_cadence_metadata_without_changing_legacy_projection() {
    let cadence = CaptureCadence::from_server_fps(29.97).unwrap();
    let plan = plan_legacy_root_projection(
        &journal_with_fps_and_cadence(
            Some(TerminalDisposition::Completed),
            29.97,
            Some(cadence.clone()),
        ),
        &inputs_with_fps(29.97_f32),
    )
    .unwrap();

    assert_eq!(plan.capture_cadence(), Some(&cadence));
    assert_eq!(plan.source_stitch().duration_ms, 200);
}

#[test]
fn replay_failure_cannot_be_projected() {
    let mut entries = journal(Some(TerminalDisposition::Completed)).entries();
    let RecordingSessionJournalEntry::Run(run) = &mut entries[2] else {
        panic!("fixture journal entry ordering changed");
    };
    run.logical_end_ms = 99;

    let error = plan_legacy_root_projection_entries(entries, &inputs()).unwrap_err();
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.cause.contains("replay validation"));
}
