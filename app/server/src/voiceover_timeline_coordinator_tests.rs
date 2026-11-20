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

// Voiceover records only the microphone. Its start admission must not read,
// list, or probe screen capture, and a refusal must name the microphone.
#[test]
fn voiceover_start_admission_never_touches_screen_capture() {
    record_capture::forbid_native_capture_access();
    let before = record_capture::native_screen_access_attempts();
    let admitted = crate::screen_record::ensure_voiceover_ready();
    assert_eq!(
        record_capture::native_screen_access_attempts(),
        before,
        "voiceover admission read screen capture state"
    );
    if let Err(error) = admitted {
        assert!(error.message.contains("microphone"), "{error:?}");
        assert!(!error.message.contains("screen"), "{error:?}");
        assert!(error.suggested_action.is_some(), "{error:?}");
    }
}

#[test]
fn a_failed_take_says_why_and_never_leaks_paths() {
    let blocked =
        record_capture::MicrophoneProblem::Blocked(record_capture::MicrophoneSwitch::DesktopApps)
            .into_error(Some(&record_core::RecordError::new(
                "capture",
                "mic config",
                "Access is denied. (0x80070005)",
            )));
    let value = status_value(
        &VoiceoverTimelineStatus::Finished {
            take: take(),
            outcome: record_capture::VoiceoverCaptureOutcome::Failed(blocked),
        },
        None,
    );
    assert_eq!(value["terminal"], "failed");
    assert_eq!(
        value["failure"]["reason"],
        "windows_desktop_apps_microphone_off"
    );
    assert_eq!(
        value["failure"]["message"],
        "Windows is blocking microphone access for desktop apps"
    );
    assert!(value["failure"]["suggested_action"]
        .as_str()
        .is_some_and(|action| action.contains("Let desktop apps access your microphone")));
    assert!(value["failure"]["detail"]
        .as_str()
        .is_some_and(|detail| detail.contains("0x80070005")));

    let other =
        record_core::RecordError::new("io", "publish microphone audio", r"C:\private\take.wav");
    let value = status_value(
        &VoiceoverTimelineStatus::Finished {
            take: take(),
            outcome: record_capture::VoiceoverCaptureOutcome::Failed(other),
        },
        None,
    );
    assert_eq!(value["failure"]["reason"], "capture_failed");
    assert!(value["failure"].get("detail").is_none());
    assert!(!value.to_string().contains("take.wav"));
}

#[test]
fn a_take_that_did_not_fail_has_no_failure_field() {
    let value = status_value(&VoiceoverTimelineStatus::Finalizing { take: take() }, None);
    assert!(value.get("failure").is_none());
}
