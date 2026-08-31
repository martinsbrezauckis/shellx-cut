//! Request-idempotency coverage owned by reviewed compound-action rejection.

use super::request_idempotency::create_project;
use super::*;

async fn seed_grouped_insert_asset(state: &AppState, root: &std::path::Path) {
    let media = root.join("compound-group-source.mp4");
    std::fs::write(&media, b"compound group test source").unwrap();
    let imported = dispatch(state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    update_asset(state, "a1", |asset| {
        asset.probe = Some(json!({
            "kind": "video",
            "width": 1920,
            "height": 1080,
            "duration_ms": 2_000,
            "has_audio": false,
        }));
    })
    .await
    .unwrap();
}

async fn grouped_insert(state: &AppState, at_ms: u64, group_id: &str) -> VerbResult {
    dispatch(
        state,
        "edit.insert",
        json!({
            "asset": "a1",
            "track": "v1",
            "at_ms": at_ms,
            "src_range_ms": [0, 500],
            "ripple": false,
            "group_id": group_id,
        }),
        test_actor(),
    )
    .await
}

#[tokio::test]
async fn compound_group_reject_is_guarded_duplicate_safe_restart_safe_and_undoable() {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("request-test.cutproj");
    let state = AppState::new();
    assert!(create_project(&state, root.path()).await.ok);
    seed_grouped_insert_asset(&state, root.path()).await;
    let first_member = grouped_insert(&state, 0, "review-inserts-01").await;
    assert!(first_member.ok, "{:?}", first_member.error);
    let last_member = grouped_insert(&state, 600, "review-inserts-01").await;
    assert!(last_member.ok, "{:?}", last_member.error);
    let first_op_id = first_member.op_ids.unwrap()[0].clone();
    let last_op_id = last_member.op_ids.unwrap()[0].clone();

    let preview = dispatch(
        &state,
        "project.group_preview",
        json!({"op_id": first_op_id}),
        test_actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    let preview = preview.result.unwrap();
    assert_eq!(preview["group"]["first_op_id"], first_op_id);
    assert_eq!(preview["group"]["last_op_id"], last_op_id);
    assert_eq!(preview["group"]["operation_count"], 2);
    assert_eq!(preview["reject"]["status"], "ready");
    let reject_args = json!({
        "first_op_id": preview["group"]["first_op_id"],
        "last_op_id": preview["group"]["last_op_id"],
        "preview_hash": preview["preview_hash"],
        "rationale": "reject the linked visual callout",
        "request_id": "request-compound-reject-0001",
        "expected_revision": preview["project_revision"],
    });

    let missing_controls = dispatch(
        &state,
        "project.group_reject",
        json!({
            "first_op_id": preview["group"]["first_op_id"],
            "last_op_id": preview["group"]["last_op_id"],
            "preview_hash": preview["preview_hash"],
        }),
        test_actor(),
    )
    .await;
    assert!(!missing_controls.ok);
    assert_eq!(
        missing_controls.error.unwrap().code,
        error_codes::INVALID_ARGS,
        "a destructive reviewed action must have an exact retry identity and revision"
    );

    let rejected = dispatch(
        &state,
        "project.group_reject",
        reject_args.clone(),
        test_actor(),
    )
    .await;
    assert!(rejected.ok, "{:?}", rejected.error);
    assert_eq!(rejected.op_ids.as_ref().unwrap().len(), 1);
    assert_eq!(
        rejected.result.as_ref().unwrap()["scope"]["generic_transaction_replay"],
        false
    );

    let duplicate = dispatch(
        &state,
        "project.group_reject",
        reject_args.clone(),
        test_actor(),
    )
    .await;
    assert_eq!(
        duplicate, rejected,
        "a lost response must replay the exact durable group-reject result"
    );

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
    let reopened_retry = dispatch(&state, "project.group_reject", reject_args, test_actor()).await;
    assert_eq!(
        reopened_retry, rejected,
        "request receipt survives reopen and prevents a second restore"
    );

    assert!(
        dispatch(&state, "project.undo", json!({}), test_actor())
            .await
            .ok,
        "one undo restores the whole rejected compound action"
    );
    let after_undo = dispatch(
        &state,
        "project.group_preview",
        json!({"op_id": first_op_id}),
        test_actor(),
    )
    .await;
    assert!(after_undo.ok, "{:?}", after_undo.error);
    assert_eq!(after_undo.result.unwrap()["reject"]["status"], "ready");
}

#[tokio::test]
async fn compound_group_reject_refuses_a_stale_preview_without_appending() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    assert!(create_project(&state, root.path()).await.ok);
    seed_grouped_insert_asset(&state, root.path()).await;
    let first_member = grouped_insert(&state, 0, "review-inserts-02").await;
    let last_member = grouped_insert(&state, 600, "review-inserts-02").await;
    assert!(first_member.ok && last_member.ok);
    let preview = dispatch(
        &state,
        "project.group_preview",
        json!({"op_id": first_member.op_ids.unwrap()[0]}),
        test_actor(),
    )
    .await
    .result
    .unwrap();
    let before = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let before_len = before.result.unwrap()["ops"].as_array().unwrap().len();

    let later = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 1200, "label": "newer review action"}),
        test_actor(),
    )
    .await;
    assert!(later.ok, "{:?}", later.error);
    let stale = dispatch(
        &state,
        "project.group_reject",
        json!({
            "first_op_id": preview["group"]["first_op_id"],
            "last_op_id": preview["group"]["last_op_id"],
            "preview_hash": preview["preview_hash"],
            "request_id": "request-compound-reject-0002",
            "expected_revision": preview["project_revision"],
        }),
        test_actor(),
    )
    .await;
    assert!(!stale.ok);
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let after = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert_eq!(
        after.result.unwrap()["ops"].as_array().unwrap().len(),
        before_len + 1,
        "only the deliberately newer edit appended; stale rejection did not"
    );
}
