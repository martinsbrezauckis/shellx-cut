use super::source_preview::{
    SourcePreviewCommand, SourcePreviewFrame, SourcePreviewLifecycle, SourcePreviewPlatform,
    SourcePreviewRecursion, SourcePreviewRequest, SourcePreviewSource, SourcePreviewState,
    MAX_SOURCE_PREVIEW_FRAME_BYTES, SOURCE_PREVIEW_MIN_FRAME_INTERVAL_MS,
};
use record_core::error_codes;

fn monitor(id: &str) -> SourcePreviewRequest {
    SourcePreviewRequest {
        source: SourcePreviewSource::Monitor {
            monitor_id: id.into(),
        },
        camera_id: None,
    }
}

fn window(id: &str) -> SourcePreviewRequest {
    SourcePreviewRequest {
        source: SourcePreviewSource::Window {
            window_id: id.into(),
        },
        camera_id: Some("opaque-camera-01".into()),
    }
}

fn start(generation: u64, request: SourcePreviewRequest) -> SourcePreviewCommand {
    SourcePreviewCommand::Start {
        generation,
        request,
    }
}

#[test]
fn windows_and_macos_keep_exact_opaque_monitor_and_window_ids() {
    let request = monitor("shellx-monitor-v1:windows:exact");
    for platform in [SourcePreviewPlatform::Windows, SourcePreviewPlatform::Macos] {
        let mut preview = SourcePreviewLifecycle::new(platform);
        assert_eq!(
            preview.start(request.clone()).unwrap(),
            vec![start(1, request.clone())]
        );
        assert_eq!(preview.status().request, Some(request.clone()));
        assert_eq!(preview.status().generation, Some(1));
    }

    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    assert_eq!(
        preview.start(window("opaque-window:pid:hwnd")).unwrap(),
        vec![start(1, window("opaque-window:pid:hwnd"))]
    );
}

#[test]
fn source_forms_fail_closed_without_titles_ordinals_or_linux_substitution() {
    let invalid = SourcePreviewRequest {
        source: SourcePreviewSource::Window {
            window_id: "   ".into(),
        },
        camera_id: None,
    };
    assert_eq!(
        invalid
            .validate_for(SourcePreviewPlatform::Windows)
            .unwrap_err()
            .code,
        error_codes::INVALID_ARGS
    );
    assert_eq!(
        monitor("shellx-monitor-v1:windows:exact")
            .validate_for(SourcePreviewPlatform::Linux)
            .unwrap_err()
            .code,
        error_codes::UNIMPLEMENTED
    );
    assert_eq!(
        SourcePreviewRequest {
            source: SourcePreviewSource::Portal,
            camera_id: None,
        }
        .validate_for(SourcePreviewPlatform::Macos)
        .unwrap_err()
        .code,
        error_codes::UNIMPLEMENTED
    );
    assert!(SourcePreviewRequest {
        source: SourcePreviewSource::Portal,
        camera_id: None,
    }
    .validate_for(SourcePreviewPlatform::Linux)
    .is_ok());
}

#[test]
fn opaque_source_selector_rejects_unrepresentable_title_or_native_fields() {
    let parsed = serde_json::from_str::<SourcePreviewSource>(
        r#"{"kind":"monitor","monitor_id":"opaque-monitor","title":"Desktop"}"#,
    );
    assert!(parsed.is_err());
    let parsed = serde_json::from_str::<SourcePreviewSource>(
        r#"{"kind":"window","window_id":"opaque-window","native_handle":7}"#,
    );
    assert!(parsed.is_err());
}

#[test]
fn replacing_a_live_source_releases_it_before_starting_the_exact_replacement() {
    let first = monitor("shellx-monitor-v1:windows:first");
    let second = window("opaque-window:pid:replacement");
    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    preview.start(first).unwrap();
    preview
        .accept_frame(1, SourcePreviewFrame::new(1, vec![1]).unwrap())
        .unwrap();

    assert_eq!(
        preview.start(second.clone()).unwrap(),
        vec![SourcePreviewCommand::Stop, start(2, second)]
    );
    assert_eq!(preview.status().state, SourcePreviewState::Starting);
    assert!(!preview.status().has_frame);
    assert!(preview
        .accept_frame(1, SourcePreviewFrame::new(2, vec![8]).unwrap())
        .is_err());
    assert!(preview
        .accept_frame(2, SourcePreviewFrame::new(2, vec![2]).unwrap())
        .unwrap());
    assert_eq!(preview.latest_frame().unwrap().encoded(), &[2]);
}

