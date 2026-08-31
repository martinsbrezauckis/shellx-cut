use super::*;
use crate::voiceover_timeline_owner::{AcceptedVoiceoverTimeline, VoiceoverTimelineStatus};

fn take() -> AcceptedVoiceoverTimeline {
    AcceptedVoiceoverTimeline {
        request_id: "voiceover-request-1".into(),
        revision: "op_000004".into(),
        audio_track: "a1t".into(),
        start_ms: 120,
        out_ms: Some(320),
    }
}

#[test]
fn status_projection_never_leaks_private_wav_paths_and_keeps_monitoring_off() {
    let value = status_value(&VoiceoverTimelineStatus::Finalizing { take: take() }, None);
    assert_eq!(value["phase"], "finishing");
    assert_eq!(value["direct_monitoring"], "off");
    assert_eq!(value["audio_track"], "a1t");
    assert!(value.get("path").is_none());
}

#[test]
fn start_request_binds_the_retry_identity_revision_track_and_out_range() {
    let actor = Actor::system().with_request(cut_core::MutationRequest {
        caller: "test".into(),
        request_id: "voiceover-request-1".into(),
        fingerprint: "sha256:test".into(),
        expected_revision: Some("op_000004".into()),
    });
    let request = start_request(
        StartArgs {
            audio_track: "a1t".into(),
            start_ms: 120,
            out_ms: Some(320),
            owner_session_id: None,
        },
        &actor,
    )
    .unwrap();
    assert_eq!(request.request_id, "voiceover-request-1");
    assert_eq!(request.expected_revision, "op_000004");
    assert!(matches!(
        request.cue,
        VoiceoverCue::InOut {
            in_ms: 120,
            out_ms: 320
        }
    ));
}
