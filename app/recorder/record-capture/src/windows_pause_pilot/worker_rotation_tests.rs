use std::time::{Duration, Instant};

use super::test_support::*;
use super::*;
use crate::windows_pause_pilot::{channel, WindowsPausePilotCheckpointRange};

#[test]
fn timeout_rotations_make_one_contiguous_logical_range_before_pause() {
    let (mut worker, log) = start_worker(vec![Some("monitor:first")], false);
    let (commands, receiver, sender, events) = channel();
    let mut starts = [20, 30].into_iter();
    let mut reservations = [20, 30].into_iter();
    let mut boundaries = 0;
    worker
        .run_until_terminal(
            &receiver,
            &sender,
            Duration::ZERO,
            || {
                reservations
                    .next()
                    .expect("two post-close reservations only")
            },
            || observation(starts.next().expect("two ordinary rotations only")),
            || {
                boundaries += 1;
                match boundaries {
                    1 => (20, Instant::now()),
                    2 => {
                        commands
                            .send(WindowsPausePilotCommand::Pause {
                                generation: 41,
                                epoch: 1,
                            })
                            .unwrap();
                        (30, Instant::now())
                    }
                    3 => {
                        commands
                            .send(WindowsPausePilotCommand::Stop { epoch: 2 })
                            .unwrap();
                        (40, Instant::now())
                    }
                    _ => (50, Instant::now()),
                }
            },
        )
        .unwrap();

    let WindowsPausePilotEvent::PauseSealed { run, .. } = events.try_recv().unwrap().unwrap()
    else {
        panic!("two timeout rotations must seal one logical pause run")
    };
    assert_eq!(run.observed_start_ms, 10);
    assert_eq!(run.observed_end_ms, 40);
    assert_eq!(run.checkpoints.len(), 3);
    assert_eq!(
        run.range,
        WindowsPausePilotCheckpointRange {
            first_physical_generation: 1,
            last_physical_generation: 3,
            first_checkpoint_sequence: 0,
            last_checkpoint_sequence: 2,
        }
    );
    assert_eq!(
        run.checkpoints
            .iter()
            .map(|checkpoint| checkpoint.checkpoint.sequence)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(run.checkpoints.iter().all(|checkpoint| {
        checkpoint.checkpoint.facts.start_ms == checkpoint.start_ms
            && checkpoint.checkpoint.facts.end_ms == checkpoint.end_ms
            && checkpoint.checkpoint.facts.event_offset_ms == checkpoint.start_ms
            && checkpoint.checkpoint.facts.audio_offset_ms.is_none()
    }));
    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::StopSealed { run: None, .. }
    ));
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|entry| *entry == "publish")
            .count(),
        3,
        "every accepted physical checkpoint is published exactly once"
    );
}

#[test]
fn timeout_rotations_make_one_contiguous_logical_range_before_stop() {
    let (mut worker, log) = start_worker(vec![Some("monitor:first")], false);
    let (commands, receiver, sender, events) = channel();
    let mut starts = [20, 30].into_iter();
    let mut reservations = [20, 30].into_iter();
    let mut boundaries = 0;
    worker
        .run_until_terminal(
            &receiver,
            &sender,
            Duration::ZERO,
            || {
                reservations
                    .next()
                    .expect("two post-close reservations only")
            },
            || observation(starts.next().expect("two ordinary rotations only")),
            || {
                boundaries += 1;
                match boundaries {
                    1 => (20, Instant::now()),
                    2 => {
                        commands
                            .send(WindowsPausePilotCommand::Stop { epoch: 2 })
                            .unwrap();
                        (30, Instant::now())
                    }
                    3 => (40, Instant::now()),
                    _ => (50, Instant::now()),
                }
            },
        )
        .unwrap();

    let WindowsPausePilotEvent::StopSealed { run: Some(run), .. } =
        events.try_recv().unwrap().unwrap()
    else {
        panic!("two timeout rotations must seal one logical stop run")
    };
    assert_eq!(run.observed_start_ms, 10);
    assert_eq!(run.observed_end_ms, 40);
    assert_eq!(run.checkpoints.len(), 3);
    assert_eq!(
        run.range,
        WindowsPausePilotCheckpointRange {
            first_physical_generation: 1,
            last_physical_generation: 3,
            first_checkpoint_sequence: 0,
            last_checkpoint_sequence: 2,
        }
    );
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|entry| *entry == "publish")
            .count(),
        3,
        "no rotated physical checkpoint is omitted from the terminal range"
    );
}

#[test]
fn accepted_settings_or_range_drift_is_terminal_after_reaping_wgc() {
    let (mut worker, log) = start_drift_worker(vec![accepted(), accepted_with_size(1280, 720)]);
    let (_commands, receiver, sender, events) = channel();
    assert_eq!(
        worker.run_until_terminal(
            &receiver,
            &sender,
            Duration::ZERO,
            || 30,
            || observation(30),
            || (30, Instant::now()),
        ),
        Err(WindowsPausePilotChannelError::NativeTerminal)
    );
    assert_eq!(events.try_recv().unwrap(), None);
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|entry| *entry == "close")
            .count(),
        2,
        "the drifted replacement control is closed and reaped"
    );
}

#[test]
fn blocking_worker_gives_queued_stop_dominance_over_pause_and_resume() {
    let (mut worker, log) = start_worker(
        vec![Some("monitor:first"), Some("monitor:must-not-resolve")],
        false,
    );
    let (commands, receiver, sender, events) = channel();
    commands
        .send(WindowsPausePilotCommand::Pause {
            generation: 3,
            epoch: 1,
        })
        .unwrap();
    commands
        .send(WindowsPausePilotCommand::Resume {
            generation: 4,
            epoch: 2,
        })
        .unwrap();
    commands
        .send(WindowsPausePilotCommand::Stop { epoch: 3 })
        .unwrap();

    worker
        .run_until_terminal(
            &receiver,
            &sender,
            Duration::from_secs(1),
            || panic!("Stop must clear queued pause/resume before reserve"),
            || panic!("Stop must clear queued pause/resume before restart"),
            || (60, Instant::now()),
        )
        .unwrap();

    assert!(matches!(
        events.try_recv().unwrap().unwrap(),
        WindowsPausePilotEvent::StopSealed {
            epoch: 3,
            run: Some(_),
            ..
        }
    ));
    assert_eq!(events.try_recv().unwrap(), None);
    assert_eq!(
        log.borrow().as_slice(),
        ["reserve", "start:monitor:first", "close", "publish"],
        "Pause and Resume must not start, seal, or emit before queued Stop"
    );
}
