use super::pause_projection::SealedLegacyProjectionRun;
use super::pause_session_owner::{
    PauseSessionFactOutcome, PauseSessionOwner, PauseSessionOwnerError, PauseSessionOwnerPhase,
    PauseSessionProjectionExecutor, PauseSessionWorkerAdapter, PauseSessionWorkerError,
};
use super::pause_worker_protocol::{
    ObservedBoundaryIdentity, PauseWorkerCommand, PauseWorkerFact, PauseWorkerFactRejection,
    WorkerBoundaryKind, WorkerEpoch,
};
use super::run_seal_coordinator::{
    RecordingSessionJournalSink, SealedRunEvidence, SessionTimeOrigin,
};
use record_capture::SelectedCaptureStreams;
use record_core::{CursorCorrelation, EventTrack, Settings};
use record_recovery::{
    CheckpointSequenceRange, RecordingSessionIntent, RecordingSessionJournal,
    RecordingSessionJournalEntry, RecordingStream, SealedRun, SessionJournalError, StreamFragment,
    StreamFragmentFacts, TerminalDisposition,
};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct MemoryJournalSink {
    journal: RecordingSessionJournal,
}

impl MemoryJournalSink {
    fn new(intent: RecordingSessionIntent) -> Self {
        Self {
            journal: RecordingSessionJournal::new(intent).unwrap(),
        }
    }

    fn from_journal(journal: RecordingSessionJournal) -> Self {
        Self { journal }
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
        match entry {
            RecordingSessionJournalEntry::Intent(_) => {
                Err(SessionJournalError::Invalid("duplicate intent".into()))
            }
            RecordingSessionJournalEntry::Transition(transition) => {
                self.journal.append_transition(transition)
            }
            RecordingSessionJournalEntry::InputSidecar(pin) => {
                self.journal.append_input_sidecar(pin)
            }
            RecordingSessionJournalEntry::Run(run) => self.journal.seal_run(run),
            RecordingSessionJournalEntry::Terminal(terminal) => {
                self.journal.seal_terminal(terminal)
            }
        }
    }
}

#[derive(Debug, Default)]
struct FakeWorker {
    commands: Vec<PauseWorkerCommand>,
    fail_on_dispatch: Option<usize>,
    dispatches: usize,
}

impl PauseSessionWorkerAdapter for FakeWorker {
    fn dispatch(&mut self, command: &PauseWorkerCommand) -> Result<(), PauseSessionWorkerError> {
        let attempt = self.dispatches;
        self.dispatches += 1;
        if self.fail_on_dispatch == Some(attempt) {
            return Err(PauseSessionWorkerError::new(
                "injected worker dispatch failure",
            ));
        }
        self.commands.push(command.clone());
        Ok(())
    }
}

#[derive(Default)]
struct FakeProjection {
    calls: usize,
    entries: Vec<RecordingSessionJournalEntry>,
    event_runs: usize,
}

impl PauseSessionProjectionExecutor for FakeProjection {
    fn execute(
        &mut self,
        journal: &RecordingSessionJournal,
        event_runs: &[SealedLegacyProjectionRun],
    ) -> Result<(), PauseSessionWorkerError> {
        self.calls += 1;
        self.entries = journal.entries();
        self.event_runs = event_runs.len();
        Ok(())
    }
}

fn at(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

fn streams() -> SelectedCaptureStreams {
    SelectedCaptureStreams::new(false, false, false, true)
}

fn intent(streams: &SelectedCaptureStreams) -> RecordingSessionIntent {
    RecordingSessionIntent::new(
        "pause-session-owner-test",
        1_000,
        1_000,
        30.0,
        None,
        false,
        "monitor:opaque-17",
        streams.streams().to_vec(),
    )
}

fn owner() -> PauseSessionOwner<MemoryJournalSink, FakeWorker> {
    owner_with_worker(FakeWorker::default())
}

fn owner_with_worker(worker: FakeWorker) -> PauseSessionOwner<MemoryJournalSink, FakeWorker> {
    let streams = streams();
    PauseSessionOwner::new(MemoryJournalSink::new(intent(&streams)), streams, worker).unwrap()
}

fn start(owner: &mut PauseSessionOwner<MemoryJournalSink, FakeWorker>, origin: Instant) {
    owner
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 1_000))
        .unwrap();
}

