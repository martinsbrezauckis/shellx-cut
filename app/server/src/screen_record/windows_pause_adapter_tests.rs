use super::*;
use crate::screen_record::pause_projection::SealedLegacyProjectionRun;
use crate::screen_record::pause_session_owner::PauseSessionOwner;
use crate::screen_record::run_seal_coordinator::SealedRunEvidence;
use crate::screen_record::run_seal_coordinator::{RecordingSessionJournalSink, SessionTimeOrigin};
use record_capture::windows_pause_pilot::{
    channel, WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange,
    WindowsPausePilotCheckpointRange, WindowsPausePilotCommand, WindowsPausePilotEvent,
    WindowsPausePilotEventSender, WindowsPausePilotStarted, WindowsSealedScreenRun,
    WindowsSealedWgcCheckpoint,
};
use record_capture::SelectedCaptureStreams;
use record_core::{CursorCorrelation, EventTrack, Settings};
use record_recovery::{
    Checkpoint, CheckpointFacts, CheckpointSequenceRange, RecordingSessionIntent,
    RecordingSessionJournal, RecordingSessionJournalEntry, RecordingSessionState, RecordingStream,
    SealedRun, SessionJournalError, StreamFragment, StreamFragmentFacts, TerminalDisposition,
};
use std::time::{Duration, Instant};

struct MemoryJournal(RecordingSessionJournal);

impl RecordingSessionJournalSink for MemoryJournal {
    fn journal(&self) -> &RecordingSessionJournal {
        &self.0
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
                self.0.append_transition(transition)
            }
            RecordingSessionJournalEntry::InputSidecar(pin) => self.0.append_input_sidecar(pin),
            RecordingSessionJournalEntry::Run(run) => self.0.seal_run(run),
            RecordingSessionJournalEntry::Terminal(terminal) => self.0.seal_terminal(terminal),
        }
    }
}

struct Factory {
    reject: bool,
}

impl WindowsPauseEvidenceFactory for Factory {
    fn stage_started(
        &mut self,
        _: Option<u64>,
        _: &WindowsPausePilotStarted,
    ) -> Result<(), WindowsPauseAdapterError> {
        Ok(())
    }

    fn set_session_origin(&mut self, _: SessionTimeOrigin) -> Result<(), WindowsPauseAdapterError> {
        Ok(())
    }

