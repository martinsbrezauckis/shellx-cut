use std::time::{Duration, Instant};

use super::test_support::*;
use super::*;
use crate::windows_pause_pilot::{
    channel, WindowsPausePilotCheckpointRange, WindowsPausePilotOperation, WindowsPausePilotRequest,
};

#[test]
fn admission_refuses_every_non_screen_only_setting_without_dropping_it() {
    let mut invalid = WindowsPausePilotRequest::screen_video_only(exact_monitor_id(), 30.0);
    invalid.audio = true;
    assert_eq!(
        WindowsPausePilotProfile::admit(invalid),
        Err(WindowsPausePilotRefusal::Audio)
    );

    let bad_fps = WindowsPausePilotRequest::screen_video_only(exact_monitor_id(), 29.97);
    assert_eq!(
        WindowsPausePilotProfile::admit(bad_fps),
        Err(WindowsPausePilotRefusal::IntegerFpsRequired)
    );

    let malformed_id = WindowsPausePilotRequest::screen_video_only(
        "shellx-monitor-v1:windows:not-a-sha256-digest".into(),
        30.0,
    );
    assert_eq!(
        WindowsPausePilotProfile::admit(malformed_id),
        Err(WindowsPausePilotRefusal::ExactMonitorRequired)
    );
}

#[test]
fn stop_wins_queued_pause_before_native_wgc_work_begins() {
    let (mut worker, log) = start_worker(vec![Some("monitor:first")], false);
    let (commands, receiver, sender, events) = channel();
    commands
        .send(WindowsPausePilotCommand::Pause {
            generation: 84,
            epoch: 1,
        })
        .unwrap();
    commands
        .send(WindowsPausePilotCommand::Stop { epoch: 2 })
        .unwrap();
    process(&mut worker, &receiver, &sender, 50, 60);

    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::StopSealed {
            epoch: 2,
            run: Some(WindowsSealedScreenRun {
                range: WindowsPausePilotCheckpointRange {
                    first_physical_generation: 1,
                    last_physical_generation: 1,
                    first_checkpoint_sequence: 0,
                    last_checkpoint_sequence: 0,
                },
                ..
            }),
            ..
        }
    ));
    assert_eq!(events.try_recv().unwrap(), None);
    assert_eq!(
        log.borrow().as_slice(),
        ["reserve", "start:monitor:first", "close", "publish"]
    );
}

#[test]
fn two_pause_resume_logical_sessions_do_not_depend_on_physical_checkpoint_count() {
    for (pause_generation, resume_generation) in [(41, 99), (7, 8)] {
        let (mut worker, _, initial_started) = start_worker_with_started(
            vec![Some("monitor:first"), Some("monitor:resolved-again")],
            false,
        );
        let (commands, receiver, sender, events) = channel();
        commands
            .send(WindowsPausePilotCommand::Pause {
                generation: pause_generation,
                epoch: 1,
            })
            .unwrap();
        process(&mut worker, &receiver, &sender, 50, 60);
        let WindowsPausePilotEvent::PauseSealed {
            generation, run, ..
        } = events.try_recv().unwrap().unwrap()
        else {
            panic!("pause must seal one logical checkpoint range")
        };
        assert_eq!(generation, pause_generation);
        assert_eq!(run.range.first_physical_generation, 1);
        assert_eq!(run.range.last_physical_generation, 1);
        assert_eq!(run.checkpoints.len(), 1);
        assert_eq!(run.accepted.settings.fps, 30.0);
        assert_eq!(run.observed_start_ms, initial_started.observed_start_ms);

        commands
            .send(WindowsPausePilotCommand::Resume {
                generation: resume_generation,
                epoch: 2,
            })
            .unwrap();
        process(&mut worker, &receiver, &sender, 70, 80);
        let WindowsPausePilotEvent::ResumeReady {
            generation,
            started,
            ..
        } = events.try_recv().unwrap().unwrap()
        else {
            panic!("resume must report a new physical WGC start")
        };
        assert_eq!(generation, resume_generation);
        assert_eq!(started.physical_generation, 2);
        assert_eq!(started.observed_start_ms, 70);
        assert_eq!(started.accepted.range.width, 640);

        commands
            .send(WindowsPausePilotCommand::Pause {
                generation: pause_generation + 100,
                epoch: 3,
            })
            .unwrap();
        process(&mut worker, &receiver, &sender, 90, 100);
        let WindowsPausePilotEvent::PauseSealed { run, .. } = events.try_recv().unwrap().unwrap()
        else {
            panic!("fresh resumed logical run must seal")
        };
        assert_eq!(run.range.first_physical_generation, 2);
        assert_eq!(run.range.last_physical_generation, 2);
        assert_eq!(run.range.first_checkpoint_sequence, 1);
        assert_eq!(run.range.last_checkpoint_sequence, 1);
        assert_eq!(run.observed_start_ms, started.observed_start_ms);
        assert_eq!(run.observed_end_ms, 100);
    }
}

#[test]
fn resume_refuses_a_missing_exact_target_without_fallback() {
    let (mut worker, _) = start_worker(vec![Some("monitor:first"), None], false);
    let (commands, receiver, sender, events) = channel();
    commands
        .send(WindowsPausePilotCommand::Pause {
            generation: 1,
            epoch: 1,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 50, 60);
    let _ = events.try_recv().unwrap().unwrap();

    commands
        .send(WindowsPausePilotCommand::Resume {
            generation: 17,
            epoch: 2,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 70, 80);
    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::ResumeRefused {
            generation: 17,
            epoch: 2,
            refusal: WindowsPausePilotRefusal::TargetUnavailable,
            ..
        }
    ));
}

#[test]
fn peer_loss_closes_and_reaps_the_owned_active_wgc_worker() {
    let (mut worker, log) = start_worker(vec![Some("monitor:first")], false);
    let (commands, receiver, sender, _events) = channel();
    drop(commands);
    assert_eq!(
        worker.run_until_terminal(
            &receiver,
            &sender,
            Duration::from_millis(1),
            || 50,
            || observation(50),
            || (60, Instant::now()),
        ),
        Err(WindowsPausePilotChannelError::Disconnected)
    );
    assert_eq!(
        log.borrow().as_slice(),
        ["reserve", "start:monitor:first", "close", "publish"]
    );
}

#[test]
fn failed_publication_blocks_later_resume_and_never_emits_a_seal() {
    let (mut worker, _) = start_worker(vec![Some("monitor:first")], true);
    let (commands, receiver, sender, events) = channel();
    commands
        .send(WindowsPausePilotCommand::Pause {
            generation: 1,
            epoch: 1,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 50, 60);
    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::Failed {
            operation: WindowsPausePilotOperation::Pause,
            ..
        }
    ));

    commands
        .send(WindowsPausePilotCommand::Resume {
            generation: 2,
            epoch: 2,
        })
        .unwrap();
    process(&mut worker, &receiver, &sender, 70, 80);
    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::Failed {
            operation: WindowsPausePilotOperation::Resume,
            ..
        }
    ));
}
