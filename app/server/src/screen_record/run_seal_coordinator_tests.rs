use super::pause_projection::SealedLegacyProjectionRun;
use super::run_seal_coordinator::{
    RecordingSessionJournalSink, ResumeReadinessEvidence, RunSealCoordinator,
    RunSealCoordinatorError, SealedRunEvidence, SessionTimeOrigin,
};
use record_capture::{SelectedCaptureStreams, SessionPhase};
use record_core::{CursorCorrelation, CursorSample, EventTrack, KeySample, Settings};
use record_recovery::{
    CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournal, RecordingSessionJournalEntry, RecordingSessionState, RecordingStream,
    SealedRun, SessionJournalError, StreamFragment, StreamFragmentFacts, TerminalDisposition,
};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct MemoryJournalSink {
    journal: RecordingSessionJournal,
    fail_on_append: Option<usize>,
    append_calls: usize,
}

impl MemoryJournalSink {
    fn new(intent: RecordingSessionIntent, fail_on_append: Option<usize>) -> Self {
        Self {
            journal: RecordingSessionJournal::new(intent).unwrap(),
            fail_on_append,
            append_calls: 0,
        }
    }
}

impl RecordingSessionJournalSink for MemoryJournalSink {
    fn journal(&self) -> &RecordingSessionJournal {
        &self.journal
    }

    fn append_entry(
        &mut self,
        entry: RecordingSessionJournalEntry,
    ) -> Result<(), SessionJournalError> {
        if self.fail_on_append == Some(self.append_calls) {
            return Err(SessionJournalError::Invalid(
                "injected append failure".into(),
            ));
        }
        match entry {
            RecordingSessionJournalEntry::Intent(_) => {
                return Err(SessionJournalError::Invalid("duplicate intent".into()))
            }
            RecordingSessionJournalEntry::Transition(transition) => {
                self.journal.append_transition(transition)?;
            }
            RecordingSessionJournalEntry::Run(run) => self.journal.seal_run(run)?,
            RecordingSessionJournalEntry::Terminal(terminal) => {
                self.journal.seal_terminal(terminal)?
            }
        }
        self.append_calls += 1;
        Ok(())
    }
}

fn selected() -> SelectedCaptureStreams {
    SelectedCaptureStreams::new(false, false, false, true)
}

fn intent_with_record_keys(
    streams: &SelectedCaptureStreams,
    record_keys: bool,
) -> RecordingSessionIntent {
    RecordingSessionIntent::new(
        "run-seal-test",
        1_000,
        1_000,
        30.0,
        None,
        record_keys,
        "opaque-target",
        streams.streams().to_vec(),
    )
}

fn coordinator(fail_on_append: Option<usize>) -> RunSealCoordinator<MemoryJournalSink> {
    let streams = selected();
    coordinator_with(streams, true, fail_on_append)
}

fn coordinator_with(
    streams: SelectedCaptureStreams,
    record_keys: bool,
    fail_on_append: Option<usize>,
) -> RunSealCoordinator<MemoryJournalSink> {
    RunSealCoordinator::new(
        MemoryJournalSink::new(
            intent_with_record_keys(&streams, record_keys),
            fail_on_append,
        ),
        streams,
    )
    .unwrap()
}

fn at(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

fn start(coordinator: &mut RunSealCoordinator<MemoryJournalSink>, origin: Instant) {
    coordinator
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 1_000))
        .unwrap();
}

fn settings() -> Settings {
    Settings {
        width: 640,
        height: 360,
        fps: 30.0,
        audio_rate: 48_000,
    }
}

fn events(duration_ms: u64) -> EventTrack {
    EventTrack {
        duration_ms,
        screen_w: 640,
        screen_h: 360,
        monitors: Vec::new(),
        cursor: Vec::new(),
        clicks: Vec::new(),
        scrolls: Vec::new(),
        keys: Vec::new(),
        cursor_correlation: CursorCorrelation::default(),
    }
}

fn evidence(
    generation: u64,
    sequence: u64,
    observed_start_ms: u64,
    observed_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    streams: &SelectedCaptureStreams,
) -> SealedRunEvidence {
    evidence_with_sealed_streams(
        generation,
        sequence,
        observed_start_ms,
        observed_end_ms,
        logical_start_ms,
        logical_end_ms,
        streams,
        streams.streams().to_vec(),
    )
}