fn fact(
    stream: RecordingStream,
    command: WorkerBoundaryKind,
    generation: u64,
    epoch: u64,
    observed_at: Instant,
) -> PauseWorkerFact {
    let observed_boundary =
        ObservedBoundaryIdentity::observed(WorkerEpoch::new(epoch), observed_at);
    match command {
        WorkerBoundaryKind::Pause => PauseWorkerFact::PauseSealed {
            stream,
            generation,
            observed_boundary,
        },
        WorkerBoundaryKind::Resume => PauseWorkerFact::ResumeReady {
            stream,
            generation,
            observed_boundary,
        },
    }
}

fn sealed_evidence(
    generation: u64,
    sequence: u64,
    start_ms: u64,
    end_ms: u64,
) -> SealedRunEvidence {
    let selected = streams();
    let duration = end_ms - start_ms;
    let fragments = selected
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
                end_offset_ms: duration,
                media_duration_ms: duration,
                decoded_video_frames: (*stream == RecordingStream::ScreenVideo).then_some(1),
                avg_frame_rate: None,
                r_frame_rate: None,
            },
        })
        .collect();
    let run = SealedRun {
        sequence,
        observed_start_ms: start_ms,
        observed_end_ms: end_ms,
        logical_start_ms: start_ms,
        logical_end_ms: end_ms,
        checkpoints: CheckpointSequenceRange {
            first: sequence,
            last: sequence,
        },
        fragments,
    };
    let events = EventTrack {
        duration_ms: duration,
        screen_w: 640,
        screen_h: 360,
        monitors: Vec::new(),
        cursor: Vec::new(),
        clicks: Vec::new(),
        scrolls: Vec::new(),
        keys: Vec::new(),
        cursor_correlation: CursorCorrelation::default(),
    };
    SealedRunEvidence::new(
        generation,
        run,
        SealedLegacyProjectionRun::new(
            sequence,
            start_ms,
            end_ms,
            Settings {
                width: 640,
                height: 360,
                fps: 30.0,
                audio_rate: 48_000,
            },
            events,
        ),
        selected.streams().to_vec(),
    )
}

fn seal_first_pause(owner: &mut PauseSessionOwner<MemoryJournalSink, FakeWorker>, origin: Instant) {
    owner.request_pause_at(at(origin, 100)).unwrap();
    assert_eq!(
        owner
            .accept_worker_fact(
                fact(
                    RecordingStream::ScreenVideo,
                    WorkerBoundaryKind::Pause,
                    1,
                    1,
                    at(origin, 110),
                ),
                at(origin, 110),
            )
            .unwrap(),
        PauseSessionFactOutcome::AwaitingWorkerFacts
    );
    assert_eq!(
        owner
            .accept_worker_fact(
                fact(
                    RecordingStream::InputEvents,
                    WorkerBoundaryKind::Pause,
                    1,
                    1,
                    at(origin, 111),
                ),
                at(origin, 111),
            )
            .unwrap(),
        PauseSessionFactOutcome::PauseFactsComplete { generation: 1 }
    );
    owner
        .seal_pause_at(sealed_evidence(1, 0, 0, 100), at(origin, 112))
        .unwrap();
}

#[test]
fn pause_is_not_durable_until_exact_sealed_run_facts_arrive() {
    let origin = Instant::now();
    let mut owner = owner();
    start(&mut owner, origin);
    owner.request_pause_at(at(origin, 100)).unwrap();

    for stream in [RecordingStream::ScreenVideo, RecordingStream::InputEvents] {
        owner
            .accept_worker_fact(
                fact(stream, WorkerBoundaryKind::Pause, 1, 1, at(origin, 110)),
                at(origin, 110),
            )
            .unwrap();
    }
    assert_eq!(
        owner.phase(),
        PauseSessionOwnerPhase::PauseAwaitingSeal { generation: 1 }
    );
    assert!(owner.journal().sealed_runs().is_empty());
    assert!(matches!(
        owner.seal_pause_at(sealed_evidence(2, 0, 0, 100), at(origin, 111)),
        Err(PauseSessionOwnerError::MismatchedGeneration { .. })
    ));
    assert!(owner.journal().sealed_runs().is_empty());

    owner
        .seal_pause_at(sealed_evidence(1, 0, 0, 100), at(origin, 112))
        .unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Paused);
    assert_eq!(owner.journal().sealed_runs().len(), 1);
}

