//! A native capture pins its project directory until worker teardown.

use super::*;
use crate::screen_record::{capture_test_lock, reserve_project_transition_test_capture};

async fn create_project(state: &AppState, name: &str, path: &std::path::Path) {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": name, "dir": path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{name} create failed: {:?}", created.error);
}

#[tokio::test]
async fn active_capture_reservation_refuses_project_replacement_before_any_project_mutation() {
    let _capture_lock = capture_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let c_path = root.path().join("c.cutproj");
    let state = AppState::new();
    create_project(&state, "a", &a_path).await;
    create_project(&state, "b", &b_path).await;

    let reopened_a = dispatch(
        &state,
        "project.open",
        json!({"path": a_path}),
        test_actor(),
    )
    .await;
    assert!(reopened_a.ok, "A reopen failed: {:?}", reopened_a.error);

    let reservation =
        reserve_project_transition_test_capture(&a_path, "project-transition-capture");
    let mut events = state.events.subscribe();
    for (verb, args) in [
        ("project.open", json!({"path": b_path})),
        ("project.close", json!({})),
        ("project.create", json!({"name": "c", "dir": c_path})),
        ("project.delete", json!({"path": a_path})),
    ] {
        let rejected = dispatch(&state, verb, args, test_actor()).await;
        assert!(!rejected.ok, "{verb} must refuse a held native reservation");
        let error = rejected.error.expect("the refusal has a typed error");
        assert_eq!(error.code, error_codes::CONFLICT, "{verb}: {error:?}");
        assert!(
            error
                .message
                .contains("cannot change projects while a native capture is active"),
            "{verb}: {error:?}"
        );
        let current_name = state
            .project
            .read()
            .await
            .as_ref()
            .map(|store| store.project.name.clone());
        assert_eq!(
            current_name.as_deref(),
            Some("a"),
            "{verb} must leave A current"
        );
    }
    assert!(
        !c_path.exists(),
        "project.create must reject before creating a project directory"
    );
    assert!(
        events.try_recv().is_err(),
        "rejected transitions must not publish a project-changed event"
    );

    // A process can host independent AppState fixtures. The capture owns A's
    // project root, not every unrelated project root in the process: B may
    // continue through ordinary transitions, but it still cannot delete A.
    let foreign_state = AppState::new();
    let d_path = root.path().join("d.cutproj");
    let foreign_opened_b = dispatch(
        &foreign_state,
        "project.open",
        json!({"path": b_path}),
        test_actor(),
    )
    .await;
    assert!(
        foreign_opened_b.ok,
        "independent B open must not be blocked by A capture: {:?}",
        foreign_opened_b.error
    );
    let foreign_created_d = dispatch(
        &foreign_state,
        "project.create",
        json!({"name": "d", "dir": d_path}),
        test_actor(),
    )
    .await;
    assert!(
        foreign_created_d.ok,
        "independent B create must not be blocked by A capture: {:?}",
        foreign_created_d.error
    );
    let foreign_reopened_b = dispatch(
        &foreign_state,
        "project.open",
        json!({"path": b_path}),
        test_actor(),
    )
    .await;
    assert!(
        foreign_reopened_b.ok,
        "independent B reopen must not be blocked by A capture: {:?}",
        foreign_reopened_b.error
    );
    let foreign_closed_b = dispatch(&foreign_state, "project.close", json!({}), test_actor()).await;
    assert!(
        foreign_closed_b.ok,
        "independent B close must not be blocked by A capture: {:?}",
        foreign_closed_b.error
    );
    let foreign_delete_a = dispatch(
        &foreign_state,
        "project.delete",
        json!({"path": a_path}),
        test_actor(),
    )
    .await;
    assert!(!foreign_delete_a.ok, "B must not delete A's capture root");
    assert_eq!(
        foreign_delete_a
            .error
            .expect("B delete of A must return a typed refusal")
            .code,
        error_codes::CONFLICT
    );
    let foreign_delete_d = dispatch(
        &foreign_state,
        "project.delete",
        json!({"path": d_path}),
        test_actor(),
    )
    .await;
    assert!(
        foreign_delete_d.ok,
        "foreign closed project deletion must remain available: {:?}",
        foreign_delete_d.error
    );

    drop(reservation);
    let opened_b = dispatch(
        &state,
        "project.open",
        json!({"path": b_path}),
        test_actor(),
    )
    .await;
    assert!(
        opened_b.ok,
        "B opens after native worker release: {:?}",
        opened_b.error
    );
    let current_name = state
        .project
        .read()
        .await
        .as_ref()
        .map(|store| store.project.name.clone());
    assert_eq!(current_name.as_deref(), Some("b"));

    let created_c = dispatch(
        &state,
        "project.create",
        json!({"name": "c", "dir": c_path}),
        test_actor(),
    )
    .await;
    assert!(
        created_c.ok,
        "project.create proceeds after native worker release: {:?}",
        created_c.error
    );
    let closed = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(
        closed.ok,
        "project.close proceeds after native worker release: {:?}",
        closed.error
    );
}
