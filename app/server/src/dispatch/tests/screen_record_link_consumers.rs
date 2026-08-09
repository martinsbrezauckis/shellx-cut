#[cfg(unix)]
use super::super::dispatch;
#[cfg(unix)]
use super::test_actor;
#[cfg(unix)]
use crate::state::AppState;
#[cfg(unix)]
use serde_json::json;

#[cfg(unix)]
#[tokio::test]
async fn linked_capture_inputs_are_rejected_by_polish_export_and_autoedit() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = temp.path().join("linked_capture_consumers.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"linked_capture_consumers","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let outside = temp.path().join("outside-capture-consumer-sentinel");
    std::fs::write(&outside, b"outside capture consumer sentinel").unwrap();

    let polish_source = project_dir.join("linked-polish-source.mp4");
    symlink(&outside, &polish_source).unwrap();
    let polished = dispatch(
        &state,
        "screen_record.polish",
        json!({"source": polish_source, "plan": "unused-plan.json"}),
        test_actor(),
    )
    .await;
    assert!(!polished.ok, "polish must reject a linked source leaf");
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"outside capture consumer sentinel"
    );

    let export_source = project_dir.join("linked-export-source.mp4");
    symlink(&outside, &export_source).unwrap();
    let exported = dispatch(
        &state,
        "screen_record.export",
        json!({"source": export_source, "plan": "unused-plan.json"}),
        test_actor(),
    )
    .await;
    assert!(!exported.ok, "export must reject a linked source leaf");
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"outside capture consumer sentinel"
    );

    let track = project_dir.join("events.json");
    std::fs::write(
        &track,
        serde_json::to_vec(&json!({
            "duration_ms": 1,
            "screen_w": 1,
            "screen_h": 1,
            "monitors": [],
            "cursor": [],
            "clicks": [],
            "scrolls": [],
            "keys": [],
        }))
        .unwrap(),
    )
    .unwrap();
    let studio_events = project_dir.join("linked-studio-events.json");
    symlink(&outside, &studio_events).unwrap();
    let autoedited = dispatch(
        &state,
        "screen_record.autoedit",
        json!({"track": track, "studio_events": studio_events}),
        test_actor(),
    )
    .await;
    assert!(
        !autoedited.ok,
        "autoedit must reject linked Studio event metadata"
    );
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"outside capture consumer sentinel"
    );
}
