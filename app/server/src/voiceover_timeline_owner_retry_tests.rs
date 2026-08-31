//! Response-loss retry tests stay separate so the general owner lifecycle
//! fixture remains under its bounded feature-module size.

use super::*;

#[test]
fn exact_active_retry_survives_a_later_project_revision_and_keeps_stop_cancel_authority() {
    let request_a = request(VoiceoverCue::Playhead { at_ms: 20 });
    let mut stopped = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    stopped
        .accept(&project(), "op_000004", request_a.clone(), capture(false).0)
        .unwrap();
    let stopped_claim = stopped.owner_claim().unwrap();

    // An ordinary project edit can advance the current revision while A is
    // active. The coordinator probes this exact live owner before it compares
    // that later revision, so A must reattach instead of being called stale.
    let later_project_revision = "op_000005";
    assert_ne!(request_a.expected_revision, later_project_revision);
    assert!(stopped
        .retry_status(
            &request_a,
            &Actor::system(),
            Some(&stopped_claim.session_id),
        )
        .unwrap()
        .is_some());
    let reattached_stop_claim = stopped.owner_claim().unwrap();
    assert_eq!(reattached_stop_claim, stopped_claim);
    stopped
        .authorize(&Actor::system(), &reattached_stop_claim)
        .unwrap();
    assert!(matches!(
        stopped.stop().unwrap(),
        VoiceoverTimelineStatus::Finalizing { .. }
    ));

    let mut cancelled = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    cancelled
        .accept(&project(), "op_000004", request_a.clone(), capture(false).0)
        .unwrap();
    let cancelled_claim = cancelled.owner_claim().unwrap();
    assert!(cancelled
        .retry_status(
            &request_a,
            &Actor::system(),
            Some(&cancelled_claim.session_id),
        )
        .unwrap()
        .is_some());
    let reattached_cancel_claim = cancelled.owner_claim().unwrap();
    assert_eq!(reattached_cancel_claim, cancelled_claim);
    cancelled
        .authorize(&Actor::system(), &reattached_cancel_claim)
        .unwrap();
    assert!(matches!(
        cancelled.cancel().unwrap(),
        VoiceoverTimelineStatus::Finished {
            outcome: VoiceoverCaptureOutcome::Cancelled,
            ..
        }
    ));
}

#[test]
fn new_b_is_refused_while_active_a_owns_the_capture() {
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::Playhead { at_ms: 20 }),
            capture(false).0,
        )
        .unwrap();
    let mut request_b = request(VoiceoverCue::Playhead { at_ms: 80 });
    request_b.request_id = "voiceover-request-b".into();
    request_b.expected_revision = "op_000005".into();
    assert_eq!(
        owner
            .retry_status(&request_b, &Actor::system(), None)
            .unwrap_err()
            .code,
        error_codes::CONFLICT,
        "a later B must not reserve a second microphone while A is active",
    );
}
