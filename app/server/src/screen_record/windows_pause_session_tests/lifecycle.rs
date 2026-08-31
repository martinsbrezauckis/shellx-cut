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
