use super::*;
use crate::screen_record::monitor_start_admission::Target;
use crate::screen_record::pause_projection::SealedLegacyProjectionRun;
use crate::screen_record::run_seal_coordinator::{
    RecordingSessionJournalSink, SealedRunEvidence, SessionTimeOrigin,
};
use crate::screen_record::windows_pause_adapter::{
    WindowsPauseAdapterError, WindowsPauseEvidenceFactory,
};
use record_capture::windows_pause_pilot::{
    channel, WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange,
    WindowsPausePilotChannelError, WindowsPausePilotCheckpointRange,
    WindowsPausePilotCommandSender, WindowsPausePilotEvent, WindowsPausePilotEventReceiver,
    WindowsPausePilotEventSender, WindowsPausePilotStarted, WindowsSealedScreenRun,
    WindowsSealedWgcCheckpoint,
};
use record_capture::SelectedCaptureStreams;
use record_core::{CursorCorrelation, EventTrack, Settings};
use record_recovery::{
    Checkpoint, CheckpointFacts, CheckpointSequenceRange, RecordingSessionIntent,
    RecordingSessionJournal, RecordingSessionJournalEntry, RecordingStream, SealedRun,
    SessionJournalError, StreamFragment, StreamFragmentFacts,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(super) struct MemoryJournal {
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
            RecordingSessionJournalEntry::InputSidecar(value) => {
                self.journal.append_input_sidecar(value)
            }
            RecordingSessionJournalEntry::Run(value) => self.journal.seal_run(value),
            RecordingSessionJournalEntry::Terminal(value) => self.journal.seal_terminal(value),
        }
    }
}

pub(super) struct Lifecycle {
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
pub(super) struct Factory {
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

pub(super) fn at(origin: Instant, ms: u64) -> Instant {
    origin + Duration::from_millis(ms)
}

pub(super) fn started(origin: Instant, ms: u64) -> WindowsPausePilotStarted {
    started_with_unix(origin, ms, 1_000 + ms)
}

pub(super) fn started_with_unix(
    origin: Instant,
    ms: u64,
    unix_ms: u64,
) -> WindowsPausePilotStarted {
    WindowsPausePilotStarted {
        physical_generation: ms,
        observed_start_ms: ms + 17,
        monotonic_at: at(origin, ms),
        unix_ms,
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

pub(super) fn native_run(start_ms: u64, end_ms: u64) -> WindowsSealedScreenRun {
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
                held_last_frame_ms: None,
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

pub(super) fn admission() -> WindowsPauseSessionAdmission {
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

pub(super) fn session(
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
