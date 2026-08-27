use crate::{
    AcknowledgementRejection, AcknowledgementResult, BoundaryRequestResult, PauseStreamCoordinator,
    RecordingStream, SelectedCaptureStreams, SessionPhase, SessionTransitionIgnored,
    StreamAcknowledgement, StreamBoundary, StreamBoundaryKind, StreamRefusal,
};
use std::time::{Duration, Instant};

fn at(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

fn started(streams: SelectedCaptureStreams, origin: Instant) -> PauseStreamCoordinator {
    let mut coordinator = PauseStreamCoordinator::new(streams, None);
    assert!(coordinator.start_at(origin).changed());
    coordinator
}

fn issued(result: BoundaryRequestResult) -> StreamBoundary {
    match result {
        BoundaryRequestResult::Issued(boundary) => boundary,
        other => panic!("expected an issued boundary, got {other:?}"),
    }
}

fn pause_sealed(stream: RecordingStream, generation: u64) -> StreamAcknowledgement {
    StreamAcknowledgement::PauseSealed { stream, generation }
}

fn resume_ready(stream: RecordingStream, generation: u64) -> StreamAcknowledgement {
    StreamAcknowledgement::ResumeReady { stream, generation }
}

#[test]
fn selected_stream_registration_is_immutable_exact_and_uses_recovery_taxonomy() {
    let selected = SelectedCaptureStreams::new(true, false, true, false);
    assert_eq!(
        selected.streams(),
        &[
            RecordingStream::ScreenVideo,
            RecordingStream::MicrophoneAudio,
            RecordingStream::CameraVideo,
        ]
    );
    assert_eq!(
        SelectedCaptureStreams::screen_only().streams(),
        &[RecordingStream::ScreenVideo]
    );

    let origin = Instant::now();
    let mut coordinator = started(selected.clone(), origin);
    let boundary = issued(coordinator.request_pause_at(at(origin, 10)));
    assert_eq!(boundary.kind, StreamBoundaryKind::Pause);
    assert_eq!(boundary.generation, 1);
    assert_eq!(boundary.streams(), selected.streams());
    assert_eq!(
        coordinator
            .status_at(at(origin, 10))
            .pending_boundary
            .unwrap()
            .awaiting_streams,
        selected.streams()
    );

    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::InputEvents, 1),
            at(origin, 11)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::UnknownStream {
            stream: RecordingStream::InputEvents,
        })
    );
    assert_eq!(coordinator.phase(), SessionPhase::Pausing);
}

#[test]
fn delayed_audio_acknowledgement_holds_pause_until_every_selected_stream_seals() {
    let origin = Instant::now();
    let mut coordinator = started(SelectedCaptureStreams::new(true, true, false, true), origin);
    assert_eq!(
        coordinator.timestamp_at(at(origin, 40)),
        Some(Duration::from_millis(40))
    );

    let pause = issued(coordinator.request_pause_at(at(origin, 100)));
    assert_eq!(pause.generation, 1);
    assert_eq!(coordinator.timestamp_at(at(origin, 100)), None);
    assert_eq!(
        coordinator.request_pause_at(at(origin, 101)),
        BoundaryRequestResult::Ignored {
            phase: SessionPhase::Pausing,
            reason: SessionTransitionIgnored::AlreadyApplied,
        }
    );
    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::ScreenVideo, 1),
            at(origin, 110)
        ),
        AcknowledgementResult::Accepted {
            phase: SessionPhase::Pausing,
            completed_boundary: false,
        }
    );
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::InputEvents, 1),
        at(origin, 120),
    );
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::MicrophoneAudio, 1),
        at(origin, 500),
    );
    assert_eq!(
        coordinator
            .status_at(at(origin, 900))
            .pending_boundary
            .unwrap()
            .awaiting_streams,
        vec![RecordingStream::SystemAudio]
    );
    assert_eq!(coordinator.timestamp_at(at(origin, 900)), None);

    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::SystemAudio, 1),
            at(origin, 1_000)
        ),
        AcknowledgementResult::Accepted {
            phase: SessionPhase::Paused,
            completed_boundary: true,
        }
    );
    assert_eq!(
        coordinator.timestamp_at(at(origin, 2_000)),
        None,
        "delayed audio must not reopen logical timestamps"
    );

    let resume = issued(coordinator.request_resume_at(at(origin, 3_000)));
    assert_eq!(resume.generation, 2);
    assert_eq!(
        coordinator.request_resume_at(at(origin, 3_001)),
        BoundaryRequestResult::Ignored {
            phase: SessionPhase::Resuming,
            reason: SessionTransitionIgnored::AlreadyApplied,
        }
    );
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::MicrophoneAudio, 2),
        at(origin, 3_010),
    );
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::InputEvents, 2),
        at(origin, 3_020),
    );
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::ScreenVideo, 2),
        at(origin, 3_030),
    );
    assert_eq!(coordinator.timestamp_at(at(origin, 3_500)), None);
    assert_eq!(
        coordinator.acknowledge_at(
            resume_ready(RecordingStream::SystemAudio, 2),
            at(origin, 4_000)
        ),
        AcknowledgementResult::Accepted {
            phase: SessionPhase::Recording,
            completed_boundary: true,
        }
    );
    assert_eq!(
        coordinator.timestamp_at(at(origin, 4_010)),
        Some(Duration::from_millis(110))
    );
}