#[test]
fn stale_and_duplicate_worker_facts_cannot_change_session_ownership() {
    let origin = Instant::now();
    let mut owner = owner();
    start(&mut owner, origin);
    owner.request_pause_at(at(origin, 100)).unwrap();
    let screen = fact(
        RecordingStream::ScreenVideo,
        WorkerBoundaryKind::Pause,
        1,
        1,
        at(origin, 101),
    );
    owner.accept_worker_fact(screen, at(origin, 101)).unwrap();

    assert!(matches!(
        owner.accept_worker_fact(screen, at(origin, 102)),
        Err(PauseSessionOwnerError::WorkerFact(
            PauseWorkerFactRejection::DuplicateStream { .. }
        ))
    ));
    assert!(matches!(
        owner.accept_worker_fact(
            fact(
                RecordingStream::InputEvents,
                WorkerBoundaryKind::Pause,
                1,
                0,
                at(origin, 103),
            ),
            at(origin, 103),
        ),
        Err(PauseSessionOwnerError::WorkerFact(
            PauseWorkerFactRejection::StaleEpoch { .. }
        ))
    ));
    assert_eq!(
        owner.phase(),
        PauseSessionOwnerPhase::PauseAwaitingFacts { generation: 1 }
    );
    assert!(owner.journal().sealed_runs().is_empty());
}

#[test]
fn resume_refusal_returns_to_durable_paused_then_retries_with_a_fresh_epoch() {
    let origin = Instant::now();
    let mut owner = owner();
    start(&mut owner, origin);
    seal_first_pause(&mut owner, origin);
    owner.request_resume_at(at(origin, 150)).unwrap();

    let refusal = PauseWorkerFact::Refused {
        stream: RecordingStream::ScreenVideo,
        command: WorkerBoundaryKind::Resume,
        generation: 2,
        observed_boundary: ObservedBoundaryIdentity::observed(WorkerEpoch::new(2), at(origin, 151)),
    };
    assert_eq!(
        owner.accept_worker_fact(refusal, at(origin, 151)).unwrap(),
        PauseSessionFactOutcome::ResumeRefused { generation: 2 }
    );
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Paused);
    assert_eq!(owner.journal().transitions().len(), 2);

    owner.request_resume_at(at(origin, 160)).unwrap();
    for stream in [RecordingStream::ScreenVideo, RecordingStream::InputEvents] {
        owner
            .accept_worker_fact(
                fact(stream, WorkerBoundaryKind::Resume, 3, 3, at(origin, 161)),
                at(origin, 161),
            )
            .unwrap();
    }
    assert_eq!(
        owner.phase(),
        PauseSessionOwnerPhase::ResumeAwaitingDurability { generation: 3 }
    );
    owner.seal_resume_at(at(origin, 162)).unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Recording);
    assert_eq!(owner.journal().transitions().len(), 3);
}

#[test]
fn stop_wins_over_pending_pause_and_late_facts() {
    let origin = Instant::now();
    let mut owner = owner();
    start(&mut owner, origin);
    owner.request_pause_at(at(origin, 100)).unwrap();
    let stop = owner.request_stop_at(at(origin, 105)).unwrap();
    assert!(stop.requires_sealed_run());
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopping);

    assert!(matches!(
        owner.accept_worker_fact(
            fact(
                RecordingStream::ScreenVideo,
                WorkerBoundaryKind::Pause,
                1,
                1,
                at(origin, 106),
            ),
            at(origin, 106),
        ),
        Err(PauseSessionOwnerError::WrongPhase { .. })
    ));
    owner
        .seal_stop_at(
            Some(sealed_evidence(1, 0, 0, 100)),
            TerminalDisposition::Completed,
            at(origin, 107),
        )
        .unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopped);
    assert!(matches!(
        owner.request_stop_at(at(origin, 108)),
        Err(PauseSessionOwnerError::WrongPhase { .. })
    ));

    let worker = owner.into_worker_adapter();
    assert!(matches!(
        worker.commands[0],
        PauseWorkerCommand::Pause { .. }
    ));
    assert!(matches!(
        worker.commands[1],
        PauseWorkerCommand::Stop { .. }
    ));
}

