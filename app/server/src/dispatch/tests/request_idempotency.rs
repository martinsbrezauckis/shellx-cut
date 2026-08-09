use super::*;

async fn create_project(state: &AppState, dir: &std::path::Path) -> VerbResult {
    dispatch(
        state,
        "project.create",
        json!({
            "name": "request-test",
            "dir": dir.join("request-test.cutproj"),
        }),
        test_actor(),
    )
    .await
}

#[tokio::test]
async fn mutation_retry_replays_exact_response_without_duplicate_op() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = create_project(&state, root.path()).await;
    assert!(created.ok, "{:?}", created.error);
    let baseline = created.project_revision.as_deref().unwrap();
    let args = json!({
        "at_ms": 120,
        "label": "idempotent",
        "request_id": "request-marker-0001",
        "expected_revision": baseline,
    });

    let first = dispatch(&state, "edit.add_marker", args.clone(), test_actor()).await;
    assert!(first.ok, "{:?}", first.error);
    assert_eq!(first.op_ids.as_ref().unwrap(), &["op_000002".to_string()]);
    assert_eq!(first.project_revision.as_deref(), Some("op_000002"));

    let retry = dispatch(&state, "edit.add_marker", args, test_actor()).await;
    assert_eq!(
        retry, first,
        "a lost-response retry returns the durable envelope"
    );

    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let marker_count = ops.result.unwrap()["ops"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["verb"] == "edit.add_marker")
        .count();
    assert_eq!(marker_count, 1);
}

#[tokio::test]
async fn request_identity_survives_reopen_and_changed_payload_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("request-test.cutproj");
    let state = AppState::new();
    let created = create_project(&state, root.path()).await;
    let baseline = created.project_revision.unwrap();
    let original = json!({
        "at_ms": 240,
        "label": "durable",
        "request_id": "request-marker-0002",
        "expected_revision": baseline,
    });
    let first = dispatch(&state, "edit.add_marker", original.clone(), test_actor()).await;
    assert!(first.ok, "{:?}", first.error);

    assert!(
        dispatch(&state, "project.close", json!({}), test_actor())
            .await
            .ok
    );
    assert!(
        dispatch(
            &state,
            "project.open",
            json!({"path": project_dir}),
            test_actor(),
        )
        .await
        .ok
    );
    let reopened_retry = dispatch(&state, "edit.add_marker", original, test_actor()).await;
    assert_eq!(reopened_retry, first);

    let changed = dispatch(
        &state,
        "edit.add_marker",
        json!({
            "at_ms": 241,
            "label": "changed",
            "request_id": "request-marker-0002",
            "expected_revision": baseline,
        }),
        test_actor(),
    )
    .await;
    assert!(!changed.ok);
    assert_eq!(changed.error.unwrap().code, error_codes::CONFLICT);
}

#[tokio::test]
async fn stale_expected_revision_fails_before_mutating() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = create_project(&state, root.path()).await;
    assert!(created.ok);
    let before = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let before_count = before.result.unwrap()["ops"].as_array().unwrap().len();

    let stale = dispatch(
        &state,
        "edit.add_marker",
        json!({
            "at_ms": 360,
            "label": "stale",
            "request_id": "request-marker-0003",
            "expected_revision": "op_999999",
        }),
        test_actor(),
    )
    .await;
    assert!(!stale.ok);
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let after = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert_eq!(
        after.result.unwrap()["ops"].as_array().unwrap().len(),
        before_count
    );
}

#[tokio::test]
async fn concurrent_mutations_from_one_revision_commit_only_once() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = create_project(&state, root.path()).await;
    let baseline = created.project_revision.unwrap();
    let left_state = state.clone();
    let left_revision = baseline.clone();
    let left = tokio::spawn(async move {
        dispatch(
            &left_state,
            "edit.add_marker",
            json!({
                "at_ms": 400,
                "label": "left",
                "request_id": "request-race-left",
                "expected_revision": left_revision,
            }),
            test_actor(),
        )
        .await
    });
    let right_state = state.clone();
    let right = tokio::spawn(async move {
        dispatch(
            &right_state,
            "edit.add_marker",
            json!({
                "at_ms": 500,
                "label": "right",
                "request_id": "request-race-right",
                "expected_revision": baseline,
            }),
            test_actor(),
        )
        .await
    });
    let outcomes = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(outcomes.iter().filter(|result| result.ok).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| result
                .error
                .as_ref()
                .is_some_and(|error| error.code == error_codes::CONFLICT))
            .count(),
        1
    );
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert_eq!(
        ops.result.unwrap()["ops"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|op| op["verb"] == "edit.add_marker")
            .count(),
        1
    );
}
