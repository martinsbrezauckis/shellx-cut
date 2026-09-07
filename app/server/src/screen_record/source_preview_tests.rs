use super::super::capture_session_control::CaptureSessionControl;
use super::*;
use record_capture::source_preview_native::test_support::SourcePreviewTestMailbox;

const TEST_LEASE_NONCE: &str = "00112233445566778899aabbccddeeff";
const RESTARTED_LEASE_NONCE: &str = "ffeeddccbbaa99887766554433221100";

fn active_preview(session: NativeSourcePreviewSession) -> PreviewOwner {
    active_preview_with_nonce(session, TEST_LEASE_NONCE)
}

fn active_preview_with_nonce(
    session: NativeSourcePreviewSession,
    lease_nonce: &str,
) -> PreviewOwner {
    let mut lifecycle = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    lifecycle
        .start(SourcePreviewRequest {
            source: SourcePreviewSource::Monitor {
                monitor_id: "opaque-monitor".into(),
            },
            camera_id: None,
        })
        .unwrap();
    PreviewOwner {
        lifecycle,
        native: Some(session),
        region: None,
        lease: PreviewLease::with_nonce(lease_nonce),
        teardown_failed: false,
    }
}

#[test]
fn private_region_rejects_a_window_before_starting_native_capture() {
    let mut preview = PreviewOwner::new();
    let request = SourcePreviewRequest {
        source: SourcePreviewSource::Window {
            window_id: "opaque-window".into(),
        },
        camera_id: None,
    };
    let region = CaptureRegion::new(0, 0, 2, 2, 2, 2).unwrap();

    assert!(preview.start_with_region(request, Some(region)).is_err());
    assert!(preview.region.is_none());
}

#[test]
fn recording_release_erases_private_region_state() {
    let mut preview = PreviewOwner::new();
    preview.region = CaptureRegion::new(0, 0, 2, 2, 2, 2);

    preview.release_for_recording().unwrap();
    assert!(preview.region.is_none());
}

#[test]
fn hiding_preview_erases_private_region_state() {
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = active_preview(mailbox.session(1));
    preview.region = CaptureRegion::new(0, 0, 2, 2, 2, 2);

    preview.hide(1, TEST_LEASE_NONCE).unwrap();
    assert!(preview.region.is_none());
}

#[test]
fn failed_native_start_cannot_retain_private_region_state() {
    let mut preview = PreviewOwner::new();
    let request = SourcePreviewRequest {
        source: SourcePreviewSource::Monitor {
            monitor_id: "missing-native-monitor".into(),
        },
        camera_id: None,
    };
    let region = CaptureRegion::new(0, 0, 2, 2, 2, 2).unwrap();

    assert!(preview.start_with_region(request, Some(region)).is_err());
    assert!(preview.region.is_none());
}

#[test]
fn snapshot_never_exposes_a_mailbox_update_dropped_by_the_rate_gate() {
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = PreviewOwner {
        lifecycle: SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows),
        native: Some(mailbox.session(1)),
        region: None,
        lease: PreviewLease::with_nonce(TEST_LEASE_NONCE),
        teardown_failed: false,
    };
    preview
        .lifecycle
        .start(SourcePreviewRequest {
            source: SourcePreviewSource::Monitor {
                monitor_id: "opaque-monitor".into(),
            },
            camera_id: None,
        })
        .unwrap();
    mailbox
        .publish_bgra(1_000, 1, 1, 4, &[1, 2, 3, 255])
        .unwrap();
    let first = preview.snapshot().unwrap().frame;
    mailbox
        .publish_bgra(1_050, 1, 1, 4, &[9, 8, 7, 255])
        .unwrap();
    let second = preview.snapshot().unwrap().frame;

    assert_eq!(first.unwrap().captured_at_ms(), 1_000);
    assert_eq!(second.unwrap().captured_at_ms(), 1_000);
}

#[test]
fn explicit_native_permission_denial_is_not_relabelled_as_source_loss() {
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = PreviewOwner {
        lifecycle: SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows),
        native: Some(mailbox.session(1)),
        region: None,
        lease: PreviewLease::with_nonce(TEST_LEASE_NONCE),
        teardown_failed: false,
    };
    preview
        .lifecycle
        .start(SourcePreviewRequest {
            source: SourcePreviewSource::Monitor {
                monitor_id: "opaque-monitor".into(),
            },
            camera_id: None,
        })
        .unwrap();
    mailbox.mark_permission_denied();

    let snapshot = preview.snapshot().unwrap();
    let status = snapshot.status;
    let frame = snapshot.frame;
    assert_eq!(
        status.state,
        record_capture::source_preview::SourcePreviewState::PermissionDenied
    );
    assert!(!status.has_frame);
    assert!(frame.is_none());
}