fn evidence_with_sealed_streams(
    generation: u64,
    sequence: u64,
    observed_start_ms: u64,
    observed_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    streams: &SelectedCaptureStreams,
    sealed_streams: Vec<RecordingStream>,
) -> SealedRunEvidence {
    evidence_with_projection(
        generation,
        sequence,
        observed_start_ms,
        observed_end_ms,
        logical_start_ms,
        logical_end_ms,
        streams,
        sealed_streams,
        settings(),
        logical_end_ms - logical_start_ms,
    )
}

#[allow(clippy::too_many_arguments)]
fn evidence_with_projection(
    generation: u64,
    sequence: u64,
    observed_start_ms: u64,
    observed_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    streams: &SelectedCaptureStreams,
    sealed_streams: Vec<RecordingStream>,
    projection_settings: Settings,
    event_duration_ms: u64,
) -> SealedRunEvidence {
    evidence_with_event_track(
        generation,
        sequence,
        observed_start_ms,
        observed_end_ms,
        logical_start_ms,
        logical_end_ms,
        streams,
        sealed_streams,
        projection_settings,
        events(event_duration_ms),
    )
}

#[allow(clippy::too_many_arguments)]
fn evidence_with_event_track(
    generation: u64,
    sequence: u64,
    observed_start_ms: u64,
    observed_end_ms: u64,
    logical_start_ms: u64,
    logical_end_ms: u64,
    streams: &SelectedCaptureStreams,
    sealed_streams: Vec<RecordingStream>,
    projection_settings: Settings,
    projection_events: EventTrack,
) -> SealedRunEvidence {
    let duration_ms = logical_end_ms - logical_start_ms;
    let fragments = streams
        .streams()
        .iter()
        .enumerate()
        .map(|(index, stream)| StreamFragment {
            stream: *stream,
            checkpoint_sequence: (*stream == RecordingStream::ScreenVideo).then_some(sequence),
            stream_sequence: 0,
            artifact: format!("runs/{sequence}/{index}.bin"),
            bytes: 1,
            sha256: "a".repeat(64),
            facts: StreamFragmentFacts {
                start_offset_ms: 0,
                end_offset_ms: duration_ms,
                media_duration_ms: duration_ms,
                decoded_video_frames: match stream {
                    RecordingStream::ScreenVideo | RecordingStream::CameraVideo => Some(1),
                    _ => None,
                },
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        })
        .collect();
    let run = SealedRun {
        sequence,
        observed_start_ms,
        observed_end_ms,
        logical_start_ms,
        logical_end_ms,
        checkpoints: CheckpointSequenceRange {
            first: sequence,
            last: sequence,
        },
        fragments,
    };
    SealedRunEvidence::new(
        generation,
        run,
        SealedLegacyProjectionRun::new(
            sequence,
            logical_start_ms,
            logical_end_ms,
            projection_settings,
            projection_events,
        ),
        sealed_streams,
    )
}

#[test]
fn intent_precedes_started_and_started_uses_the_observed_backend_origin() {
    let origin = Instant::now();
    let mut coordinator = coordinator(None);

    start(&mut coordinator, origin);

    assert_eq!(coordinator.phase(), SessionPhase::Recording);
    let entries = coordinator.journal_sink().journal.entries();
    assert!(matches!(
        entries[0],
        RecordingSessionJournalEntry::Intent(_)
    ));
    let RecordingSessionJournalEntry::Transition(started) = &entries[1] else {
        panic!("started transition missing after intent");
    };
    assert_eq!(started.state, RecordingSessionState::Started);
    assert_eq!(started.logical_offset_ms, 0);
    assert_eq!(started.observed_unix_ms, 1_000);
}

#[test]
fn rejects_a_backend_origin_that_predates_the_durable_intent() {
    let origin = Instant::now();
    let mut coordinator = coordinator(None);

    assert!(coordinator
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 999))
        .is_err());
    assert_eq!(coordinator.phase(), SessionPhase::Preparing);
    assert!(coordinator.journal_sink().journal.transitions().is_empty());
}

#[test]
fn pause_seals_exact_evidence_before_paused_and_then_accepts_acknowledgements() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);

    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    assert_eq!(pause.boundary().generation, 1);
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 100, 0, 100, &streams),
            at(origin, 125),
        )
        .unwrap();

    assert_eq!(coordinator.phase(), SessionPhase::Paused);
    let entries = coordinator.journal_sink().journal.entries();
    assert!(matches!(entries[2], RecordingSessionJournalEntry::Run(_)));
    let RecordingSessionJournalEntry::Transition(paused) = &entries[3] else {
        panic!("paused transition was not durable after run");
    };
    assert_eq!(paused.state, RecordingSessionState::Paused);
    assert_eq!(paused.logical_offset_ms, 100);
    assert_eq!(paused.observed_unix_ms, 1_125);
}

