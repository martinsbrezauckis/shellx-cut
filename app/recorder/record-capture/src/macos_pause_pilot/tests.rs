use super::{
    MacosPauseAcceptedScreen, MacosPauseAudioFactory, MacosPauseAudioLayout, MacosPauseAudioOwner,
    MacosPauseCommand, MacosPauseCommandRejection, MacosPausePilotEvent, MacosPausePilotProfile,
    MacosPausePilotRefusal, MacosPausePilotRequest, MacosPausePilotStarted, MacosPauseRunOwner,
    MacosPauseScreenOwner, MacosPauseScreenRange, MacosPauseStartError, MacosSealedAudioRun,
    MacosSealedScreenRun,
};
use record_core::Settings;
use record_recovery::{Checkpoint, CheckpointFacts, RecordingStream};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

type Log = Rc<RefCell<Vec<String>>>;

struct Screen {
    starts: Vec<MacosPausePilotStarted>,
    active: Option<MacosPausePilotStarted>,
    log: Log,
}

impl MacosPauseScreenOwner for Screen {
    fn start(
        &mut self,
        profile: &MacosPausePilotProfile,
    ) -> Result<MacosPausePilotStarted, MacosPauseStartError> {
        self.log
            .borrow_mut()
            .push(format!("screen-start:{}", profile.exact_monitor_id()));
        let started = self
            .starts
            .first()
            .cloned()
            .ok_or(MacosPauseStartError::NativeStartFailed)?;
        self.starts.remove(0);
        self.active = Some(started.clone());
        Ok(started)
    }

    fn seal_active(&mut self) -> Result<MacosSealedScreenRun, ()> {
        self.log.borrow_mut().push("screen-seal".into());
        self.active
            .take()
            .map(|started| sealed_run(&started))
            .ok_or(())
    }

    fn abort_and_join(&mut self) -> Result<(), ()> {
        self.log.borrow_mut().push("screen-abort".into());
        self.active = None;
        Ok(())
    }
}

struct AudioFactory(Log);

impl MacosPauseAudioFactory for AudioFactory {
    fn start(
        &mut self,
        profile: &MacosPausePilotProfile,
        started: &MacosPausePilotStarted,
    ) -> Result<Vec<Box<dyn MacosPauseAudioOwner>>, ()> {
        profile
            .selected_audio_streams()
            .into_iter()
            .map(|stream| {
                self.0.borrow_mut().push(format!(
                    "audio-start:{stream:?}:{}",
                    started.physical_generation
                ));
                Ok(Box::new(Audio {
                    stream,
                    started: started.clone(),
                    log: self.0.clone(),
                }) as Box<dyn MacosPauseAudioOwner>)
            })
            .collect()
    }
}

struct Audio {
    stream: RecordingStream,
    started: MacosPausePilotStarted,
    log: Log,
}

impl MacosPauseAudioOwner for Audio {
    fn stream(&self) -> RecordingStream {
        self.stream
    }

    fn seal_after_screen(self: Box<Self>, end_ms: u64) -> Result<MacosSealedAudioRun, ()> {
        self.log
            .borrow_mut()
            .push(format!("audio-seal:{:?}:{end_ms}", self.stream));
        Ok(MacosSealedAudioRun {
            stream: self.stream,
            layout: MacosPauseAudioLayout::PacketStart,
            source_generation: self.started.physical_generation,
            artifact: format!(
                "recording-{:?}-{:020}.wav",
                self.stream, self.started.physical_generation
            ),
            bytes: 48,
            sha256: "a".repeat(64),
            media_duration_ms: end_ms - self.started.observed_start_ms,
            native_ready_unix_ms: self.started.unix_ms,
            native_ready_raw_ms: self.started.observed_start_ms,
            raw_start_ms: self.started.observed_start_ms,
            raw_end_ms: end_ms,
        })
    }

    fn abort_and_join(self: Box<Self>) {
        self.log
            .borrow_mut()
            .push(format!("audio-abort:{:?}", self.stream));
    }
}

fn exact_monitor() -> String {
    format!("shellx-monitor-v1:macos:{}", "a".repeat(64))
}

fn started() -> MacosPausePilotStarted {
    MacosPausePilotStarted {
        exact_monitor_id: exact_monitor(),
        physical_generation: 1,
        observed_start_ms: 10,
        monotonic_at: Instant::now(),
        unix_ms: 1_700_000_000_010,
        accepted: accepted(),
    }
}

fn started_after_resume() -> MacosPausePilotStarted {
    MacosPausePilotStarted {
        exact_monitor_id: exact_monitor(),
        physical_generation: 2,
        observed_start_ms: 70,
        monotonic_at: Instant::now(),
        unix_ms: 1_700_000_000_070,
        accepted: accepted(),
    }
}

fn accepted() -> MacosPauseAcceptedScreen {
    MacosPauseAcceptedScreen {
        settings: Settings {
            width: 640,
            height: 360,
            fps: 30.0,
            audio_rate: 48_000,
        },
        range: MacosPauseScreenRange {
            origin_x: 0,
            origin_y: 0,
            width: 640,
            height: 360,
        },
    }
}

