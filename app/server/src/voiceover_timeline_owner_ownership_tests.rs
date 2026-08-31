//! Negative ownership and stale-Preview claim tests kept beside the active
//! owner helpers so the general lifecycle fixture stays bounded.

use super::*;
use cut_core::{error_codes, Actor, ActorKind};
use std::time::Duration;

#[test]
fn owner_capability_refuses_foreign_actor_and_stale_preview_out_claim() {
    let mut owner = VoiceoverTimelineOwner::with_countdown(Duration::ZERO);
    let (capture, _) = capture(true);
    owner
        .accept(
            &project(),
            "op_000004",
            request(VoiceoverCue::InOut {
                in_ms: 120,
                out_ms: 320,
            }),
            capture,
        )
        .unwrap();
    let claim = owner.owner_claim().unwrap();
    let foreign = Actor {
        kind: ActorKind::Human,
        name: "foreign-loopback".into(),
        via: "ui".into(),
        request: None,
    };
    assert_eq!(
        owner.authorize(&foreign, &claim).unwrap_err().code,
        error_codes::CONFLICT
    );
    owner.authorize(&Actor::system(), &claim).unwrap();

    assert!(matches!(
        owner.status().unwrap(),
        VoiceoverTimelineStatus::StartProgramPlayback { .. }
    ));
    let fingerprint = super::super::fingerprint::for_accepted(&owner.active.as_ref().unwrap().take);
    owner
        .playback_started("voiceover-request-1", &fingerprint, 0)
        .unwrap();
    assert_eq!(
        owner
            .verify_preview_observation("voiceover-request-1", &"f".repeat(64), 0)
            .unwrap_err()
            .code,
        error_codes::CONFLICT
    );
    assert_eq!(
        owner
            .verify_preview_observation("voiceover-request-1", &fingerprint, 7)
            .unwrap_err()
            .code,
        error_codes::CONFLICT
    );
}