#[test]
fn delayed_post_close_evidence_owns_the_pause_endpoint_not_command_issue() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);

    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    assert!(coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 140, 0, 140, &streams),
            at(origin, 139),
        )
        .is_err());
    assert!(coordinator.journal_sink().journal.sealed_runs().is_empty());
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 140, 0, 140, &streams),
            at(origin, 145),
        )
        .unwrap();

    let journal = coordinator.journal_sink().journal();
    assert_eq!(journal.sealed_runs()[0].observed_end_ms, 140);
    assert_eq!(journal.sealed_runs()[0].logical_end_ms, 140);
    assert_eq!(journal.transitions()[1].logical_offset_ms, 140);
    assert_eq!(coordinator.phase(), SessionPhase::Paused);

    let resume = coordinator.request_resume_at(at(origin, 300)).unwrap();
    coordinator
        .seal_resume_at(
            ResumeReadinessEvidence::new(resume.generation, streams.streams().to_vec()),
            at(origin, 320),
        )
        .unwrap();
    let stop = coordinator.request_stop_at(at(origin, 330)).unwrap();
    assert_eq!(stop.generation(), Some(2));
    coordinator
        .seal_stop_at(
            Some(evidence(2, 1, 320, 350, 140, 170, &streams)),
            TerminalDisposition::Completed,
            at(origin, 355),
        )
        .unwrap();

    let journal = coordinator.journal_sink().journal();
    assert_eq!(journal.sealed_runs()[1].logical_start_ms, 140);
    assert_eq!(journal.sealed_runs()[1].logical_end_ms, 170);
    assert_eq!(journal.terminal().unwrap().logical_end_ms, 170);
    assert!(RecordingSessionJournal::replay(journal.entries()).is_ok());
}

#[test]
fn stale_or_overlong_media_evidence_cannot_replace_the_pending_pause_boundary() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();

    assert!(coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 99, 0, 99, &streams),
            at(origin, 130),
        )
        .is_err());
    assert!(coordinator.journal_sink().journal.sealed_runs().is_empty());
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);

    let evidence_at_125 = evidence(pause.boundary().generation, 0, 0, 125, 0, 125, &streams);
    let mut run = evidence_at_125.run().clone();
    run.fragments[0].facts.media_duration_ms = 126;
    let media_too_long = SealedRunEvidence::new(
        pause.boundary().generation,
        run,
        evidence_at_125.legacy_projection_run(),
        streams.streams().to_vec(),
    );
    assert!(coordinator
        .seal_pause_at(media_too_long, at(origin, 130))
        .is_err());
    assert!(coordinator.journal_sink().journal.sealed_runs().is_empty());

    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 125, 0, 125, &streams),
            at(origin, 130),
        )
        .unwrap();
    assert_eq!(
        coordinator.journal_sink().journal.sealed_runs()[0].logical_end_ms,
        125
    );
}

#[test]
fn fractional_intent_is_not_rejected_by_the_legacy_f32_projection_settings() {
    let origin = Instant::now();
    let streams = selected();
    let intent = RecordingSessionIntent::new(
        "fractional-run-seal-test",
        1_000,
        1_000,
        29.97,
        None,
        true,
        "opaque-target",
        streams.streams().to_vec(),
    );
    let mut coordinator =
        RunSealCoordinator::new(MemoryJournalSink::new(intent, None), streams).unwrap();
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    let mut projection_settings = settings();
    projection_settings.fps = 29.97_f32;
    let projection_streams = selected();

    coordinator
        .seal_pause_at(
            evidence_with_projection(
                pause.boundary().generation,
                0,
                0,
                100,
                0,
                100,
                &projection_streams,
                projection_streams.streams().to_vec(),
                projection_settings,
                100,
            ),
            at(origin, 125),
        )
        .unwrap();
}

#[test]
fn resume_requires_exact_readiness_and_durably_precedes_logical_timestamp_reopening() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 100, 0, 100, &streams),
            at(origin, 110),
        )
        .unwrap();

    let resume = coordinator.request_resume_at(at(origin, 200)).unwrap();
    coordinator
        .seal_resume_at(
            ResumeReadinessEvidence::new(resume.generation, streams.streams().to_vec()),
            at(origin, 240),
        )
        .unwrap();

    assert_eq!(coordinator.phase(), SessionPhase::Recording);
    let entries = coordinator.journal_sink().journal.entries();
    let RecordingSessionJournalEntry::Transition(resumed) = &entries[4] else {
        panic!("resumed transition missing after durable paused prefix");
    };
    assert_eq!(resumed.state, RecordingSessionState::Resumed);
    assert_eq!(resumed.logical_offset_ms, 100);
}