fn sealed_run(started: &MacosPausePilotStarted) -> MacosSealedScreenRun {
    MacosSealedScreenRun {
        exact_monitor_id: started.exact_monitor_id.clone(),
        physical_generation: started.physical_generation,
        observed_start_ms: started.observed_start_ms,
        observed_end_ms: started.observed_start_ms + 40,
        observed_at: Instant::now(),
        accepted: started.accepted.clone(),
        checkpoints: vec![Checkpoint {
            sequence: started.physical_generation - 1,
            file: "checkpoints/segment-000000.mp4".into(),
            bytes: 1,
            sha256: "a".repeat(64),
            media: None,
            held_last_frame_ms: None,
            facts: CheckpointFacts {
                start_ms: started.observed_start_ms,
                end_ms: started.observed_start_ms + 40,
                event_offset_ms: started.observed_start_ms,
                audio_offset_ms: None,
            },
        }],
    }
}

#[test]
fn admission_refuses_non_screen_surfaces_without_silently_dropping_them() {
    let mut request = MacosPausePilotRequest::screen_video_only(exact_monitor(), 30.0);
    request.region = true;
    assert_eq!(
        MacosPausePilotProfile::admit(request),
        Err(MacosPausePilotRefusal::Region)
    );
    assert_eq!(
        MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_video_only(
            "shellx-monitor-v1:macos:not-a-digest".into(),
            30.0,
        )),
        Err(MacosPausePilotRefusal::ExactMonitorRequired)
    );
}

#[test]
fn start_rejects_a_screen_fact_not_bound_to_the_selected_exact_target() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_video_only(
        exact_monitor(),
        30.0,
    ))
    .unwrap();
    let mut wrong_target = started();
    wrong_target.exact_monitor_id = format!("shellx-monitor-v1:macos:{}", "b".repeat(64));

    assert!(matches!(
        MacosPauseRunOwner::start(
            profile,
            Screen {
                starts: vec![wrong_target],
                active: None,
                log: log.clone(),
            },
            AudioFactory(log.clone()),
        ),
        Err(MacosPauseStartError::NativeStartFailed)
    ));
    assert_eq!(
        log.borrow().as_slice(),
        [
            format!("screen-start:{}", exact_monitor()),
            "screen-abort".into(),
        ]
    );
}

#[test]
fn selected_audio_starts_after_screen_and_seals_only_after_screen_publication() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_with_audio(
        exact_monitor(),
        30.0,
        true,
        true,
    ))
    .unwrap();
    let (mut owner, _) = MacosPauseRunOwner::start(
        profile,
        Screen {
            starts: vec![started(), started_after_resume()],
            active: None,
            log: log.clone(),
        },
        AudioFactory(log.clone()),
    )
    .unwrap();

    let events = owner.process_commands(
        [MacosPauseCommand::Pause {
            generation: 1,
            epoch: 1,
            physical_generation: 1,
        }],
        Instant::now(),
    );
    assert!(
        matches!(events.as_slice(), [MacosPausePilotEvent::PauseSealed { audio, .. }] if audio.len() == 2)
    );
    let resumed = owner.process_commands(
        [MacosPauseCommand::Resume {
            generation: 2,
            epoch: 2,
            resume_from_physical_generation: 1,
        }],
        Instant::now(),
    );
    assert!(matches!(
        resumed.as_slice(),
        [MacosPausePilotEvent::ResumeReady { started, .. }] if started.physical_generation == 2
    ));
    let resealed = owner.process_commands(
        [MacosPauseCommand::Pause {
            generation: 3,
            epoch: 3,
            physical_generation: 2,
        }],
        Instant::now(),
    );
    assert!(
        matches!(resealed.as_slice(), [MacosPausePilotEvent::PauseSealed { audio, .. }] if audio.iter().all(|item| item.source_generation == 2))
    );
    assert_eq!(
        log.borrow().as_slice(),
        [
            format!("screen-start:{}", exact_monitor()),
            "audio-start:MicrophoneAudio:1".into(),
            "audio-start:SystemAudio:1".into(),
            "screen-seal".into(),
            "audio-seal:MicrophoneAudio:50".into(),
            "audio-seal:SystemAudio:50".into(),
            format!("screen-start:{}", exact_monitor()),
            "audio-start:MicrophoneAudio:2".into(),
            "audio-start:SystemAudio:2".into(),
            "screen-seal".into(),
            "audio-seal:MicrophoneAudio:110".into(),
            "audio-seal:SystemAudio:110".into(),
        ]
    );
}

