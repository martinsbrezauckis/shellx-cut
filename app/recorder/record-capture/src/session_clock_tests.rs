use crate::{
    LogicalSessionClock, SessionPhase, SessionTransition, SessionTransitionIgnored,
    SessionTransitionResult,
};
use std::time::{Duration, Instant};

fn at(origin: Instant, millis: u64) -> Instant {
    origin + Duration::from_millis(millis)
}

#[test]
fn timestamps_exist_only_during_recording() {
    let origin = Instant::now();
    let mut clock = LogicalSessionClock::default();

    assert_eq!(clock.phase(), SessionPhase::Preparing);
    assert_eq!(clock.timestamp_at(origin), None);
    assert_eq!(
        clock.apply_at(SessionTransition::Start, origin),
        SessionTransitionResult::Applied {
            from: SessionPhase::Preparing,
            to: SessionPhase::Recording,
        }
    );
    assert_eq!(
        clock.timestamp_at(at(origin, 25)),
        Some(Duration::from_millis(25))
    );

    assert!(clock
        .apply_at(SessionTransition::PauseRequested, at(origin, 25))
        .changed());
    assert_eq!(clock.phase(), SessionPhase::Pausing);
    assert_eq!(
        clock.logical_elapsed_at(at(origin, 3_000)),
        Duration::from_millis(25)
    );
    assert_eq!(clock.timestamp_at(at(origin, 3_000)), None);

    assert!(clock
        .apply_at(SessionTransition::PauseCompleted, at(origin, 3_100))
        .changed());
    assert_eq!(clock.phase(), SessionPhase::Paused);
    assert_eq!(clock.timestamp_at(at(origin, 3_100)), None);

    assert!(clock
        .apply_at(SessionTransition::ResumeRequested, at(origin, 4_000))
        .changed());
    assert_eq!(clock.phase(), SessionPhase::Resuming);
    assert_eq!(clock.timestamp_at(at(origin, 4_000)), None);

    assert!(clock
        .apply_at(SessionTransition::ResumeCompleted, at(origin, 4_200))
        .changed());
    assert_eq!(clock.phase(), SessionPhase::Recording);
    assert_eq!(
        clock.timestamp_at(at(origin, 4_250)),
        Some(Duration::from_millis(75))
    );
}

#[test]
fn repeated_pause_resume_edges_are_deterministic() {
    let origin = Instant::now();
    let mut clock = LogicalSessionClock::default();
    clock.apply_at(SessionTransition::Start, origin);

    clock.apply_at(SessionTransition::PauseRequested, at(origin, 10));
    assert_eq!(
        clock.apply_at(SessionTransition::PauseRequested, at(origin, 20)),
        SessionTransitionResult::Ignored {
            phase: SessionPhase::Pausing,
            reason: SessionTransitionIgnored::AlreadyApplied,
        }
    );
    clock.apply_at(SessionTransition::PauseCompleted, at(origin, 30));
    assert_eq!(
        clock.apply_at(SessionTransition::PauseCompleted, at(origin, 40)),
        SessionTransitionResult::Ignored {
            phase: SessionPhase::Paused,
            reason: SessionTransitionIgnored::AlreadyApplied,
        }
    );

    clock.apply_at(SessionTransition::ResumeRequested, at(origin, 50));
    assert_eq!(
        clock.apply_at(SessionTransition::ResumeRequested, at(origin, 60)),
        SessionTransitionResult::Ignored {
            phase: SessionPhase::Resuming,
            reason: SessionTransitionIgnored::AlreadyApplied,
        }
    );
    clock.apply_at(SessionTransition::ResumeCompleted, at(origin, 70));
    assert_eq!(
        clock.apply_at(SessionTransition::ResumeCompleted, at(origin, 80)),
        SessionTransitionResult::Ignored {
            phase: SessionPhase::Recording,
            reason: SessionTransitionIgnored::AlreadyApplied,
        }
    );
    assert_eq!(
        clock.timestamp_at(at(origin, 90)),
        Some(Duration::from_millis(30))
    );
}

#[test]
fn active_duration_budget_excludes_paused_wall_time() {
    let origin = Instant::now();
    let mut clock = LogicalSessionClock::new(Some(Duration::from_millis(100)));
    clock.apply_at(SessionTransition::Start, origin);
    clock.apply_at(SessionTransition::PauseRequested, at(origin, 40));
    clock.apply_at(SessionTransition::PauseCompleted, at(origin, 45));

    assert_eq!(
        clock.remaining_active_duration_at(at(origin, 10_000)),
        Some(Duration::from_millis(60))
    );
    assert!(!clock.active_duration_exhausted_at(at(origin, 10_000)));

    clock.apply_at(SessionTransition::ResumeRequested, at(origin, 10_010));
    clock.apply_at(SessionTransition::ResumeCompleted, at(origin, 10_020));
    assert!(!clock.active_duration_exhausted_at(at(origin, 10_079)));
    assert!(clock.active_duration_exhausted_at(at(origin, 10_080)));
    assert_eq!(
        clock.remaining_active_duration_at(at(origin, 10_080)),
        Some(Duration::ZERO)
    );
}

#[test]
fn stop_wins_over_late_pause_and_resume_edges() {
    let origin = Instant::now();
    let mut clock = LogicalSessionClock::default();
    clock.apply_at(SessionTransition::Start, origin);
    clock.apply_at(SessionTransition::PauseRequested, at(origin, 15));

    assert_eq!(
        clock.apply_at(SessionTransition::Stop, at(origin, 20)),
        SessionTransitionResult::Applied {
            from: SessionPhase::Pausing,
            to: SessionPhase::Stopped,
        }
    );
    assert_eq!(
        clock.logical_elapsed_at(at(origin, 1_000)),
        Duration::from_millis(15)
    );
    for transition in [
        SessionTransition::PauseCompleted,
        SessionTransition::ResumeRequested,
        SessionTransition::ResumeCompleted,
        SessionTransition::Stop,
    ] {
        assert_eq!(
            clock.apply_at(transition, at(origin, 1_001)),
            SessionTransitionResult::Ignored {
                phase: SessionPhase::Stopped,
                reason: SessionTransitionIgnored::Stopped,
            }
        );
    }
    assert_eq!(clock.timestamp_at(at(origin, 1_000)), None);
}

#[test]
fn resume_refusal_returns_to_paused_without_reopening_timestamps() {
    let origin = Instant::now();
    let mut clock = LogicalSessionClock::default();
    clock.apply_at(SessionTransition::Start, origin);
    clock.apply_at(SessionTransition::PauseRequested, at(origin, 20));
    clock.apply_at(SessionTransition::PauseCompleted, at(origin, 25));
    clock.apply_at(SessionTransition::ResumeRequested, at(origin, 100));

    assert_eq!(
        clock.apply_at(SessionTransition::ResumeAborted, at(origin, 110)),
        SessionTransitionResult::Applied {
            from: SessionPhase::Resuming,
            to: SessionPhase::Paused,
        }
    );
    assert_eq!(
        clock.logical_elapsed_at(at(origin, 1_000)),
        Duration::from_millis(20)
    );
    assert_eq!(clock.timestamp_at(at(origin, 1_000)), None);
}