    fn verify_and_build(
        &mut self,
        generation: u64,
        native: &WindowsSealedScreenRun,
        _: Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        if self.reject {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let first = native
            .checkpoints
            .first()
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let last = native
            .checkpoints
            .last()
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let duration = native
            .observed_end_ms
            .checked_sub(native.observed_start_ms)
            .ok_or(WindowsPauseAdapterError::EvidenceRejected)?;
        let settings = native.accepted.settings;
        if settings.width != native.accepted.range.width
            || settings.height != native.accepted.range.height
            || first.start_ms != native.observed_start_ms
            || last.end_ms != native.observed_end_ms
            || native.range.first_physical_generation != first.physical_generation
            || native.range.last_physical_generation != last.physical_generation
            || native.range.first_checkpoint_sequence != first.checkpoint.sequence
            || native.range.last_checkpoint_sequence != last.checkpoint.sequence
            || native.checkpoints.windows(2).any(|pair| {
                pair[1].physical_generation != pair[0].physical_generation + 1
                    || pair[1].checkpoint.sequence != pair[0].checkpoint.sequence + 1
                    || pair[1].start_ms < pair[0].end_ms
            })
        {
            return Err(WindowsPauseAdapterError::EvidenceRejected);
        }
        let events = EventTrack {
            duration_ms: duration,
            screen_w: settings.width,
            screen_h: settings.height,
            monitors: Vec::new(),
            cursor: Vec::new(),
            clicks: Vec::new(),
            scrolls: Vec::new(),
            keys: Vec::new(),
            cursor_correlation: CursorCorrelation::default(),
        };
        let run = SealedRun {
            sequence: first.checkpoint.sequence,
            observed_start_ms: native.observed_start_ms,
            observed_end_ms: native.observed_end_ms,
            logical_start_ms: native.observed_start_ms,
            logical_end_ms: native.observed_end_ms,
            checkpoints: CheckpointSequenceRange {
                first: native.range.first_checkpoint_sequence,
                last: native.range.last_checkpoint_sequence,
            },
            fragments: native
                .checkpoints
                .iter()
                .enumerate()
                .map(|(stream_sequence, checkpoint)| StreamFragment {
                    stream: RecordingStream::ScreenVideo,
                    checkpoint_sequence: Some(checkpoint.checkpoint.sequence),
                    stream_sequence: stream_sequence as u64,
                    artifact: checkpoint.checkpoint.file.clone(),
                    bytes: checkpoint.checkpoint.bytes,
                    sha256: checkpoint.checkpoint.sha256.clone(),
                    facts: StreamFragmentFacts {
                        start_offset_ms: checkpoint.start_ms - native.observed_start_ms,
                        end_offset_ms: checkpoint.end_ms - native.observed_start_ms,
                        media_duration_ms: checkpoint.end_ms - checkpoint.start_ms,
                        decoded_video_frames: Some(1),
                        avg_frame_rate: None,
                        r_frame_rate: None,
                    },
                })
                .collect(),
        };
        Ok(SealedRunEvidence::new(
            generation,
            run,
            SealedLegacyProjectionRun::new(
                first.checkpoint.sequence,
                native.observed_start_ms,
                native.observed_end_ms,
                settings,
                events,
            ),
            vec![RecordingStream::ScreenVideo],
        ))
    }

    fn verify_and_build_with_audio(
        &mut self,
        generation: u64,
        native: &WindowsSealedScreenRun,
        _audio: &[record_capture::windows_pause_pilot::WindowsSealedAudioRun],
        at: Instant,
    ) -> Result<SealedRunEvidence, WindowsPauseAdapterError> {
        self.verify_and_build(generation, native, at)
    }
}

fn run(physical_generation: u64, start_ms: u64, end_ms: u64) -> WindowsSealedScreenRun {
    WindowsSealedScreenRun {
        observed_start_ms: start_ms,
        observed_end_ms: end_ms,
        accepted: WindowsPausePilotAcceptedCapture {
            settings: Settings {
                width: 640,
                height: 360,
                fps: 30.0,
                audio_rate: 48_000,
            },
            range: WindowsPausePilotCaptureRange {
                origin_x: -320,
                origin_y: 0,
                width: 640,
                height: 360,
            },
        },
        range: WindowsPausePilotCheckpointRange {
            first_physical_generation: physical_generation,
            last_physical_generation: physical_generation,
            first_checkpoint_sequence: 0,
            last_checkpoint_sequence: 0,
        },
        checkpoints: vec![WindowsSealedWgcCheckpoint {
            physical_generation,
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

fn owner(
    commands: WindowsPausePilotCommandSender,
) -> PauseSessionOwner<MemoryJournal, WindowsPauseDispatchAdapter> {
    let streams = SelectedCaptureStreams::screen_only();
    owner_with_streams(commands, streams)
}

fn owner_with_streams(
    commands: WindowsPausePilotCommandSender,
    streams: SelectedCaptureStreams,
) -> PauseSessionOwner<MemoryJournal, WindowsPauseDispatchAdapter> {
    let intent = RecordingSessionIntent::new(
        "windows-pause-adapter-test",
        1_000,
        1_000,
        30.0,
        None,
        false,
        "shellx-monitor-v1:windows:test",
        streams.streams().to_vec(),
    );
    let adapter = WindowsPauseDispatchAdapter::with_streams(commands, streams.clone());
    PauseSessionOwner::new(
        MemoryJournal(RecordingSessionJournal::new(intent).unwrap()),
        streams,
        adapter,
    )
    .unwrap()
}

fn at(origin: Instant, ms: u64) -> Instant {
    origin + Duration::from_millis(ms)
}

fn send_stop(
    sender: &WindowsPausePilotEventSender,
    epoch: u64,
    native: WindowsSealedScreenRun,
    observed_at: Instant,
) {
    sender
        .send(WindowsPausePilotEvent::StopSealed {
            epoch,
            run: Some(native),
            input: None,
            audio: Vec::new(),
            observed_at,
        })
        .unwrap();
}

#[test]
fn owner_stop_then_translator_binds_any_correlated_physical_checkpoint() {
    let (commands, command_rx, event_tx, event_rx) = channel();
    let mut owner = owner(commands);
    let mut translator = WindowsPauseEventTranslator::new(event_rx, Factory { reject: false });
    let origin = Instant::now();
    owner
        .start_after_backend_origin(SessionTimeOrigin::observed(origin, 1_000))
        .unwrap();

    let request = owner.request_stop_at(at(origin, 100)).unwrap();
    assert_eq!(
        command_rx.try_recv().unwrap(),
        Some(WindowsPausePilotCommand::Stop { epoch: 1 })
    );
    // This models WGC sealing before the controller receives the returned
    // Stop request. The channel preserves it until the expectation binds.
    // A normal WGC checkpoint rollover may have occurred before the logical
    // Stop. Its physical number cannot be compared to server generation zero.
    send_stop(&event_tx, 1, run(44, 0, 120), at(origin, 120));
    translator.expect_stop(&request);
    let WindowsPauseAdapterEvent::StopSealed {
        epoch,
        evidence: Some(evidence),
        observed_at,
    } = translator.try_next().unwrap().unwrap()
    else {
        panic!("translated initial Stop evidence is required")
    };
    assert_eq!(epoch, 1);
    owner
        .seal_stop_at(Some(evidence), TerminalDisposition::Completed, observed_at)
        .unwrap();
    let replay = RecordingSessionJournal::replay(owner.journal().entries().to_vec()).unwrap();
    assert_eq!(
        replay.terminal().unwrap().disposition,
        TerminalDisposition::Completed
    );
    assert!(replay
        .transitions()
        .iter()
        .any(|transition| transition.state == RecordingSessionState::Stopping));
}

#[path = "windows_pause_adapter_selected_audio_tests.rs"]
mod selected_audio_tests;

#[path = "windows_pause_adapter_rejections_tests.rs"]
mod rejections_tests;
#[path = "windows_pause_adapter_stop_tests.rs"]
mod stop_tests;