#[test]
fn queued_stop_discards_pause_and_remains_terminal() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_with_audio(
        exact_monitor(),
        30.0,
        true,
        false,
    ))
    .unwrap();
    let (mut owner, _) = MacosPauseRunOwner::start(
        profile,
        Screen {
            starts: vec![started()],
            active: None,
            log: log.clone(),
        },
        AudioFactory(log.clone()),
    )
    .unwrap();
    let events = owner.process_commands(
        [
            MacosPauseCommand::Pause {
                generation: 7,
                epoch: 1,
                physical_generation: 1,
            },
            MacosPauseCommand::Stop {
                epoch: 2,
                physical_generation: 1,
            },
        ],
        Instant::now(),
    );
    assert!(
        matches!(events.as_slice(), [MacosPausePilotEvent::StopSealed { epoch: 2, audio, .. }] if audio.len() == 1)
    );
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Resume {
                    generation: 8,
                    epoch: 3,
                    resume_from_physical_generation: 1,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::Rejected {
            rejection: MacosPauseCommandRejection::Stopped,
            ..
        }]
    ));
    assert_eq!(
        log.borrow().as_slice(),
        [
            format!("screen-start:{}", exact_monitor()),
            "audio-start:MicrophoneAudio:1".into(),
            "screen-seal".into(),
            "audio-seal:MicrophoneAudio:50".into(),
        ]
    );
}

#[test]
fn stop_first_batch_cannot_skip_an_epoch_from_trailing_commands() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_video_only(
        exact_monitor(),
        30.0,
    ))
    .unwrap();
    let (mut owner, _) = MacosPauseRunOwner::start(
        profile,
        Screen {
            starts: vec![started()],
            active: None,
            log,
        },
        AudioFactory(Rc::new(RefCell::new(Vec::new()))),
    )
    .unwrap();
    assert!(matches!(
        owner
            .process_commands(
                [
                    MacosPauseCommand::Stop {
                        epoch: 2,
                        physical_generation: 1,
                    },
                    MacosPauseCommand::Pause {
                        generation: 1,
                        epoch: 1,
                        physical_generation: 1,
                    },
                ],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::Rejected {
            rejection: MacosPauseCommandRejection::EpochOutOfOrder {
                expected: 1,
                received: 2,
            },
            ..
        }]
    ));
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Stop {
                    epoch: 1,
                    physical_generation: 1,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::StopSealed { .. }]
    ));
}

#[test]
fn delayed_pause_resume_and_stop_are_rejected_without_touching_the_active_generation() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_video_only(
        exact_monitor(),
        30.0,
    ))
    .unwrap();
    let (mut owner, _) = MacosPauseRunOwner::start(
        profile,
        Screen {
            starts: vec![started(), started_after_resume()],
            active: None,
            log: log.clone(),
        },
        AudioFactory(log.clone()),
    )
    .unwrap();
    let pause = MacosPauseCommand::Pause {
        generation: 1,
        epoch: 1,
        physical_generation: 1,
    };
    assert!(matches!(
        owner.process_commands([pause], Instant::now()).as_slice(),
        [MacosPausePilotEvent::PauseSealed { .. }]
    ));
    assert!(matches!(
        owner.process_commands([pause], Instant::now()).as_slice(),
        [MacosPausePilotEvent::Rejected {
            rejection: MacosPauseCommandRejection::EpochOutOfOrder {
                expected: 2,
                received: 1
            },
            ..
        }]
    ));
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Resume {
                    generation: 3,
                    epoch: 2,
                    resume_from_physical_generation: 1,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::Rejected {
            rejection: MacosPauseCommandRejection::GenerationOutOfOrder {
                expected: 2,
                received: 3
            },
            ..
        }]
    ));
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Resume {
                    generation: 2,
                    epoch: 2,
                    resume_from_physical_generation: 1,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::ResumeReady { .. }]
    ));
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Stop {
                    epoch: 3,
                    physical_generation: 1,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::Rejected {
            rejection: MacosPauseCommandRejection::PhysicalGenerationMismatch {
                expected: 2,
                received: 1
            },
            ..
        }]
    ));
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Stop {
                    epoch: 3,
                    physical_generation: 2,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::StopSealed { .. }]
    ));
    assert!(matches!(
        owner
            .process_commands(
                [MacosPauseCommand::Stop {
                    epoch: 4,
                    physical_generation: 2,
                }],
                Instant::now(),
            )
            .as_slice(),
        [MacosPausePilotEvent::Rejected {
            rejection: MacosPauseCommandRejection::Stopped,
            ..
        }]
    ));
    assert_eq!(
        log.borrow().as_slice(),
        [
            format!("screen-start:{}", exact_monitor()),
            "screen-seal".into(),
            format!("screen-start:{}", exact_monitor()),
            "screen-seal".into(),
        ]
    );
}

#[test]
fn dropping_an_active_owner_aborts_screen_then_joins_selected_audio() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let profile = MacosPausePilotProfile::admit(MacosPausePilotRequest::screen_with_audio(
        exact_monitor(),
        30.0,
        true,
        false,
    ))
    .unwrap();
    let (owner, _) = MacosPauseRunOwner::start(
        profile,
        Screen {
            starts: vec![started()],
            active: None,
            log: log.clone(),
        },
        AudioFactory(log.clone()),
    )
    .unwrap();
    drop(owner);
    assert_eq!(
        log.borrow().as_slice(),
        [
            format!("screen-start:{}", exact_monitor()),
            "audio-start:MicrophoneAudio:1".into(),
            "screen-abort".into(),
            "audio-abort:MicrophoneAudio".into(),
        ]
    );
}
