use super::*;

pub(super) async fn create_project(state: &AppState, dir: &std::path::Path) -> VerbResult {
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

#[tokio::test]
async fn voiceover_out_request_identity_survives_real_dispatch_preparation() {
    let state = AppState::new();
    // No microphone is opened: reaching the active-owner refusal proves that
    // real registry + shared request preparation + coordinator parsing retained
    // the correlation identity, rather than reporting a phantom missing field.
    for session in ["a".repeat(22), "b".repeat(22)] {
        let result = dispatch(
            &state,
            "voiceover.observe_playhead",
            json!({
                "owner_session_id": session,
                "owner_capability": "c".repeat(43),
                "request_id": "voiceover-out-request-1",
                "request_fingerprint": "a".repeat(64),
                "bridge_epoch": 1,
                "playhead_ms": 9985,
            }),
            test_actor(),
        )
        .await;
        assert!(!result.ok);
        let error = result.error.unwrap();
        assert_eq!(error.code, error_codes::NOT_FOUND, "{error:?}");
        assert_eq!(error.message, "no active voiceover take");
    }
}

#[tokio::test]
async fn voiceover_out_dispatch_still_refuses_missing_and_malformed_identity() {
    let state = AppState::new();
    for identity in [None, Some(json!(42))] {
        let mut args = json!({
            "owner_session_id": "b".repeat(22),
            "owner_capability": "c".repeat(43),
            "request_fingerprint": "a".repeat(64),
            "bridge_epoch": 1,
            "playhead_ms": 9985,
        });
        if let Some(identity) = identity {
            args["request_id"] = identity;
        }
        let result = dispatch(&state, "voiceover.observe_playhead", args, test_actor()).await;
        assert!(!result.ok);
        assert_eq!(result.error.unwrap().code, error_codes::INVALID_ARGS);
    }
}

#[test]
fn voiceover_out_preparation_preserves_domain_identity_without_new_retry_metadata() {
    let args = json!({
        "request_id": "voiceover-out-request-1",
        "owner_session_id": "a".repeat(22),
        "owner_capability": "c".repeat(43),
        "request_fingerprint": "a".repeat(64),
        "bridge_epoch": 1,
        "playhead_ms": 9985,
    });
    let actor = test_actor();
    let prepared =
        crate::request_control::prepare("voiceover.observe_playhead", args.clone(), actor.clone())
            .unwrap();
    assert!(!prepared.controlled);
    assert_eq!(prepared.args, args);
    assert_eq!(prepared.actor, actor);
    for verb in ["edit.add_marker", "voiceover.start"] {
        let prepared = crate::request_control::prepare(
            verb,
            json!({"request_id":"retry-1", "expected_revision":"op_000001"}),
            test_actor(),
        )
        .unwrap();
        assert!(prepared.controlled);
        assert!(prepared.args.get("request_id").is_none());
        assert!(prepared.args.get("expected_revision").is_none());
        let request = prepared.actor.request.unwrap();
        assert_eq!(request.request_id, "retry-1");
        assert_eq!(request.expected_revision.as_deref(), Some("op_000001"));
    }
}

#[tokio::test]
async fn voiceover_out_domain_id_does_not_replay_or_conflict_with_a_durable_mutation() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = create_project(&state, root.path()).await;
    assert!(created.ok);
    let marker_args = json!({
        "request_id": "voiceover-out-request-1",
        "expected_revision": created.project_revision.unwrap(),
        "at_ms": 1,
        "label": "durable collision witness",
    });
    let marker = dispatch(&state, "edit.add_marker", marker_args.clone(), test_actor()).await;
    assert!(marker.ok, "{:?}", marker.error);
    let observed = dispatch(
        &state,
        "voiceover.observe_playhead",
        json!({
            "request_id": "voiceover-out-request-1",
            "owner_session_id": "b".repeat(22),
            "owner_capability": "c".repeat(43),
            "request_fingerprint": "a".repeat(64),
            "bridge_epoch": 1,
            "playhead_ms": 9985,
        }),
        test_actor(),
    )
    .await;
    let error = observed.error.unwrap();
    assert_eq!(error.code, error_codes::NOT_FOUND, "{error:?}");
    assert_eq!(error.message, "no active voiceover take");
    let retry = dispatch(&state, "edit.add_marker", marker_args, test_actor()).await;
    assert_eq!(
        retry, marker,
        "Out must not replace another durable retry receipt"
    );
}