#[test]
fn invalid_resume_readiness_keeps_timestamps_disabled_for_same_generation_retry() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 100, 0, 100, &streams),
            at(origin, 110),
        )
        .unwrap();
    let resume = coordinator.request_resume_at(at(origin, 200)).unwrap();

    assert!(coordinator
        .seal_resume_at(
            ResumeReadinessEvidence::new(resume.generation, vec![RecordingStream::ScreenVideo]),
            at(origin, 240),
        )
        .is_err());
    assert_eq!(coordinator.phase(), SessionPhase::Resuming);
    assert!(matches!(
        coordinator.request_resume_at(at(origin, 241)),
        Err(RunSealCoordinatorError::BoundaryPending)
    ));
    coordinator
        .seal_resume_at(
            ResumeReadinessEvidence::new(resume.generation, streams.streams().to_vec()),
            at(origin, 242),
        )
        .unwrap();
    assert_eq!(coordinator.phase(), SessionPhase::Recording);
}

#[test]
fn second_run_settings_or_event_drift_fails_before_a_journal_append() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 100, 0, 100, &streams),
            at(origin, 110),
        )
        .unwrap();
    let resume = coordinator.request_resume_at(at(origin, 200)).unwrap();
    coordinator
        .seal_resume_at(
            ResumeReadinessEvidence::new(resume.generation, streams.streams().to_vec()),
            at(origin, 240),
        )
        .unwrap();
    let second_pause = coordinator.request_pause_at(at(origin, 300)).unwrap();

    let mut drifted_settings = settings();
    drifted_settings.audio_rate = 44_100;
    let settings_drift = evidence_with_projection(
        second_pause.boundary().generation,
        1,
        240,
        300,
        100,
        160,
        &streams,
        streams.streams().to_vec(),
        drifted_settings,
        60,
    );
    assert!(coordinator
        .seal_pause_at(settings_drift, at(origin, 310))
        .is_err());
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    assert_eq!(coordinator.journal_sink().journal.sealed_runs().len(), 1);

    let event_drift = evidence_with_projection(
        second_pause.boundary().generation,
        1,
        240,
        300,
        100,
        160,
        &streams,
        streams.streams().to_vec(),
        settings(),
        59,
    );
    assert!(coordinator
        .seal_pause_at(event_drift, at(origin, 310))
        .is_err());
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    assert_eq!(coordinator.journal_sink().journal.sealed_runs().len(), 1);
}

#[test]
fn projection_input_events_require_input_selection_and_respect_record_keys() {
    let origin = Instant::now();
    let screen_only = SelectedCaptureStreams::screen_only();
    let mut no_input = coordinator_with(screen_only.clone(), true, None);
    start(&mut no_input, origin);
    let pause = no_input.request_pause_at(at(origin, 100)).unwrap();
    let mut cursor_events = events(100);
    cursor_events.cursor.push(CursorSample {
        t_ms: 0,
        x: 0.0,
        y: 0.0,
    });
    assert!(no_input
        .seal_pause_at(
            evidence_with_event_track(
                pause.boundary().generation,
                0,
                0,
                100,
                0,
                100,
                &screen_only,
                screen_only.streams().to_vec(),
                settings(),
                cursor_events,
            ),
            at(origin, 110),
        )
        .is_err());
    assert_eq!(no_input.phase(), SessionPhase::Pausing);

    let streams = selected();
    let mut no_keys = coordinator_with(streams.clone(), false, None);
    start(&mut no_keys, origin);
    let pause = no_keys.request_pause_at(at(origin, 100)).unwrap();
    let mut key_events = events(100);
    key_events.keys.push(KeySample {
        t_ms: 0,
        key: "A".into(),
        down: true,
    });
    assert!(no_keys
        .seal_pause_at(
            evidence_with_event_track(
                pause.boundary().generation,
                0,
                0,
                100,
                0,
                100,
                &streams,
                streams.streams().to_vec(),
                settings(),
                key_events,
            ),
            at(origin, 110),
        )
        .is_err());
    assert_eq!(no_keys.phase(), SessionPhase::Pausing);
}