#[test]
fn real_memory_frame_is_required_for_ready_and_pause_releases_native_ownership() {
    let request = monitor("shellx-monitor-v1:macos:exact");
    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Macos);
    preview.start(request.clone()).unwrap();
    assert_eq!(preview.status().state, SourcePreviewState::Starting);
    preview
        .accept_frame(1, SourcePreviewFrame::new(42, vec![1, 2, 3]).unwrap())
        .unwrap();
    assert_eq!(preview.status().state, SourcePreviewState::Ready);
    assert_eq!(preview.latest_frame().unwrap().captured_at_ms(), 42);
    assert_eq!(preview.pause(), vec![SourcePreviewCommand::Stop]);
    assert_eq!(preview.status().state, SourcePreviewState::Paused);
    assert!(!preview.status().has_frame);
    assert_eq!(preview.resume().unwrap(), vec![start(2, request)]);
    assert!(preview
        .accept_frame(2, SourcePreviewFrame::new(1, vec![9]).unwrap())
        .unwrap());
}

#[test]
fn hidden_terminal_and_recording_release_paths_never_leave_a_native_lease() {
    let request = monitor("shellx-monitor-v1:windows:exact");
    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    preview.start(request.clone()).unwrap();
    assert_eq!(preview.hide(), vec![SourcePreviewCommand::Stop]);
    assert_eq!(preview.status().state, SourcePreviewState::Hidden);
    assert_eq!(preview.status().request, None);
    assert_eq!(preview.status().generation, None);
    assert!(preview.stop().is_empty());
    assert_eq!(preview.status().state, SourcePreviewState::Stopped);
    assert_eq!(preview.status().request, None);

    preview.start(request).unwrap();
    assert_eq!(
        preview.release_for_recording(),
        vec![SourcePreviewCommand::Stop]
    );
    assert_eq!(preview.status().state, SourcePreviewState::Stopped);
}

#[test]
fn permission_loss_and_unavailable_are_explicit_terminal_states() {
    let request = monitor("shellx-monitor-v1:windows:exact");
    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    preview.start(request.clone()).unwrap();
    assert_eq!(
        preview.permission_required(),
        vec![SourcePreviewCommand::Stop]
    );
    assert_eq!(
        preview.status().state,
        SourcePreviewState::PermissionRequired
    );
    assert_eq!(preview.status().request, None);
    assert_eq!(preview.status().generation, None);

    preview.start(request.clone()).unwrap();
    assert_eq!(
        preview.permission_denied(),
        vec![SourcePreviewCommand::Stop]
    );
    assert_eq!(preview.status().state, SourcePreviewState::PermissionDenied);
    preview.start(request).unwrap();
    assert_eq!(preview.source_lost(), vec![SourcePreviewCommand::Stop]);
    assert_eq!(preview.status().state, SourcePreviewState::SourceLost);
    assert!(preview.unavailable().is_empty());
    assert_eq!(preview.status().state, SourcePreviewState::Unavailable);
}

#[test]
fn recursion_is_disclosed_without_fabricating_or_retaining_media() {
    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    preview
        .start(monitor("shellx-monitor-v1:windows:exact"))
        .unwrap();
    preview.set_recursion(SourcePreviewRecursion::Unavoidable);
    assert_eq!(
        preview.status().recursion,
        SourcePreviewRecursion::Unavoidable
    );
    assert!(!preview.status().has_frame);
    assert_eq!(preview.source_lost(), vec![SourcePreviewCommand::Stop]);
    assert!(!preview.status().has_frame);
}

#[test]
fn frame_retention_is_bounded_and_requires_an_active_preview() {
    assert!(SourcePreviewFrame::new(0, Vec::new()).is_err());
    assert!(SourcePreviewFrame::new(0, vec![0; MAX_SOURCE_PREVIEW_FRAME_BYTES + 1]).is_err());

    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    assert!(preview
        .accept_frame(1, SourcePreviewFrame::new(0, vec![1]).unwrap())
        .is_err());
}

#[test]
fn frame_updates_are_rate_limited_without_turning_a_real_preview_unready() {
    let mut preview = SourcePreviewLifecycle::new(SourcePreviewPlatform::Windows);
    preview
        .start(monitor("shellx-monitor-v1:windows:exact"))
        .unwrap();
    assert!(preview
        .accept_frame(1, SourcePreviewFrame::new(1_000, vec![1]).unwrap())
        .unwrap());
    assert!(!preview
        .accept_frame(
            1,
            SourcePreviewFrame::new(1_000 + SOURCE_PREVIEW_MIN_FRAME_INTERVAL_MS - 1, vec![2])
                .unwrap()
        )
        .unwrap());
    assert_eq!(preview.latest_frame().unwrap().encoded(), &[1]);
    assert_eq!(preview.status().state, SourcePreviewState::Ready);
}

#[test]
fn generation_exhaustion_fails_without_wrapping_or_changing_lifecycle_state() {
    let mut preview =
        SourcePreviewLifecycle::with_generation_for_test(SourcePreviewPlatform::Windows, u64::MAX);
    let error = preview
        .start(monitor("shellx-monitor-v1:windows:exact"))
        .unwrap_err();
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert_eq!(preview.status().state, SourcePreviewState::Idle);
    assert_eq!(preview.status().generation, None);
}
