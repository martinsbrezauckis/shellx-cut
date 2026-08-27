use super::pause_worker_protocol::{
    ObservedBoundaryIdentity, PauseWorkerCommand, PauseWorkerCoordinator, PauseWorkerFact,
    PauseWorkerFactRejection, PauseWorkerFactResult, PauseWorkerProtocolPhase, WorkerEpoch,
};
use record_capture::{RecordingStream, SelectedCaptureStreams};
use std::time::{Duration, Instant};

fn at(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

fn selected() -> SelectedCaptureStreams {
    SelectedCaptureStreams::new(true, false, false, true)
}

fn boundary(command: &PauseWorkerCommand, observed_at: Instant) -> ObservedBoundaryIdentity {
    ObservedBoundaryIdentity::observed(command.epoch(), observed_at)
}

fn generation(command: &PauseWorkerCommand) -> u64 {
    command
        .generation()
        .expect("pause and resume commands have a generation")
}

fn pause_sealed(
    command: &PauseWorkerCommand,
    stream: RecordingStream,
    observed_at: Instant,
) -> PauseWorkerFact {
    PauseWorkerFact::PauseSealed {
        stream,
        generation: generation(command),
        observed_boundary: boundary(command, observed_at),
    }
}

fn resume_ready(
    command: &PauseWorkerCommand,
    stream: RecordingStream,
    observed_at: Instant,
) -> PauseWorkerFact {
    PauseWorkerFact::ResumeReady {
        stream,
        generation: generation(command),
        observed_boundary: boundary(command, observed_at),
    }
}

fn seal_pause(
    coordinator: &mut PauseWorkerCoordinator,
    command: &PauseWorkerCommand,
    origin: Instant,
) {
    for (index, stream) in command.streams().iter().copied().enumerate() {
        let result = coordinator.accept_fact(pause_sealed(
            command,
            stream,
            at(origin, 20 + u64::try_from(index).unwrap()),
        ));
        assert!(matches!(result, PauseWorkerFactResult::Accepted { .. }));
    }
}

#[test]
fn pause_and_resume_commands_fan_out_to_the_exact_immutable_stream_set() {
    let origin = Instant::now();
    let mut coordinator = PauseWorkerCoordinator::new(selected());
    let pause = coordinator.issue_pause_at(at(origin, 10)).unwrap();

    assert_eq!(pause.generation(), Some(1));
    assert_eq!(pause.epoch().value(), 1);
    assert_eq!(
        pause.streams(),
        &[
            RecordingStream::ScreenVideo,
            RecordingStream::MicrophoneAudio,
            RecordingStream::InputEvents,
        ]
    );
    seal_pause(&mut coordinator, &pause, origin);
    assert_eq!(coordinator.phase(), PauseWorkerProtocolPhase::Paused);

    let resume = coordinator.issue_resume_at(at(origin, 40)).unwrap();
    assert_eq!(resume.generation(), Some(2));
    assert_eq!(resume.epoch().value(), 2);
    for (index, stream) in resume.streams().iter().copied().enumerate() {
        let result = coordinator.accept_fact(resume_ready(
            &resume,
            stream,
            at(origin, 50 + u64::try_from(index).unwrap()),
        ));
        assert!(matches!(result, PauseWorkerFactResult::Accepted { .. }));
    }
    assert_eq!(coordinator.phase(), PauseWorkerProtocolPhase::Recording);
}

#[test]
fn facts_reject_wrong_stream_kind_generation_epoch_and_duplicate_deterministically() {
    let origin = Instant::now();
    let mut coordinator = PauseWorkerCoordinator::new(selected());
    let pause = coordinator.issue_pause_at(at(origin, 10)).unwrap();

    assert_eq!(
        coordinator.accept_fact(pause_sealed(
            &pause,
            RecordingStream::CameraVideo,
            at(origin, 11)
        )),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::WrongStream {
            stream: RecordingStream::CameraVideo,
        })
    );
    assert_eq!(
        coordinator.accept_fact(resume_ready(
            &pause,
            RecordingStream::ScreenVideo,
            at(origin, 12)
        )),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::WrongBoundary {
            expected: super::pause_worker_protocol::WorkerBoundaryKind::Pause,
            received: super::pause_worker_protocol::WorkerBoundaryKind::Resume,
        })
    );
    assert_eq!(
        coordinator.accept_fact(PauseWorkerFact::PauseSealed {
            stream: RecordingStream::ScreenVideo,
            generation: 2,
            observed_boundary: boundary(&pause, at(origin, 13)),
        }),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::FutureGeneration {
            expected: 1,
            received: 2,
        })
    );
    assert_eq!(
        coordinator.accept_fact(pause_sealed(
            &pause,
            RecordingStream::ScreenVideo,
            at(origin, 14)
        )),
        PauseWorkerFactResult::Accepted {
            phase: PauseWorkerProtocolPhase::PausePending,
            completed_epoch: false,
        }
    );
    assert_eq!(
        coordinator.accept_fact(pause_sealed(
            &pause,
            RecordingStream::ScreenVideo,
            at(origin, 15)
        )),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::DuplicateStream {
            stream: RecordingStream::ScreenVideo,
            generation: 1,
            epoch: pause.epoch(),
        })
    );

    let stale_epoch = WorkerEpoch::new(0);
    assert_eq!(
        coordinator.accept_fact(PauseWorkerFact::PauseSealed {
            stream: RecordingStream::MicrophoneAudio,
            generation: 1,
            observed_boundary: ObservedBoundaryIdentity::observed(stale_epoch, at(origin, 16)),
        }),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::StaleEpoch {
            expected: pause.epoch(),
            received: stale_epoch,
        })
    );
    let future_epoch = WorkerEpoch::new(2);
    assert_eq!(
        coordinator.accept_fact(PauseWorkerFact::PauseSealed {
            stream: RecordingStream::MicrophoneAudio,
            generation: 1,
            observed_boundary: ObservedBoundaryIdentity::observed(future_epoch, at(origin, 17)),
        }),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::FutureEpoch {
            expected: pause.epoch(),
            received: future_epoch,
        })
    );
}

