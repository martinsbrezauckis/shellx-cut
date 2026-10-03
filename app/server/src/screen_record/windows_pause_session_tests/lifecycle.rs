use super::support::*;
use super::*;
use crate::screen_record::monitor_start_admission::Target;
use record_capture::windows_pause_pilot::WindowsPausePilotEvent;
use record_capture::SelectedCaptureStreams;
use record_recovery::{RecordingSessionJournalEntry, RecordingSessionState};
use std::time::Instant;

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
            input: None,
            audio: Vec::new(),
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
            input: None,
            audio: Vec::new(),
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
fn resumed_native_clock_skew_keeps_the_durable_transition_authoritative() {
    let origin = Instant::now();
    let (mut session, commands, events, _log) = session(origin);
    session.request_pause_at(at(origin, 100)).unwrap();
    let _ = commands.try_recv().unwrap();
    events
        .send(WindowsPausePilotEvent::PauseSealed {
            generation: 1,
            epoch: 1,
            run: native_run(17, 137),
            input: None,
            audio: Vec::new(),
            observed_at: at(origin, 120),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Paused)
    );
    session.request_resume_at(at(origin, 180)).unwrap();
    let _ = commands.try_recv().unwrap();
    events
        .send(WindowsPausePilotEvent::ResumeReady {
            generation: 2,
            epoch: 2,
            // The WGC native pair is sampled independently: it must not be
            // equated to the durable origin conversion at this same Instant.
            started: started_with_unix(origin, 200, 1_201),
        })
        .unwrap();
    assert_eq!(
        session.pump_once().unwrap(),
        Some(WindowsPauseSessionEvent::Resumed)
    );
    assert!(matches!(
        session.journal().transitions().last(),
        Some(value) if value.state == RecordingSessionState::Resumed
            && value.observed_unix_ms == 1_200
    ));
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
            input: None,
            audio: Vec::new(),
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
            input: None,
            audio: Vec::new(),
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
            input: None,
            audio: Vec::new(),
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
            input: None,
            audio: Vec::new(),
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
            input: None,
            audio: Vec::new(),
            observed_at: at(origin, 120),
        })
        .unwrap();
    assert!(session.pump_once().is_err());
    assert_eq!(session.phase(), PauseSessionOwnerPhase::Blocked);
    assert!(session.journal().terminal().is_none());
}

#[test]
fn no_capture_registry_is_reintroduced_into_the_private_owner() {
    assert!(!include_str!("../windows_pause_session.rs").contains("capture_registry"));
    let origin = Instant::now();
    let (_session, _commands, _events, log) = session(origin);
    assert_eq!(&*log.borrow(), &["intent", "native"]);
}

#[test]
fn selected_audio_stop_from_paused_retains_the_sealed_run_and_joins() {
    for system_audio in [false, true] {
        let streams = SelectedCaptureStreams::new(true, system_audio, false, false);
        let origin = Instant::now();
        let (mut session, commands, events, log) = session_with_streams(origin, streams.clone());
        session.request_pause_at(at(origin, 100)).unwrap();
        let _ = commands.try_recv().unwrap();
        let audio = streams
            .streams()
            .iter()
            .copied()
            .filter(|stream| *stream != record_recovery::RecordingStream::ScreenVideo)
            .map(
                |stream| record_capture::windows_pause_pilot::WindowsSealedAudioRun {
                    stream,
                    source_generation: 1,
                    artifact: format!("recording-{stream:?}-generation-1.wav"),
                    bytes: 48,
                    sha256: "a".repeat(64),
                    media_duration_ms: 120,
                    native_ready_unix_ms: 1_000,
                    native_ready_raw_ms: 0,
                    raw_start_ms: 0,
                    raw_end_ms: 120,
                },
            )
            .collect();
        events
            .send(WindowsPausePilotEvent::PauseSealed {
                generation: 1,
                epoch: 1,
                run: native_run(17, 137),
                input: None,
                audio,
                observed_at: at(origin, 120),
            })
            .unwrap();
        assert_eq!(
            session.pump_once().unwrap(),
            Some(WindowsPauseSessionEvent::Paused)
        );
        let sealed = session.journal().sealed_runs()[0].clone();
        assert_eq!(sealed.fragments.len(), streams.streams().len());
        session.request_stop_at(at(origin, 150)).unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            Some(record_capture::windows_pause_pilot::WindowsPausePilotCommand::Stop { epoch: 2 })
        ));
        events
            .send(WindowsPausePilotEvent::StopSealed {
                epoch: 2,
                run: None,
                input: None,
                audio: Vec::new(),
                observed_at: at(origin, 160),
            })
            .unwrap();
        assert_eq!(
            session.pump_once().unwrap(),
            Some(WindowsPauseSessionEvent::Stopped)
        );
        assert_eq!(session.journal().sealed_runs(), &[sealed]);
        assert_eq!(
            session.journal().terminal().unwrap().disposition,
            record_recovery::TerminalDisposition::Completed
        );
        assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
        drop(session);
        assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
    }
}