#[test]
fn a_resume_refusal_returns_to_safely_paused_without_faking_completion() {
    let origin = Instant::now();
    let mut coordinator = started(
        SelectedCaptureStreams::new(true, false, false, false),
        origin,
    );
    let pause = issued(coordinator.request_pause_at(at(origin, 25)));
    assert_eq!(pause.generation, 1);
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::ScreenVideo, 1),
        at(origin, 30),
    );
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::MicrophoneAudio, 1),
        at(origin, 35),
    );
    assert_eq!(coordinator.phase(), SessionPhase::Paused);

    let resume = issued(coordinator.request_resume_at(at(origin, 100)));
    assert_eq!(resume.generation, 2);
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::ScreenVideo, 2),
        at(origin, 110),
    );
    assert_eq!(
        coordinator.acknowledge_at(
            StreamAcknowledgement::ResumeRefused {
                stream: RecordingStream::MicrophoneAudio,
                generation: 2,
            },
            at(origin, 120)
        ),
        AcknowledgementResult::ResumeRefused(StreamRefusal {
            stream: RecordingStream::MicrophoneAudio,
            generation: 2,
        })
    );
    let status = coordinator.status_at(at(origin, 130));
    assert_eq!(status.phase, SessionPhase::Paused);
    assert_eq!(status.pending_boundary, None);
    assert_eq!(
        status.last_resume_refusal,
        Some(StreamRefusal {
            stream: RecordingStream::MicrophoneAudio,
            generation: 2,
        })
    );
    assert!(!status.timestamp_issuance_enabled);
    assert_eq!(
        coordinator.acknowledge_at(
            resume_ready(RecordingStream::MicrophoneAudio, 2),
            at(origin, 131)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::StaleGeneration {
            expected: 2,
            received: 2,
        })
    );

    let retry = issued(coordinator.request_resume_at(at(origin, 200)));
    assert_eq!(retry.generation, 3);
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::MicrophoneAudio, 3),
        at(origin, 210),
    );
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::ScreenVideo, 3),
        at(origin, 220),
    );
    assert_eq!(coordinator.phase(), SessionPhase::Recording);
    assert_eq!(
        coordinator.status_at(at(origin, 220)).last_resume_refusal,
        None
    );
}

#[test]
fn ready_then_refused_for_one_generation_is_a_duplicate_not_a_resume_abort() {
    let origin = Instant::now();
    let mut coordinator = started(
        SelectedCaptureStreams::new(true, false, false, false),
        origin,
    );
    coordinator.request_pause_at(at(origin, 10));
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::ScreenVideo, 1),
        at(origin, 11),
    );
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::MicrophoneAudio, 1),
        at(origin, 12),
    );
    coordinator.request_resume_at(at(origin, 20));
    assert_eq!(
        coordinator.acknowledge_at(
            resume_ready(RecordingStream::MicrophoneAudio, 2),
            at(origin, 21)
        ),
        AcknowledgementResult::Accepted {
            phase: SessionPhase::Resuming,
            completed_boundary: false,
        }
    );
    assert_eq!(
        coordinator.acknowledge_at(
            StreamAcknowledgement::ResumeRefused {
                stream: RecordingStream::MicrophoneAudio,
                generation: 2,
            },
            at(origin, 22)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::DuplicateStream {
            stream: RecordingStream::MicrophoneAudio,
            generation: 2,
        })
    );
    assert_eq!(coordinator.phase(), SessionPhase::Resuming);
    assert_eq!(
        coordinator.acknowledge_at(
            resume_ready(RecordingStream::ScreenVideo, 2),
            at(origin, 23)
        ),
        AcknowledgementResult::Accepted {
            phase: SessionPhase::Recording,
            completed_boundary: true,
        }
    );
}