#[test]
fn completed_epoch_rejects_late_and_stale_facts_without_reopening_the_boundary() {
    let origin = Instant::now();
    let mut coordinator = PauseWorkerCoordinator::new(selected());
    let pause = coordinator.issue_pause_at(at(origin, 10)).unwrap();
    seal_pause(&mut coordinator, &pause, origin);

    assert_eq!(
        coordinator.accept_fact(pause_sealed(
            &pause,
            RecordingStream::ScreenVideo,
            at(origin, 30)
        )),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::LateEpoch {
            generation: 1,
            epoch: pause.epoch(),
        })
    );

    let resume = coordinator.issue_resume_at(at(origin, 40)).unwrap();
    assert_eq!(
        coordinator.accept_fact(PauseWorkerFact::ResumeReady {
            stream: RecordingStream::ScreenVideo,
            generation: generation(&pause),
            observed_boundary: boundary(&resume, at(origin, 41)),
        }),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::StaleGeneration {
            expected: generation(&resume),
            received: generation(&pause),
        })
    );
    assert_eq!(coordinator.phase(), PauseWorkerProtocolPhase::ResumePending);
}

#[test]
fn physical_fact_must_name_a_boundary_observed_after_its_command() {
    let origin = Instant::now();
    let mut coordinator = PauseWorkerCoordinator::new(selected());
    let pause = coordinator.issue_pause_at(at(origin, 10)).unwrap();

    assert_eq!(
        coordinator.accept_fact(pause_sealed(
            &pause,
            RecordingStream::ScreenVideo,
            at(origin, 9)
        )),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::PredatesCommand {
            epoch: pause.epoch(),
        })
    );
}

#[test]
fn refused_and_failed_facts_are_explicit_and_block_further_boundaries() {
    let origin = Instant::now();
    let mut refused = PauseWorkerCoordinator::new(selected());
    let pause = refused.issue_pause_at(at(origin, 10)).unwrap();
    assert_eq!(
        refused.accept_fact(PauseWorkerFact::Refused {
            stream: RecordingStream::InputEvents,
            command: super::pause_worker_protocol::WorkerBoundaryKind::Pause,
            generation: generation(&pause),
            observed_boundary: boundary(&pause, at(origin, 11)),
        }),
        PauseWorkerFactResult::Refused {
            stream: RecordingStream::InputEvents,
            command: super::pause_worker_protocol::WorkerBoundaryKind::Pause,
            generation: 1,
            epoch: pause.epoch(),
        }
    );
    assert_eq!(refused.phase(), PauseWorkerProtocolPhase::Blocked);

    let mut failed = PauseWorkerCoordinator::new(selected());
    let failed_pause = failed.issue_pause_at(at(origin, 20)).unwrap();
    assert_eq!(
        failed.accept_fact(PauseWorkerFact::Failed {
            stream: RecordingStream::ScreenVideo,
            command: super::pause_worker_protocol::WorkerBoundaryKind::Pause,
            generation: generation(&failed_pause),
            observed_boundary: boundary(&failed_pause, at(origin, 21)),
        }),
        PauseWorkerFactResult::Failed {
            stream: RecordingStream::ScreenVideo,
            command: super::pause_worker_protocol::WorkerBoundaryKind::Pause,
            generation: 1,
            epoch: failed_pause.epoch(),
        }
    );
    assert_eq!(failed.phase(), PauseWorkerProtocolPhase::Blocked);
}

#[test]
fn resume_refusal_returns_to_paused_and_requires_a_fresh_generation() {
    let origin = Instant::now();
    let mut coordinator = PauseWorkerCoordinator::new(selected());
    let pause = coordinator.issue_pause_at(at(origin, 10)).unwrap();
    seal_pause(&mut coordinator, &pause, origin);
    let resume = coordinator.issue_resume_at(at(origin, 30)).unwrap();

    assert!(matches!(
        coordinator.accept_fact(PauseWorkerFact::Refused {
            stream: RecordingStream::ScreenVideo,
            command: super::pause_worker_protocol::WorkerBoundaryKind::Resume,
            generation: generation(&resume),
            observed_boundary: boundary(&resume, at(origin, 31)),
        }),
        PauseWorkerFactResult::Refused { .. }
    ));
    assert_eq!(coordinator.phase(), PauseWorkerProtocolPhase::Paused);

    let retry = coordinator.issue_resume_at(at(origin, 40)).unwrap();
    assert_eq!(retry.generation(), Some(generation(&resume) + 1));
}

#[test]
fn stop_invalidates_pending_epochs_and_wins_over_every_late_fact() {
    let origin = Instant::now();
    let mut coordinator = PauseWorkerCoordinator::new(selected());
    let pause = coordinator.issue_pause_at(at(origin, 10)).unwrap();
    let stop = coordinator.issue_stop_at(at(origin, 11)).unwrap();

    assert_eq!(stop.generation(), None);
    assert_eq!(stop.epoch().value(), 2);
    assert_eq!(stop.streams(), pause.streams());
    assert_eq!(coordinator.phase(), PauseWorkerProtocolPhase::Stopped);
    assert_eq!(
        coordinator.accept_fact(pause_sealed(
            &pause,
            RecordingStream::ScreenVideo,
            at(origin, 12)
        )),
        PauseWorkerFactResult::Rejected(PauseWorkerFactRejection::PostStop)
    );
}