#[test]
fn invalid_pause_evidence_keeps_the_boundary_frozen_for_same_generation_retry() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    let invalid_evidence = evidence_with_sealed_streams(
        pause.boundary().generation,
        0,
        0,
        100,
        0,
        100,
        &streams,
        vec![RecordingStream::ScreenVideo, RecordingStream::ScreenVideo],
    );

    assert!(coordinator
        .seal_pause_at(invalid_evidence, at(origin, 125))
        .is_err());
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    assert!(coordinator.journal_sink().journal.sealed_runs().is_empty());
    assert_eq!(coordinator.journal_sink().journal.transitions().len(), 1);
    assert!(matches!(
        coordinator.request_pause_at(at(origin, 126)),
        Err(RunSealCoordinatorError::BoundaryPending)
    ));
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 100, 0, 100, &streams),
            at(origin, 127),
        )
        .unwrap();
    assert_eq!(coordinator.phase(), SessionPhase::Paused);
}

#[test]
fn stop_wins_after_pre_durable_pause_validation_failure() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();

    assert!(coordinator
        .seal_pause_at(
            evidence_with_sealed_streams(
                pause.boundary().generation,
                0,
                0,
                100,
                0,
                100,
                &streams,
                vec![RecordingStream::ScreenVideo, RecordingStream::ScreenVideo],
            ),
            at(origin, 110),
        )
        .is_err());
    let stop = coordinator.request_stop_at(at(origin, 120)).unwrap();
    assert!(stop.requires_sealed_run());
    assert_eq!(coordinator.phase(), SessionPhase::Stopped);
    coordinator
        .seal_stop_at(
            Some(evidence(
                pause.boundary().generation,
                0,
                0,
                100,
                0,
                100,
                &streams,
            )),
            TerminalDisposition::Interrupted,
            at(origin, 125),
        )
        .unwrap();
    assert!(coordinator.journal_sink().journal.terminal().is_some());
}

#[test]
fn append_failure_keeps_a_replayable_prefix_frozen_and_closes_the_coordinator() {
    let origin = Instant::now();
    let streams = selected();
    // `Started` is append 0, `Run` is append 1, and the `Paused` append fails.
    let mut coordinator = coordinator(Some(2));
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();

    let error = coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 100, 0, 100, &streams),
            at(origin, 125),
        )
        .unwrap_err();
    assert!(matches!(error, RunSealCoordinatorError::Journal(_)));
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    assert_eq!(coordinator.journal_sink().journal.sealed_runs().len(), 1);
    assert!(RecordingSessionJournal::replay(coordinator.journal_sink().journal.entries()).is_ok());
    assert!(matches!(
        coordinator.request_pause_at(at(origin, 200)),
        Err(RunSealCoordinatorError::FailedAfterDurableAppend)
    ));
}

#[test]
fn stopping_append_failure_keeps_the_run_prefix_replayable_and_stays_closed() {
    let origin = Instant::now();
    let streams = selected();
    // `Started` is append 0, `Run` is append 1, and `Stopping` is append 2.
    let mut coordinator = coordinator(Some(2));
    start(&mut coordinator, origin);
    coordinator.request_stop_at(at(origin, 100)).unwrap();

    let error = coordinator
        .seal_stop_at(
            Some(evidence(0, 0, 0, 100, 0, 100, &streams)),
            TerminalDisposition::Interrupted,
            at(origin, 125),
        )
        .unwrap_err();
    assert!(matches!(error, RunSealCoordinatorError::Journal(_)));
    assert_eq!(coordinator.phase(), SessionPhase::Stopped);
    assert_eq!(coordinator.journal_sink().journal.sealed_runs().len(), 1);
    assert!(coordinator.journal_sink().journal.terminal().is_none());
    assert!(RecordingSessionJournal::replay(coordinator.journal_sink().journal.entries()).is_ok());
    assert!(matches!(
        coordinator.seal_stop_at(None, TerminalDisposition::Interrupted, at(origin, 126)),
        Err(RunSealCoordinatorError::FailedAfterDurableAppend)
    ));
}

