use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use serde_json::json;

fn write_ready_capture(project_dir: &std::path::Path, capture_id: &str) -> std::path::PathBuf {
    let root = record_recovery::CaptureRoot::for_project(project_dir).unwrap();
    let capture_dir = root.create_capture_dir(capture_id).unwrap();
    let source = capture_dir.join("source.mp4");
    std::fs::write(&source, b"inside capture source").unwrap();
    std::fs::write(
        capture_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": source,
            "events": {"clicks": [], "cursor": []},
        }))
        .unwrap(),
    )
    .unwrap();
    capture_dir
}

#[tokio::test]
async fn recovery_status_does_not_create_a_missing_capture_cache() {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("status_no_cache.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"status_no_cache","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let cache = project_dir.join("cache");
    assert!(
        !cache.exists(),
        "project creation must not need capture cache"
    );

    let status = dispatch(
        &state,
        "screen_record.recovery_status",
        json!({}),
        test_actor(),
    )
    .await;

    assert!(status.ok, "{:?}", status.error);
    assert_eq!(status.result.unwrap()["captures"], json!([]));
    assert!(
        !cache.exists(),
        "read-only status must not create capture cache"
    );
}

#[tokio::test]
async fn screen_record_stop_ignores_forged_legacy_marker_project_path() {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("marker_path_guard.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"marker_path_guard","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let capture_id = "cap-forged-marker";
    let root = record_recovery::CaptureRoot::for_project(&project_dir).unwrap();
    let capture_dir = root.create_capture_dir(capture_id).unwrap();
    let source = capture_dir.join("source.mp4");
    std::fs::write(&source, b"inside capture source").unwrap();
    let inside_project = capture_dir.join("project.json");
    std::fs::write(
        &inside_project,
        serde_json::to_vec(&json!({
            "source_video": source,
            "events": {"clicks": [], "cursor": []},
        }))
        .unwrap(),
    )
    .unwrap();
    let outside_project = temp.path().join("outside-project.json");
    std::fs::write(&outside_project, b"outside project sentinel").unwrap();
    root.publish_new_capture_file(
        capture_id,
        ".capture.json",
        &serde_json::to_vec(&json!({
            "duration_ms": 1,
            "project_path": outside_project,
        }))
        .unwrap(),
    )
    .unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id}),
        test_actor(),
    )
    .await;

    assert!(stopped.ok, "{:?}", stopped.error);
    assert_eq!(
        stopped.result.unwrap()["project"],
        serde_json::Value::String(inside_project.display().to_string())
    );
    assert_eq!(
        std::fs::read(&outside_project).unwrap(),
        b"outside project sentinel"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn screen_record_stop_rejects_a_linked_marker_without_reading_outside() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("marker_link_guard.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"marker_link_guard","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let capture_id = "cap-linked-marker";
    let root = record_recovery::CaptureRoot::for_project(&project_dir).unwrap();
    let capture_dir = root.create_capture_dir(capture_id).unwrap();
    let marker_target = temp.path().join("outside-marker.json");
    std::fs::write(&marker_target, br#"{"duration_ms": 1}"#).unwrap();
    symlink(&marker_target, capture_dir.join(".capture.json")).unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id}),
        test_actor(),
    )
    .await;

    assert!(!stopped.ok, "linked marker must be rejected");
    assert_eq!(
        std::fs::read(&marker_target).unwrap(),
        br#"{"duration_ms": 1}"#
    );
}

#[cfg(unix)]
#[tokio::test]
async fn screen_record_stop_rejects_linked_capture_artifacts_before_raw_mux() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("linked_artifact_guard.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"linked_artifact_guard","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let outside = temp.path().join("outside-artifact-sentinel");
    std::fs::write(&outside, b"artifact sentinel").unwrap();
    for (capture_id, leaf) in [
        ("cap-linked-source", "source.mp4"),
        ("cap-linked-mic", "mic.wav"),
        ("cap-linked-system", "system.wav"),
        ("cap-linked-timing", "system-audio.json"),
        ("cap-linked-studio", "studio-events.json"),
    ] {
        let capture_dir = write_ready_capture(&project_dir, capture_id);
        if leaf == "source.mp4" {
            std::fs::remove_file(capture_dir.join(leaf)).unwrap();
        }
        if leaf == crate::screen_record::system_audio::SYSTEM_AUDIO_TIMING_FILE {
            std::fs::write(capture_dir.join("system.wav"), b"local system audio").unwrap();
        }
        symlink(&outside, capture_dir.join(leaf)).unwrap();

        let stopped = dispatch(
            &state,
            "screen_record.stop",
            json!({"capture_id": capture_id, "mux_raw": true}),
            test_actor(),
        )
        .await;

        assert!(!stopped.ok, "linked {leaf} must be rejected before raw mux");
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"artifact sentinel",
            "linked {leaf} must not read or mutate the outside sentinel"
        );
    }
}

#[tokio::test]
async fn screen_record_stop_rejects_an_outside_declared_audio_path_without_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("declared_audio_guard.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"declared_audio_guard","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let capture_id = "cap-outside-declared-audio";
    let capture_dir = write_ready_capture(&project_dir, capture_id);
    let outside = temp.path().join("outside-declared-audio.wav");
    std::fs::write(&outside, b"outside audio sentinel").unwrap();
    std::fs::write(
        capture_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": capture_dir.join("source.mp4"),
            "audio": outside,
            "events": {"clicks": [], "cursor": []},
        }))
        .unwrap(),
    )
    .unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "mux_raw": true}),
        test_actor(),
    )
    .await;

    assert!(
        !stopped.ok,
        "outside declared audio must not fall back to mic.wav"
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside audio sentinel");
}