#[test]
fn stale_duplicate_and_out_of_order_acknowledgements_are_rejected_deterministically() {
    let origin = Instant::now();
    let mut coordinator = started(
        SelectedCaptureStreams::new(true, false, false, false),
        origin,
    );
    let pause = issued(coordinator.request_pause_at(at(origin, 10)));

    assert_eq!(
        coordinator.acknowledge_at(
            resume_ready(RecordingStream::ScreenVideo, 1),
            at(origin, 11)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::WrongBoundary {
            expected: StreamBoundaryKind::Pause,
            received: StreamBoundaryKind::Resume,
        })
    );
    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::ScreenVideo, 2),
            at(origin, 12)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::FutureGeneration {
            expected: 1,
            received: 2,
        })
    );
    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::CameraVideo, 1),
            at(origin, 13)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::UnknownStream {
            stream: RecordingStream::CameraVideo,
        })
    );
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::ScreenVideo, 1),
        at(origin, 14),
    );
    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::ScreenVideo, 1),
            at(origin, 15)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::DuplicateStream {
            stream: RecordingStream::ScreenVideo,
            generation: pause.generation,
        })
    );
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::MicrophoneAudio, 1),
        at(origin, 16),
    );
    assert_eq!(coordinator.phase(), SessionPhase::Paused);
    assert_eq!(
        coordinator.acknowledge_at(
            pause_sealed(RecordingStream::ScreenVideo, 1),
            at(origin, 17)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::StaleGeneration {
            expected: 1,
            received: 1,
        })
    );
}

#[test]
fn stop_is_terminal_during_pausing_and_resuming() {
    let origin = Instant::now();
    let mut pausing = started(
        SelectedCaptureStreams::new(true, false, false, false),
        origin,
    );
    pausing.request_pause_at(at(origin, 10));
    assert!(pausing.stop_at(at(origin, 20)).changed());
    assert_eq!(pausing.phase(), SessionPhase::Stopped);
    assert_eq!(
        pausing.acknowledge_at(
            pause_sealed(RecordingStream::ScreenVideo, 1),
            at(origin, 21)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::Stopped)
    );
    assert_eq!(pausing.timestamp_at(at(origin, 1_000)), None);

    let mut resuming = started(
        SelectedCaptureStreams::new(false, false, false, false),
        origin,
    );
    resuming.request_pause_at(at(origin, 30));
    resuming.acknowledge_at(
        pause_sealed(RecordingStream::ScreenVideo, 1),
        at(origin, 31),
    );
    resuming.request_resume_at(at(origin, 40));
    assert_eq!(resuming.phase(), SessionPhase::Resuming);
    assert!(resuming.stop_at(at(origin, 50)).changed());
    assert_eq!(resuming.phase(), SessionPhase::Stopped);
    assert_eq!(
        resuming.acknowledge_at(
            resume_ready(RecordingStream::ScreenVideo, 2),
            at(origin, 51)
        ),
        AcknowledgementResult::Rejected(AcknowledgementRejection::Stopped)
    );
}

#[test]
fn active_duration_excludes_paused_and_transition_wall_time_without_auto_stop() {
    let origin = Instant::now();
    let mut coordinator = PauseStreamCoordinator::new(
        SelectedCaptureStreams::screen_only(),
        Some(Duration::from_millis(100)),
    );
    coordinator.start_at(origin);
    coordinator.request_pause_at(at(origin, 40));
    coordinator.acknowledge_at(
        pause_sealed(RecordingStream::ScreenVideo, 1),
        at(origin, 50),
    );

    let paused = coordinator.status_at(at(origin, 10_000));
    assert_eq!(paused.logical_elapsed_ms, 40);
    assert_eq!(paused.remaining_active_duration_ms, Some(60));
    assert!(!paused.active_duration_exhausted);

    coordinator.request_resume_at(at(origin, 10_010));
    coordinator.acknowledge_at(
        resume_ready(RecordingStream::ScreenVideo, 2),
        at(origin, 10_020),
    );
    assert!(!coordinator.active_duration_exhausted_at(at(origin, 10_079)));
    assert!(coordinator.active_duration_exhausted_at(at(origin, 10_080)));
    assert_eq!(
        coordinator.timestamp_at(at(origin, 10_080)),
        Some(Duration::from_millis(100))
    );
    assert_eq!(
        coordinator.phase(),
        SessionPhase::Recording,
        "duration exhaustion is an observation, not fake worker completion"
    );
}
