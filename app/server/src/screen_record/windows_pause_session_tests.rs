use super::*;
use crate::screen_record::pause_projection::SealedLegacyProjectionRun;
use crate::screen_record::run_seal_coordinator::{
    RecordingSessionJournalSink, SealedRunEvidence, SessionTimeOrigin,
};
use crate::screen_record::windows_pause_adapter::WindowsPauseEvidenceFactory;
use record_capture::windows_pause_pilot::{
    channel, WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange,
    WindowsPausePilotCheckpointRange, WindowsPausePilotEvent, WindowsPausePilotEventSender,
    WindowsPausePilotStarted, WindowsSealedScreenRun, WindowsSealedWgcCheckpoint,
};
use record_core::{CursorCorrelation, EventTrack, Settings};
use record_recovery::{
    Checkpoint, CheckpointFacts, CheckpointSequenceRange, RecordingSessionIntent,
    RecordingSessionJournal, RecordingSessionJournalEntry, RecordingSessionState, RecordingStream,
    SealedRun, SessionJournalError, StreamFragment, StreamFragmentFacts,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

struct MemoryJournal {
    journal: RecordingSessionJournal,
}

impl MemoryJournal {
    fn new(log: &Rc<RefCell<Vec<&'static str>>>) -> Self {
        log.borrow_mut().push("intent");
        Self {
            journal: RecordingSessionJournal::new(RecordingSessionIntent::new(
                "pause-session",
                1_000,
                100,
                30.0,
                None,
                false,
                "shellx-monitor-v1:windows:exact",
                vec![RecordingStream::ScreenVideo],
            ))
            .unwrap(),
        }
    }
}

impl RecordingSessionJournalSink for MemoryJournal {
    fn journal(&self) -> &RecordingSessionJournal {
        &self.journal
    }

    fn append_entry(
        &mut self,
        entry: RecordingSessionJournalEntry,
    ) -> Result<(), SessionJournalError> {
        match entry {
            RecordingSessionJournalEntry::Intent(_) => {
                Err(SessionJournalError::Invalid("intent".into()))
            }
            RecordingSessionJournalEntry::Transition(value) => {
                self.journal.append_transition(value)
            }
            RecordingSessionJournalEntry::Run(value) => self.journal.seal_run(value),
            RecordingSessionJournalEntry::Terminal(value) => self.journal.seal_terminal(value),
        }
    }
}

struct Lifecycle {
    commands: record_capture::windows_pause_pilot::WindowsPausePilotCommandSender,
    events: Option<WindowsPausePilotEventReceiver>,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Lifecycle {
    fn new(
        commands: record_capture::windows_pause_pilot::WindowsPausePilotCommandSender,
        events: WindowsPausePilotEventReceiver,
        log: Rc<RefCell<Vec<&'static str>>>,
    ) -> Self {
        log.borrow_mut().push("native");
        Self {
            commands,
            events: Some(events),
            log,
        }
    }
}

impl WindowsPauseLifecycle for Lifecycle {
    fn command_sender(&self) -> WindowsPausePilotCommandSender {
        self.commands.clone()
    }

    fn take_event_receiver(&mut self) -> Option<WindowsPausePilotEventReceiver> {
        self.events.take()
    }

    fn shutdown_and_join(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.log.borrow_mut().push("shutdown");
        Ok(())
    }

    fn join_after_terminal(&mut self) -> Result<(), WindowsPausePilotChannelError> {
        self.log.borrow_mut().push("join");
        Ok(())
    }
}

#[derive(Default)]
struct Factory {
    starts: Vec<Option<u64>>,
    next: u64,
}

impl WindowsPauseEvidenceFactory for Factory {
    fn stage_started(
        &mut self,
        generation: Option<u64>,
        _: &WindowsPausePilotStarted,
    ) -> Result<(), WindowsPauseAdapterError> {
        self.starts.push(generation);
        Ok(())
    }

    fn set_session_origin(&mut self, _: SessionTimeOrigin) -> Result<(), WindowsPauseAdapterError> {
        Ok(())
    }

    fn verify_discarded_stop(
        &mut self,
        _: &WindowsSealedScreenRun,
        _: Instant,
    ) -> Result<(), WindowsPauseAdapterError> {
        Ok(())
    }

    fn verify_and_build(
        &mut self,
        generation: u64,
        _: &WindowsSealedScreenRun,
        _at: Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        let sequence = self.next;
        self.next += 1;
        let (observed_start_ms, observed_end_ms, logical_start_ms, logical_end_ms) =
            if sequence == 0 {
                (0, 120, 0, 120)
            } else {
                (200, 300, 120, 220)
            };
        let settings = settings();
        let events = EventTrack {
            duration_ms: logical_end_ms - logical_start_ms,
            screen_w: settings.width,
            screen_h: settings.height,
            monitors: Vec::new(),
            cursor: Vec::new(),
            clicks: Vec::new(),
            scrolls: Vec::new(),
            keys: Vec::new(),
            cursor_correlation: CursorCorrelation::default(),
        };
        Ok(SealedRunEvidence::new(
            generation,
            SealedRun {
                sequence,
                observed_start_ms,
                observed_end_ms,
                logical_start_ms,
                logical_end_ms,
                checkpoints: CheckpointSequenceRange {
                    first: sequence,
                    last: sequence,
                },
                fragments: vec![StreamFragment {
                    stream: RecordingStream::ScreenVideo,
                    checkpoint_sequence: Some(sequence),
                    stream_sequence: 0,
                    artifact: format!("checkpoints/segment-{sequence:06}.mp4"),
                    bytes: 1,
                    sha256: "a".repeat(64),
                    facts: StreamFragmentFacts {
                        start_offset_ms: 0,
                        end_offset_ms: logical_end_ms - logical_start_ms,
                        media_duration_ms: logical_end_ms - logical_start_ms,
                        decoded_video_frames: Some(1),
                        avg_frame_rate: None,
                        r_frame_rate: None,
                    },
                }],
            },
            SealedLegacyProjectionRun::new(
                sequence,
                logical_start_ms,
                logical_end_ms,
                settings,
                events,
            ),
            vec![RecordingStream::ScreenVideo],
        ))
    }
}

fn settings() -> Settings {
    Settings {
        width: 640,
        height: 360,
        fps: 30.0,
        audio_rate: 48_000,
    }
}

fn at(origin: Instant, ms: u64) -> Instant {
    origin + Duration::from_millis(ms)
}

fn started(origin: Instant, ms: u64) -> WindowsPausePilotStarted {
    WindowsPausePilotStarted {
        physical_generation: ms,
        observed_start_ms: ms + 17,
        monotonic_at: at(origin, ms),
        unix_ms: 1_000 + ms,
        accepted: WindowsPausePilotAcceptedCapture {
            settings: settings(),
            range: WindowsPausePilotCaptureRange {
                origin_x: 0,
                origin_y: 0,
                width: 640,
                height: 360,
            },
        },
    }
}

fn native_run(start_ms: u64, end_ms: u64) -> WindowsSealedScreenRun {
    WindowsSealedScreenRun {
        observed_start_ms: start_ms,
        observed_end_ms: end_ms,
        accepted: WindowsPausePilotAcceptedCapture {
            settings: settings(),
            range: WindowsPausePilotCaptureRange {
                origin_x: 0,
                origin_y: 0,
                width: 640,
                height: 360,
            },
        },
        range: WindowsPausePilotCheckpointRange {
            first_physical_generation: 1,
            last_physical_generation: 1,
            first_checkpoint_sequence: 0,
            last_checkpoint_sequence: 0,
        },
        checkpoints: vec![WindowsSealedWgcCheckpoint {
            physical_generation: 1,
            start_ms,
            end_ms,
            checkpoint: Checkpoint {
                sequence: 0,
                file: "checkpoints/segment-000000.mp4".into(),
                bytes: 1,
                sha256: "a".repeat(64),
                media: None,
                facts: CheckpointFacts {
                    start_ms,
                    end_ms,
                    event_offset_ms: start_ms,
                    audio_offset_ms: None,
                },
            },
        }],
    }
}

fn admission() -> WindowsPauseSessionAdmission {
    WindowsPauseSessionAdmission::admit(
        Target {
            legacy_index: Some(7),
            exact_id: Some(exact_monitor()),
        },
        SelectedCaptureStreams::screen_only(),
        30.0,
        100,
    )
    .unwrap()
}

fn exact_monitor() -> String {
    format!("shellx-monitor-v1:windows:{}", "a".repeat(64))
}

fn session(
    origin: Instant,
) -> (
    WindowsPauseSession<MemoryJournal, Factory, Lifecycle>,
    record_capture::windows_pause_pilot::WindowsPausePilotCommandReceiver,
    WindowsPausePilotEventSender,
    Rc<RefCell<Vec<&'static str>>>,
) {
    let (commands, command_rx, event_tx, event_rx) = channel();
    let log = Rc::new(RefCell::new(Vec::new()));
    let journal = MemoryJournal::new(&log);
    event_tx
        .send(WindowsPausePilotEvent::Started {
            started: started(origin, 0),
        })
        .unwrap();
    let session = WindowsPauseSession::from_started_lifecycle(
        journal,
        &admission(),
        Lifecycle::new(commands, event_rx, log.clone()),
        Factory::default(),
    )
    .unwrap();
    (session, command_rx, event_tx, log)
}

#[test]
fn admission_refuses_before_a_lifecycle_or_filesystem_owner_can_exist() {
    assert!(WindowsPauseSessionAdmission::admit(
        Target {
            legacy_index: Some(1),
            exact_id: None
        },
        SelectedCaptureStreams::screen_only(),
        30.0,
        100,
    )
    .is_err());
    assert!(WindowsPauseSessionAdmission::admit(
        Target {
            legacy_index: None,
            exact_id: Some("exact".into())
        },
        SelectedCaptureStreams::screen_only(),
        29.97,
        100,
    )
    .is_err());
}

#[test]
fn intent_is_present_before_native_start_and_receiver_is_taken_once() {
    let origin = Instant::now();
    let (session, _commands, _events, log) = session(origin);
    assert_eq!(&*log.borrow(), &["intent", "native"]);
    assert!(matches!(
        session.journal().entries()[0],
        RecordingSessionJournalEntry::Intent(_)
    ));
    assert_eq!(session.phase(), PauseSessionOwnerPhase::Recording);
}

#[test]
fn pause_resume_and_stop_use_durable_order_and_normal_join_only() {
    let origin = Instant::now();
    let (mut session, commands, events, log) = session(origin);
    session.request_pause_at(at(origin, 100)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(
            record_capture::windows_pause_pilot::WindowsPausePilotCommand::Pause {
                generation: 1,
                epoch: 1
            }
        )
    ));
    events
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: native_run(17, 137),
            observed_at: at(origin, 120),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Paused)
    );
    assert!(matches!(
        session.journal().entries()[2],
        RecordingSessionJournalEntry::Run(_)
    ));
    assert!(
        matches!(session.journal().entries()[3], RecordingSessionJournalEntry::Transition(ref value) if value.state == RecordingSessionState::Paused)
    );

    session.request_resume_at(at(origin, 180)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(
            record_capture::windows_pause_pilot::WindowsPausePilotCommand::Resume {
                generation: 2,
                epoch: 2
            }
        )
    ));
    events
        .send(WindowsPausePilotEvent::ResumeReady {
            generation: 2,
            epoch: 2,
            started: started(origin, 200),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Resumed)
    );
    assert!(
        matches!(session.journal().transitions().last(), Some(value) if value.state == RecordingSessionState::Resumed)
    );

    session.request_stop_at(at(origin, 260)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(record_capture::windows_pause_pilot::WindowsPausePilotCommand::Stop { epoch: 3 })
    ));
    events
        .send(WindowsPausePilotEvent::StopSealed {
            epoch: 3,
            run: Some(native_run(217, 317)),
            observed_at: at(origin, 300),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Stopped)
    );
    assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
    assert!(session.journal().terminal().is_some());
    drop(session);
    assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
}

