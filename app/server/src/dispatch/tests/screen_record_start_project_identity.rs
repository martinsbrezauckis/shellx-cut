//! A background Start must bind to the project observed before the hotkey.

use super::*;
use std::sync::Arc;

async fn create(state: &AppState, name: &str, path: &std::path::Path) {
    let result = dispatch(
        state,
        "project.create",
        json!({"name":name,"dir":path}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{name}: {:?}", result.error);
}

async fn identity(state: &AppState) -> Value {
    let result = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(result.ok, "{:?}", result.error);
    result.result.unwrap()["project_identity"].clone()
}

#[tokio::test]
async fn expected_identity_refuses_closed_or_changed_project_before_capture_setup() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a.cutproj");
    let b = root.path().join("b.cutproj");
    let state = AppState::new();
    create(&state, "a", &a).await;
    let a_identity = identity(&state).await;
    // Call the handler directly to isolate its admission order from the
    // public schema: the matching project passes, then invalid FPS refuses
    // before any native capture can start.
    let admitted = crate::screen_record::screen_record_start(
        &state,
        json!({"expected_project_identity":a_identity,"fps":0}),
    )
    .await;
    assert_eq!(admitted.unwrap_err().code, error_codes::INVALID_ARGS);

    let closed = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(closed.ok, "{:?}", closed.error);
    let absent = dispatch(
        &state,
        "screen_record.start",
        json!({"expected_project_identity":a_identity}),
        test_actor(),
    )
    .await;
    assert_eq!(absent.error.unwrap().code, error_codes::NO_PROJECT);

    create(&state, "b", &b).await;
    let before = std::fs::read_dir(b.join("cache/screen_record"))
        .ok()
        .map(|entries| entries.count())
        .unwrap_or(0);
    let changed = dispatch(
        &state,
        "screen_record.start",
        json!({"expected_project_identity":a_identity}),
        test_actor(),
    )
    .await;
    let error = changed.error.expect("changed project must refuse Start");
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("project changed"));
    assert_eq!(identity(&state).await["project_name"], "b");
    let after = std::fs::read_dir(b.join("cache/screen_record"))
        .ok()
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(after, before, "no capture marker or reservation directory");
}

#[tokio::test]
async fn queued_project_switch_wins_before_stale_start_checks_identity() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a.cutproj");
    let b = root.path().join("b.cutproj");
    let state = Arc::new(AppState::new());
    create(&state, "a", &a).await;
    let a_identity = identity(&state).await;
    create(&state, "b", &b).await;
    let opened = dispatch(&state, "project.open", json!({"path":a}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);

    let hold = state.project_transition.lock().await;
    let open_state = Arc::clone(&state);
    let open = tokio::spawn(async move {
        dispatch(&open_state, "project.open", json!({"path":b}), test_actor()).await
    });
    tokio::task::yield_now().await;
    assert!(!open.is_finished());
    let start_state = Arc::clone(&state);
    let start = tokio::spawn(async move {
        dispatch(
            &start_state,
            "screen_record.start",
            json!({"expected_project_identity":a_identity}),
            test_actor(),
        )
        .await
    });
    tokio::task::yield_now().await;
    assert!(
        !start.is_finished(),
        "Start waits for the project transition"
    );
    drop(hold);

    let opened = open.await.unwrap();
    assert!(opened.ok, "{:?}", opened.error);
    let result = start.await.unwrap();
    assert_eq!(result.error.unwrap().code, error_codes::CONFLICT);
    assert_eq!(identity(&state).await["project_name"], "b");
}

#[tokio::test]
async fn malformed_expected_identity_is_rejected_by_public_schema() {
    let state = AppState::new();
    let result = dispatch(
        &state,
        "screen_record.start",
        json!({"expected_project_identity":{"schema":"shellx-cut/project-identity/1","origin_path_sha256":"/private/path","project_name":"A"}}),
        test_actor(),
    )
    .await;
    assert_eq!(result.error.unwrap().code, error_codes::INVALID_ARGS);
}
