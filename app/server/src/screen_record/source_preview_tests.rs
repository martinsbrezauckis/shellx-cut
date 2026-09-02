use super::*;
use record_capture::source_preview_native::test_support::SourcePreviewTestMailbox;

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
    let mut preview = PreviewOwner::new();
    preview.region = CaptureRegion::new(0, 0, 2, 2, 2, 2);

    preview.hide().unwrap();
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
    let (_, first) = preview.snapshot().unwrap();
    mailbox
        .publish_bgra(1_050, 1, 1, 4, &[9, 8, 7, 255])
        .unwrap();
    let (_, second) = preview.snapshot().unwrap();

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

    let (status, frame) = preview.snapshot().unwrap();
    assert_eq!(
        status.state,
        record_capture::source_preview::SourcePreviewState::PermissionDenied
    );
    assert!(!status.has_frame);
    assert!(frame.is_none());
}