// Use the production calibrated factory, not the synthetic Factory above: the
// Resume command and its later Pause boundary have distinct generations.
#[test]
fn calibrated_resume_then_pause_and_stop_preserve_boundary_generations() {
    use crate::screen_record::windows_pause_adapter::WindowsPauseAdapterError;
    use crate::screen_record::windows_pause_evidence::CalibratedWindowsPauseEvidenceFactory;
    use crate::screen_record::windows_pause_evidence_artifacts::WindowsPauseArtifactVerifier;
    use record_capture::windows_pause_pilot::{
        channel, WindowsPausePilotCommand, WindowsSealedAudioRun,
    };
    use record_recovery::{Checkpoint, MediaFacts, RecordingStream, TerminalDisposition};
    use std::{cell::RefCell, rc::Rc};

    struct Artifacts;
    impl WindowsPauseArtifactVerifier for Artifacts {
        fn verify(&self, _: &Checkpoint) -> Result<(), WindowsPauseAdapterError> {
            Ok(())
        }
        fn verify_audio(&self, _: &WindowsSealedAudioRun) -> Result<(), WindowsPauseAdapterError> {
            Ok(())
        }
    }
    fn run(
        sequence: u64,
        start: u64,
        end: u64,
    ) -> record_capture::windows_pause_pilot::WindowsSealedScreenRun {
        let mut run = native_run(start, end);
        run.range.first_physical_generation = sequence + 1;
        run.range.last_physical_generation = sequence + 1;
        run.range.first_checkpoint_sequence = sequence;
        run.range.last_checkpoint_sequence = sequence;
        let checkpoint = &mut run.checkpoints[0];
        checkpoint.physical_generation = sequence + 1;
        checkpoint.checkpoint.sequence = sequence;
        checkpoint.checkpoint.file = format!("checkpoints/segment-{sequence:06}.mp4");
        checkpoint.checkpoint.media = Some(MediaFacts {
            duration_ms: end - start,
            decoded_video_frames: 1,
            has_audio: false,
            width: None,
            height: None,
            codec_name: None,
            avg_frame_rate: None,
            r_frame_rate: None,
        });
        run
    }
    fn audio(generation: u64, start: u64, end: u64) -> Vec<WindowsSealedAudioRun> {
        [
            RecordingStream::MicrophoneAudio,
            RecordingStream::SystemAudio,
        ]
        .into_iter()
        .map(|stream| WindowsSealedAudioRun {
            stream,
            source_generation: generation,
            artifact: format!(
                "recording-{}-generation-{generation:020}.wav",
                if stream == RecordingStream::MicrophoneAudio {
                    "microphone"
                } else {
                    "system"
                }
            ),
            bytes: 48,
            sha256: "a".repeat(64),
            media_duration_ms: end - start,
            native_ready_unix_ms: 1000 + start,
            native_ready_raw_ms: start,
            raw_start_ms: start,
            raw_end_ms: end,
        })
        .collect()
    }
    // Normal paused Stop, active Stop, Stop winning an in-flight Pause, and
    // already queued Pause evidence retained by terminal Stop.
    for (second_pause, pending_stop, queued_pause) in [
        (true, false, false),
        (false, false, false),
        (false, true, false),
        (false, true, true),
    ] {
        let origin = Instant::now();
        let streams = SelectedCaptureStreams::new(true, true, false, false);
        let (command_tx, commands, events, event_rx) = channel();
        let log = Rc::new(RefCell::new(Vec::new()));
        let journal = MemoryJournal::new(&log, &streams);
        let mut first = started(origin, 0);
        first.physical_generation = 1;
        events
            .send(WindowsPausePilotEvent::Started { started: first })
            .unwrap();
        let mut session = WindowsPauseSession::from_started_lifecycle(
            journal,
            &WindowsPauseSessionAdmission::admit(
                Target {
                    legacy_index: None,
                    exact_id: Some(format!("shellx-monitor-v1:windows:{}", "a".repeat(64))),
                },
                streams,
                30.0,
                100,
            )
            .unwrap(),
            Lifecycle::new(command_tx, event_rx, log.clone()),
            CalibratedWindowsPauseEvidenceFactory::new(Artifacts),
        )
        .unwrap();
        session.request_pause_at(at(origin, 100)).unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            Some(WindowsPausePilotCommand::Pause {
                generation: 1,
                epoch: 1
            })
        ));
        events
            .send(WindowsPausePilotEvent::PauseSealed {
                generation: 1,
                epoch: 1,
                run: run(0, 17, 137),
                input: None,
                audio: audio(1, 17, 137),
                observed_at: at(origin, 160),
            })
            .unwrap();
        assert_eq!(
            session.pump_once().unwrap(),
            Some(WindowsPauseSessionEvent::Paused)
        );
        session.request_resume_at(at(origin, 180)).unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            Some(WindowsPausePilotCommand::Resume {
                generation: 2,
                epoch: 2
            })
        ));
        let mut resumed = started(origin, 200);
        resumed.physical_generation = 2;
        events
            .send(WindowsPausePilotEvent::ResumeReady {
                generation: 2,
                epoch: 2,
                started: resumed,
            })
            .unwrap();
        assert_eq!(
            session.pump_once().unwrap(),
            Some(WindowsPauseSessionEvent::Resumed)
        );
        if second_pause || pending_stop {
            session.request_pause_at(at(origin, 300)).unwrap();
            assert!(matches!(
                commands.try_recv().unwrap(),
                Some(WindowsPausePilotCommand::Pause {
                    generation: 3,
                    epoch: 3
                })
            ));
            if second_pause {
                events
                    .send(WindowsPausePilotEvent::PauseSealed {
                        generation: 3,
                        epoch: 3,
                        run: run(1, 217, 317),
                        input: None,
                        audio: audio(2, 217, 317),
                        observed_at: at(origin, 330),
                    })
                    .unwrap();
                assert_eq!(
                    session
                        .pump_once()
                        .expect("real resumed Pause boundary3 must seal readiness2"),
                    Some(WindowsPauseSessionEvent::Paused)
                );
            }
        }
        session.request_stop_at(at(origin, 350)).unwrap();
        let epoch = if second_pause || pending_stop { 4 } else { 3 };
        assert!(
            matches!(commands.try_recv().unwrap(), Some(WindowsPausePilotCommand::Stop { epoch: actual }) if actual == epoch)
        );
        if queued_pause {
            events
                .send(WindowsPausePilotEvent::PauseSealed {
                    generation: 3,
                    epoch: 3,
                    run: run(1, 217, 317),
                    input: None,
                    audio: audio(2, 217, 317),
                    observed_at: at(origin, 330),
                })
                .unwrap();
            assert_eq!(
                session.pump_once().unwrap(),
                Some(WindowsPauseSessionEvent::StopRunRetained)
            );
        }
        events
            .send(WindowsPausePilotEvent::StopSealed {
                epoch,
                run: (!second_pause && !queued_pause).then(|| run(1, 217, 417)),
                input: None,
                audio: if second_pause || queued_pause {
                    Vec::new()
                } else {
                    audio(2, 217, 417)
                },
                observed_at: at(origin, 430),
            })
            .unwrap();
        assert_eq!(
            session.pump_once().unwrap(),
            Some(WindowsPauseSessionEvent::Stopped)
        );
        assert_eq!(session.journal().sealed_runs().len(), 2);
        assert_eq!(
            session.journal().terminal().unwrap().disposition,
            TerminalDisposition::Completed
        );
        assert_eq!(&*log.borrow(), &["intent", "native", "join"]);
    }
}