#[test]
fn failure_and_drop_stop_join_without_inventing_a_terminal() {
    let origin = Instant::now();
    let (mut session, _commands, events, log) = session(origin);
    events
        .send(WindowsPausePilotEvent::Failed {
            operation: record_capture::windows_pause_pilot::WindowsPausePilotOperation::Pause,
            generation: Some(1),
            epoch: 1,
            observed_at: at(origin, 10),
        })
        .unwrap();
    assert!(session.pump_once().is_err());
    assert_eq!(session.phase(), PauseSessionOwnerPhase::Blocked);
    assert!(session.journal().terminal().is_none());
    assert_eq!(&*log.borrow(), &["intent", "native", "shutdown"]);
    drop(session);
    assert_eq!(&*log.borrow(), &["intent", "native", "shutdown"]);
}

#[test]
fn stop_dominates_late_pause_evidence_and_joins_after_terminal_only() {
    let origin = Instant::now();
    let (mut session, commands, events, log) = session(origin);
    session.request_pause_at(at(origin, 100)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(
            record_capture::windows_pause_pilot::WindowsPausePilotCommand::Pause {
                generation: 1,
                epoch: 1
            }
        )
    ));
    session.request_stop_at(at(origin, 101)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(record_capture::windows_pause_pilot::WindowsPausePilotCommand::Stop { epoch: 2 })
    ));
    events
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: native_run(17, 117),
            observed_at: at(origin, 120),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::StopRunRetained)
    );
    events
        .send(WindowsPausePilotEvent::StopSealed {
            epoch: 2,
            run: None,
            observed_at: at(origin, 130),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Stopped)
    );
    assert_eq!(session.journal().sealed_runs().len(), 1);
    assert!(session
        .journal()
        .transitions()
        .iter()
        .all(|transition| transition.state != RecordingSessionState::Paused));
    assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
}