#[test]
fn stale_generation_stop_cannot_release_the_current_preview() {
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = active_preview(mailbox.session(1));

    let error = preview
        .stop(2, TEST_LEASE_NONCE)
        .expect_err("stale generation must be rejected");

    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(preview.native.is_some());
    assert_eq!(preview.lifecycle.status().generation, Some(1));
}

#[test]
fn stale_generation_hide_cannot_release_the_current_preview() {
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = active_preview(mailbox.session(1));

    let error = preview
        .hide(2, TEST_LEASE_NONCE)
        .expect_err("stale generation must be rejected");

    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(preview.native.is_some());
    assert_eq!(preview.lifecycle.status().generation, Some(1));
}

#[test]
fn failed_native_stop_becomes_unavailable_and_blocks_recording_handoff() {
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = active_preview(mailbox.failing_stop_session(1));

    let error = preview
        .stop(1, TEST_LEASE_NONCE)
        .expect_err("failed native close must not report a released preview");

    // `record_err` retains the record-core capture code verbatim in the public
    // Cut error envelope; Cut's own code module has no `CAPTURE` constant.
    assert_eq!(error.code, "capture");
    assert_eq!(
        error.suggested_action.as_deref(),
        Some("restart ShellX Cut (or the Cut server) before starting another preview or recording")
    );
    assert!(preview.native.is_none());
    assert!(preview.teardown_failed);
    assert_eq!(
        preview.lifecycle.status().state,
        record_capture::source_preview::SourcePreviewState::Unavailable
    );
    let snapshot = preview
        .snapshot()
        .expect("terminal status remains readable");
    assert_eq!(snapshot.unavailable_reason, Some(UNRESOLVED_RELEASE_REASON));
    assert!(preview.pause(1, TEST_LEASE_NONCE).is_err());
    assert!(preview.release_for_recording().is_err());
    assert!(preview
        .start(SourcePreviewRequest {
            source: SourcePreviewSource::Monitor {
                monitor_id: "different-monitor".into(),
            },
            camera_id: None,
        })
        .is_err());
}

#[test]
fn restarted_server_nonce_rejects_a_delayed_same_generation_control() {
    let before_restart = SourcePreviewTestMailbox::new();
    let previous = active_preview_with_nonce(before_restart.session(1), TEST_LEASE_NONCE);
    let after_restart = SourcePreviewTestMailbox::new();
    let mut restarted = active_preview_with_nonce(after_restart.session(1), RESTARTED_LEASE_NONCE);

    assert_eq!(previous.lifecycle.status().generation, Some(1));
    assert_eq!(restarted.lifecycle.status().generation, Some(1));
    let error = restarted
        .stop(1, TEST_LEASE_NONCE)
        .expect_err("an old server nonce cannot control a restarted owner");

    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(restarted.native.is_some());
    assert_eq!(restarted.lifecycle.status().generation, Some(1));
}

#[test]
fn capture_handoff_reserves_before_releasing_preview_and_rejects_competing_start() {
    let _capture_lock = capture_registry::capture_test_lock().blocking_lock();
    let project_root = tempfile::tempdir().expect("test project root");
    let mailbox = SourcePreviewTestMailbox::new();
    let mut preview = active_preview(mailbox.session(1));
    let capture_id = format!("cap_preview_handoff_{}", std::process::id());
    let control = CaptureSessionControl::new(None, false, false, false);

    let reservation = capture_registry::reserve_capture_after_preview_release(
        capture_id,
        control,
        Some(project_root.path().to_path_buf()),
        || {
            assert!(
                capture_registry::has_active_capture(),
                "the capture reservation must exist before preview release"
            );
            preview.release_for_recording()?;
            let error = preview
                .start(SourcePreviewRequest {
                    source: SourcePreviewSource::Monitor {
                        monitor_id: "competing-monitor".into(),
                    },
                    camera_id: None,
                })
                .expect_err("a competing preview must observe the reservation during handoff");
            assert_eq!(error.code, error_codes::CONFLICT);
            Ok(())
        },
    )
    .expect("release succeeds while the capture reservation remains held");

    assert!(capture_registry::has_active_capture());
    drop(reservation);
    assert!(!capture_registry::has_active_capture());
}

#[test]
fn failed_handoff_release_drops_its_capture_reservation() {
    let _capture_lock = capture_registry::capture_test_lock().blocking_lock();
    let project_root = tempfile::tempdir().expect("test project root");
    let capture_id = format!("cap_preview_handoff_failure_{}", std::process::id());
    let error = capture_registry::reserve_capture_after_preview_release(
        capture_id,
        CaptureSessionControl::new(None, false, false, false),
        Some(project_root.path().to_path_buf()),
        || {
            Err(CutError::new(
                error_codes::IO,
                "synthetic preview release failure",
                "test cleanup path",
            ))
        },
    )
    .expect_err("handoff must return the preview release failure");

    assert_eq!(error.code, error_codes::IO);
    assert!(
        !capture_registry::has_active_capture(),
        "a failed handoff cannot strand a capture reservation"
    );
}
