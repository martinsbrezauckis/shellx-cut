use super::*;

/// Every runtime capture path uses the same plain spelling before native
/// capture and Recording Scenes ownership checks. Plain and Unix paths pass
/// through unchanged.
#[test]
fn capture_runtime_paths_strip_verbatim_prefix() {
    assert_eq!(
        strip_verbatim_prefix(Path::new(
            r"\\?\C:\Example\User\Documents\ShellX Cut Projects\rec.cutproj\cache"
        )),
        PathBuf::from(r"C:\Example\User\Documents\ShellX Cut Projects\rec.cutproj\cache"),
    );
    assert_eq!(
        strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\rec.cutproj\cache")),
        PathBuf::from(r"\\server\share\rec.cutproj\cache"),
    );
    // Already-plain and Unix paths are untouched.
    assert_eq!(
        strip_verbatim_prefix(Path::new(r"C:\Example\User\rec.cutproj\cache")),
        PathBuf::from(r"C:\Example\User\rec.cutproj\cache"),
    );
    assert_eq!(
        strip_verbatim_prefix(Path::new("/home/u/rec.cutproj/cache")),
        PathBuf::from("/home/u/rec.cutproj/cache"),
    );
}

#[test]
fn autoedit_config_overrides_engine_plan() {
    let dir = tempfile::tempdir().unwrap();
    let track = dir.path().join("events.json");
    let plan = screen_record_cache_dir(dir.path())
        .unwrap()
        .join("plan.json");
    std::fs::write(
        &track,
        serde_json::to_vec(&json!({
            "duration_ms": 4000,
            "screen_w": 1920,
            "screen_h": 1080,
            "clicks": [
                {"t_ms": 1000, "x": 960.0, "y": 540.0, "button": "left", "down": true}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let cfg = parse_autoedit_config(Some(json!({"max_zoom": 3.25}))).unwrap();
    autoedit(&track, &plan, &cfg).unwrap();

    let written: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    let max_scale = written["zoom"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|key| key["scale"].as_f64())
        .fold(0.0_f64, f64::max);
    assert!(
        (max_scale - 3.25).abs() < 1e-9,
        "config.max_zoom should drive the generated plan, got {max_scale}"
    );
}

#[test]
fn autoedit_config_rejects_unknown_keys() {
    let err = parse_autoedit_config(Some(json!({"max_zom": 3.0}))).unwrap_err();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("max_zom"),
        "unknown key should be named: {err:?}"
    );
}

#[test]
fn bounded_json_reader_rejects_oversized_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.json");
    std::fs::write(&path, b"12345").unwrap();

    let err = read_bounded_json(&path, "EventTrack", 4, "regenerate it").unwrap_err();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(err.message.contains("exceeds"));
}

#[test]
#[cfg(all(unix, not(windows)))]
fn system_audio_stop_strategy_falls_back_when_pid_is_too_large_for_sigint() {
    assert_eq!(
        system_audio_stop_strategy(1234),
        SystemAudioStopStrategy::Sigint(1234)
    );
    assert_eq!(
        system_audio_stop_strategy(i32::MAX as u32 + 1),
        SystemAudioStopStrategy::Kill
    );
}

#[test]
fn ready_rollup_requires_core_and_platform_cards() {
    let mk = |name: &str, status: &str| RecordCard {
        name: name.into(),
        status: status.into(),
        detail: String::new(),
        start_admission: RecordStartAdmission::Strict,
    };
    let all_ok = vec![
        mk("ffmpeg", "ok"),
        mk("screen_capture", "ok"),
        mk("input_hook", "ok"),
        mk("webcam", "missing"),
    ];
    assert!(ready_rollup(&all_ok));
    let missing_one = vec![
        mk("ffmpeg", "ok"),
        mk("screen_capture", "missing"),
        mk("input_hook", "ok"),
    ];
    assert!(!ready_rollup(&missing_one));
    let unverified_screen = vec![
        mk("ffmpeg", "ok"),
        mk("screen_capture", "unknown"),
        mk("input_hook", "ok"),
    ];
    assert!(
        !ready_rollup(&unverified_screen),
        "unknown delivery evidence must never make recording ready"
    );
    let linux_missing_gstreamer = vec![
        mk("ffmpeg", "ok"),
        mk("screen_capture", "ok"),
        mk("input_hook", "ok"),
        mk("gstreamer", "missing"),
        mk("wayland_input", "ok"),
    ];
    assert!(!ready_rollup(&linux_missing_gstreamer));
    let linux_missing_input = vec![
        mk("ffmpeg", "ok"),
        mk("screen_capture", "ok"),
        mk("input_hook", "ok"),
        mk("gstreamer", "ok"),
        mk("wayland_input", "degraded"),
    ];
    assert!(!ready_rollup(&linux_missing_input));
    assert!(!ready_rollup(&[]));
}

#[test]
fn canonical_linux_portal_card_gets_typed_start_admission() {
    let prompt = record_card(record_capture::Card {
        id: "screen_capture".into(),
        kind: "capture".into(),
        status: "unknown".into(),
        detail: record_capture::LINUX_PORTAL_PROMPT_DEFERRED_DETAIL.into(),
    });
    assert_eq!(
        prompt.start_admission,
        if cfg!(target_os = "linux") {
            RecordStartAdmission::LinuxPortalPromptDeferred
        } else {
            RecordStartAdmission::Strict
        }
    );
    let arbitrary = record_card(record_capture::Card {
        id: "screen_capture".into(),
        kind: "capture".into(),
        status: "unknown".into(),
        detail: "an unrelated prompt-deferred backend".into(),
    });
    assert_eq!(arbitrary.start_admission, RecordStartAdmission::Strict);
}

#[test]
fn capture_access_failure_degrades_ready_card_with_recovery_guidance() {
    let mut cards = vec![
        RecordCard {
            name: "ffmpeg".into(),
            status: "ok".into(),
            detail: String::new(),
            start_admission: RecordStartAdmission::Strict,
        },
        RecordCard {
            name: "screen_capture".into(),
            status: "ok".into(),
            detail: "compiled backend".into(),
            start_admission: RecordStartAdmission::Strict,
        },
        RecordCard {
            name: "input_hook".into(),
            status: "ok".into(),
            detail: String::new(),
            start_admission: RecordStartAdmission::Strict,
        },
    ];
    apply_capture_access_failure(&mut cards);
    let capture = cards
        .iter()
        .find(|card| card.name == "screen_capture")
        .unwrap();
    assert_eq!(capture.status, "degraded");
    assert!(capture.detail.contains("Privacy & Security"));
    assert!(capture.detail.contains("quit and reopen"));
    assert!(!ready_rollup(&cards));
}

#[test]
fn capture_settings_reject_invalid_duration_and_fps() {
    assert!(validate_capture_settings(None, 30.0).is_ok());
    assert!(validate_capture_settings(Some(1), 1.0).is_ok());
    assert!(validate_capture_settings(Some(1), 240.0).is_ok());
    for fps in [0.0, -1.0, 240.1, f64::INFINITY, f64::NAN] {
        let error = validate_capture_settings(None, fps).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }
    let error = validate_capture_settings(Some(0), 30.0).unwrap_err();
    assert_eq!(error.code, error_codes::INVALID_ARGS);
}

#[test]
fn record_err_maps_all_fields() {
    let re =
        record_core::RecordError::new("ffmpeg", "boom", "bad pipe").with_action("install ffmpeg");
    let ce = record_err(re);
    // CutError serializes {code,message,cause,suggested_action} — round-trip check.
    let v = serde_json::to_value(&ce).unwrap();
    assert_eq!(v["code"], "ffmpeg");
    assert_eq!(v["message"], "boom");
    assert_eq!(v["cause"], "bad pipe");
    assert_eq!(v["suggested_action"], "install ffmpeg");
}

/// cutd-RESTART fallback: stopping a capture id that was never registered (or
/// was lost when cutd restarted mid-capture) is a harmless no-op returning false —
/// the caller then falls back to the file poll for a capture that finalized on its
/// own bound. Must NOT panic.
#[test]
fn stop_capture_unknown_id_is_a_noop() {
    let unknown = format!("cap_never_started_{}", std::process::id());
    assert!(
        !stop_capture(&unknown),
        "an unknown capture id returns false (file-poll fallback path)"
    );
}