#[test]
fn stop_dominates_queued_resume_but_validates_its_physical_run_without_journaling_it() {
    let origin = Instant::now();
    let (mut session, commands, events, log) = session(origin);
    session.request_pause_at(at(origin, 100)).unwrap();
    let _ = commands.try_recv().unwrap();
    events
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: native_run(17, 117),
            observed_at: at(origin, 120),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Paused)
    );
    session.request_resume_at(at(origin, 180)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(
            record_capture::windows_pause_pilot::WindowsPausePilotCommand::Resume {
                generation: 2,
                epoch: 2
            }
        )
    ));
    events
        .send(WindowsPausePilotEvent::ResumeReady {
            generation: 2,
            epoch: 2,
            started: started(origin, 200),
        })
        .unwrap();
    session.request_stop_at(at(origin, 220)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(record_capture::windows_pause_pilot::WindowsPausePilotCommand::Stop { epoch: 3 })
    ));
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::StaleIgnored)
    );
    events
        .send(WindowsPausePilotEvent::StopSealed {
            epoch: 3,
            run: Some(native_run(217, 317)),
            observed_at: at(origin, 320),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Stopped)
    );
    assert_eq!(session.journal().sealed_runs().len(), 1);
    assert!(session
        .journal()
        .transitions()
        .iter()
        .all(|transition| transition.state != RecordingSessionState::Resumed));
    assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
}

#[test]
fn wrong_stop_epoch_is_rejected_before_terminal_mutation() {
    let origin = Instant::now();
    let (mut session, commands, events, _log) = session(origin);
    session.request_stop_at(at(origin, 100)).unwrap();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Some(record_capture::windows_pause_pilot::WindowsPausePilotCommand::Stop { epoch: 1 })
    ));
    events
        .send(WindowsPausePilotEvent::StopSealed {
            epoch: 9,
            run: Some(native_run(17, 117)),
            observed_at: at(origin, 120),
        })
        .unwrap();
    assert!(session.pump_once().is_err());
    assert_eq!(session.phase(), PauseSessionOwnerPhase::Blocked);
    assert!(session.journal().terminal().is_none());
}

#[test]
fn no_capture_registry_is_reintroduced_into_the_private_owner() {
    assert!(!include_str!("windows_pause_session.rs").contains("capture_registry"));
    let origin = Instant::now();
    let (_session, _commands, _events, log) = session(origin);
    assert_eq!(&*log.borrow(), &["intent", "native"]);
}