#[test]
fn stop_wins_over_late_facts_and_appends_one_terminal_after_the_active_run() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);

    let stop = coordinator.request_stop_at(at(origin, 100)).unwrap();
    assert!(stop.requires_sealed_run());
    assert_eq!(stop.generation(), Some(0));
    assert_eq!(coordinator.phase(), SessionPhase::Stopped);
    assert!(matches!(
        coordinator.request_pause_at(at(origin, 110)),
        Err(RunSealCoordinatorError::Stopped)
    ));
    coordinator
        .seal_stop_at(
            Some(evidence(0, 0, 0, 100, 0, 100, &streams)),
            TerminalDisposition::Completed,
            at(origin, 125),
        )
        .unwrap();

    let entries = coordinator.journal_sink().journal.entries();
    assert!(matches!(entries[2], RecordingSessionJournalEntry::Run(_)));
    assert!(matches!(
        entries[3],
        RecordingSessionJournalEntry::Transition(DurableStateTransition {
            state: RecordingSessionState::Stopping,
            ..
        })
    ));
    assert!(matches!(
        entries[4],
        RecordingSessionJournalEntry::Terminal(_)
    ));
    assert!(matches!(
        coordinator.seal_stop_at(None, TerminalDisposition::Completed, at(origin, 126)),
        Err(RunSealCoordinatorError::Stopped)
    ));
}

#[test]
fn stop_during_pause_uses_post_close_evidence_and_keeps_one_terminal_prefix() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    let stop = coordinator.request_stop_at(at(origin, 105)).unwrap();

    assert_eq!(stop.generation(), Some(pause.boundary().generation));
    assert_eq!(coordinator.phase(), SessionPhase::Stopped);
    assert!(matches!(
        coordinator.request_resume_at(at(origin, 106)),
        Err(RunSealCoordinatorError::Stopped)
    ));
    coordinator
        .seal_stop_at(
            Some(evidence(
                pause.boundary().generation,
                0,
                0,
                125,
                0,
                125,
                &streams,
            )),
            TerminalDisposition::Interrupted,
            at(origin, 130),
        )
        .unwrap();

    let journal = coordinator.journal_sink().journal();
    assert_eq!(journal.sealed_runs()[0].logical_end_ms, 125);
    assert_eq!(journal.transitions().len(), 2);
    assert_eq!(journal.terminal().unwrap().logical_end_ms, 125);
    assert!(RecordingSessionJournal::replay(journal.entries()).is_ok());
}

#[test]
fn stop_during_resume_discards_pending_readiness_without_a_second_run() {
    let origin = Instant::now();
    let streams = selected();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);
    let pause = coordinator.request_pause_at(at(origin, 100)).unwrap();
    coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 110, 0, 110, &streams),
            at(origin, 115),
        )
        .unwrap();
    let _resume = coordinator.request_resume_at(at(origin, 200)).unwrap();
    let stop = coordinator.request_stop_at(at(origin, 205)).unwrap();

    assert_eq!(stop.generation(), None);
    assert_eq!(coordinator.phase(), SessionPhase::Stopped);
    coordinator
        .seal_stop_at(None, TerminalDisposition::Interrupted, at(origin, 210))
        .unwrap();
    let journal = coordinator.journal_sink().journal();
    assert_eq!(journal.sealed_runs().len(), 1);
    assert_eq!(journal.transitions().len(), 3);
    assert_eq!(journal.terminal().unwrap().logical_end_ms, 110);
    assert!(RecordingSessionJournal::replay(journal.entries()).is_ok());
}

#[test]
fn immediate_sub_millisecond_stop_writes_no_invalid_zero_length_run() {
    let origin = Instant::now();
    let mut coordinator = coordinator(None);
    start(&mut coordinator, origin);

    let stop = coordinator
        .request_stop_at(origin + Duration::from_micros(500))
        .unwrap();
    assert!(!stop.requires_sealed_run());
    coordinator
        .seal_stop_at(
            None,
            TerminalDisposition::Discarded,
            origin + Duration::from_micros(500),
        )
        .unwrap();

    let journal = &coordinator.journal_sink().journal;
    assert!(journal.sealed_runs().is_empty());
    assert_eq!(journal.terminal().unwrap().logical_end_ms, 0);
    assert!(RecordingSessionJournal::replay(journal.entries()).is_ok());
}

#[test]
fn overflow_fails_before_the_pause_journal_mutates() {
    let origin = Instant::now();
    let mut coordinator = coordinator(None);
    coordinator
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, u64::MAX))
        .unwrap();
    let pause = coordinator.request_pause_at(at(origin, 1)).unwrap();
    let streams = selected();

    assert!(coordinator
        .seal_pause_at(
            evidence(pause.boundary().generation, 0, 0, 1, 0, 1, &streams),
            at(origin, 1),
        )
        .is_err());
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
    assert!(coordinator.journal_sink().journal.sealed_runs().is_empty());
}