#[test]
fn durable_journal_replays_identically_and_can_never_reattach_live_workers() {
    let origin = Instant::now();
    let mut owner = owner();
    start(&mut owner, origin);
    owner.request_stop_at(at(origin, 100)).unwrap();
    owner
        .seal_stop_at(
            Some(sealed_evidence(0, 0, 0, 100)),
            TerminalDisposition::Completed,
            at(origin, 101),
        )
        .unwrap();
    let replay = RecordingSessionJournal::replay(owner.journal().entries()).unwrap();
    assert_eq!(replay, *owner.journal());

    let streams = streams();
    assert!(matches!(
        PauseSessionOwner::new(
            MemoryJournalSink::from_journal(replay),
            streams,
            FakeWorker::default(),
        ),
        Err(PauseSessionOwnerError::RunSeal(_))
    ));
}

#[test]
fn projection_is_admitted_once_only_after_completed_terminal_journal() {
    let origin = Instant::now();
    let mut owner = owner();
    start(&mut owner, origin);
    owner.request_stop_at(at(origin, 100)).unwrap();
    owner
        .seal_stop_at(
            Some(sealed_evidence(0, 0, 0, 100)),
            TerminalDisposition::Completed,
            at(origin, 101),
        )
        .unwrap();
    let mut projection = FakeProjection::default();
    owner.execute_completed_projection(&mut projection).unwrap();
    assert_eq!(projection.calls, 1);
    assert_eq!(projection.event_runs, 1);
    assert_eq!(projection.entries, owner.journal().entries());
    assert!(matches!(
        owner.execute_completed_projection(&mut projection),
        Err(PauseSessionOwnerError::ProjectionUnavailable)
    ));
}

#[test]
fn pause_dispatch_failure_is_blocked_but_stop_can_still_win_and_terminalize() {
    let origin = Instant::now();
    let mut owner = owner_with_worker(FakeWorker {
        fail_on_dispatch: Some(0),
        ..FakeWorker::default()
    });
    start(&mut owner, origin);

    assert!(matches!(
        owner.request_pause_at(at(origin, 100)),
        Err(PauseSessionOwnerError::WorkerDispatch(_))
    ));
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Blocked);
    assert!(owner.journal().sealed_runs().is_empty());
    assert!(matches!(
        owner.accept_worker_fact(
            fact(
                RecordingStream::ScreenVideo,
                WorkerBoundaryKind::Pause,
                1,
                1,
                at(origin, 101),
            ),
            at(origin, 101),
        ),
        Err(PauseSessionOwnerError::WrongPhase { .. })
    ));

    owner.request_stop_at(at(origin, 102)).unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopping);
    owner
        .seal_stop_at(
            Some(sealed_evidence(1, 0, 0, 100)),
            TerminalDisposition::Discarded,
            at(origin, 103),
        )
        .unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopped);
}

#[test]
fn resume_dispatch_failure_never_claims_resumed_and_stop_needs_no_live_run() {
    let origin = Instant::now();
    let mut owner = owner_with_worker(FakeWorker {
        fail_on_dispatch: Some(1),
        ..FakeWorker::default()
    });
    start(&mut owner, origin);
    seal_first_pause(&mut owner, origin);

    assert!(matches!(
        owner.request_resume_at(at(origin, 150)),
        Err(PauseSessionOwnerError::WorkerDispatch(_))
    ));
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Blocked);
    assert_eq!(owner.journal().transitions().len(), 2);

    let stop = owner.request_stop_at(at(origin, 151)).unwrap();
    assert!(!stop.requires_sealed_run());
    owner
        .seal_stop_at(None, TerminalDisposition::Discarded, at(origin, 152))
        .unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopped);
}

#[test]
fn stop_dispatch_failure_is_already_stopping_and_can_be_durably_sealed() {
    let origin = Instant::now();
    let mut owner = owner_with_worker(FakeWorker {
        fail_on_dispatch: Some(0),
        ..FakeWorker::default()
    });
    start(&mut owner, origin);

    assert!(matches!(
        owner.request_stop_at(at(origin, 100)),
        Err(PauseSessionOwnerError::WorkerDispatch(_))
    ));
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopping);
    assert!(matches!(
        owner.accept_worker_fact(
            fact(
                RecordingStream::ScreenVideo,
                WorkerBoundaryKind::Pause,
                1,
                1,
                at(origin, 100),
            ),
            at(origin, 100),
        ),
        Err(PauseSessionOwnerError::WrongPhase { .. })
    ));
    owner
        .seal_stop_at(
            Some(sealed_evidence(0, 0, 0, 100)),
            TerminalDisposition::Discarded,
            at(origin, 101),
        )
        .unwrap();
    assert_eq!(owner.phase(), PauseSessionOwnerPhase::Stopped);
}
