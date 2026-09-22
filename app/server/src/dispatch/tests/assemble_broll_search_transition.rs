//! Assemble b-roll search-transition regression: no cross-project materialization.

use super::transition_test_support::{
    broll_actor, create_project, transition_gate_test_lock, AssetsFetchTransitionGateReset,
    ONE_BY_ONE_PNG, TEST_TIMEOUT,
};
use super::*;
use crate::dispatch::edit_tools::{install_assemble_broll_search_gate, AssembleBrollSearchGate};

#[tokio::test]
async fn assemble_broll_rejects_a_project_switch_after_search_without_mutating_either_project() {
    let _test_lock = transition_gate_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let search_dir = root.path().join("search");
    std::fs::create_dir_all(&search_dir).unwrap();
    std::fs::write(search_dir.join("held-broll.png"), ONE_BY_ONE_PNG).unwrap();

    let state = AppState::new();
    create_project(&state, "a", &a_path).await;
    create_project(&state, "b", &b_path).await;
    let reopened_a = dispatch(
        &state,
        "project.open",
        json!({"path": a_path}),
        broll_actor(),
    )
    .await;
    assert!(reopened_a.ok, "A reopen failed: {:?}", reopened_a.error);

    let gate = AssembleBrollSearchGate::new("held-broll");
    install_assemble_broll_search_gate(Some(gate.clone()));
    let _gate_reset = AssetsFetchTransitionGateReset;

    let search_completed = gate.search_completed.notified();
    let broll_state = state.clone();
    let broll_dir = search_dir.clone();
    let broll = tokio::spawn(async move {
        dispatch(
            &broll_state,
            "assemble.broll",
            json!({
                "slots": [{"query": "held-broll", "at_ms": 100, "duration_ms": 500}],
                "provider": "local_folder",
                "kind": "image",
                "dir": broll_dir,
            }),
            broll_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, search_completed)
        .await
        .expect("assemble.broll did not finish the A search before test timeout");

    let opened_b = dispatch(
        &state,
        "project.open",
        json!({"path": b_path}),
        test_actor(),
    )
    .await;
    assert!(opened_b.ok, "B open failed: {:?}", opened_b.error);
    let b_before = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(b_before.ok, "B ops failed: {:?}", b_before.error);
    let b_before_ops = b_before.result.unwrap()["ops"].as_array().unwrap().clone();

    gate.continue_after_search.notify_one();
    let rejected = tokio::time::timeout(TEST_TIMEOUT, broll)
        .await
        .expect("assemble.broll did not reject the switched project before test timeout")
        .expect("assemble.broll task panicked");
    assert!(
        rejected.ok,
        "the partial-state envelope must preserve A's checkpoint after binding fails: {rejected:?}"
    );
    let rejected_result = rejected.result.unwrap();
    assert_eq!(rejected_result["status"], "failed");
    assert_eq!(rejected_result["failed_step"], "project-binding");
    assert_eq!(
        rejected_result["error"]["code"],
        error_codes::CONFLICT,
        "post-search binding failure must preserve the typed ownership conflict"
    );
    assert_eq!(
        rejected_result["origin_checkpoint"]["id"], rejected_result["checkpoint"],
        "the retained checkpoint must be explicitly bound to the origin project"
    );
    assert_eq!(
        rejected_result["origin_checkpoint"]["project"]["project_identity"]["project_name"], "a",
        "recovery must identify A without exposing an origin path"
    );
    assert!(
        rejected_result["origin_checkpoint"]["project"]["project_revision"]
            .as_str()
            .is_some_and(|revision| !revision.is_empty()),
        "recovery must bind the retained checkpoint to A's current revision"
    );
    assert_eq!(
        rejected_result["recovery"]["action"],
        "reopen_origin_then_review_checkpoint"
    );
    assert!(
        !rejected_result["revert_hint"]
            .as_str()
            .unwrap_or_default()
            .contains("project.revert"),
        "a changed project or revision must never receive a generic revert command"
    );

    let b_after = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(
        b_after.ok,
        "B ops after rejection failed: {:?}",
        b_after.error
    );
    assert_eq!(
        b_after.result.unwrap()["ops"].as_array().unwrap(),
        &b_before_ops,
        "the A-owned b-roll request must not mutate B after the project switch"
    );
    let b_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(b_state.ok, "B state failed: {:?}", b_state.error);
    assert!(
        b_state.result.unwrap()["assets"]
            .as_object()
            .unwrap()
            .is_empty(),
        "the A-owned b-roll request must not import an asset into B"
    );

    let reopened_a = dispatch(
        &state,
        "project.open",
        json!({"path": a_path}),
        test_actor(),
    )
    .await;
    assert!(
        reopened_a.ok,
        "A reopen after rejection failed: {:?}",
        reopened_a.error
    );
    let a_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(a_ops.ok, "A ops failed: {:?}", a_ops.error);
    let a_ops_result = a_ops.result.unwrap();
    let a_ops = a_ops_result["ops"].as_array().unwrap();
    assert_eq!(
        a_ops.len(),
        3,
        "the accepted A request keeps only its checkpoint and b-roll-track setup after the later switch"
    );
    assert!(
        a_ops.iter().any(|op| op["verb"] == "project.checkpoint")
            && a_ops.iter().any(|op| op["verb"] == "edit.add_track"),
        "the preserved A partial state must remain checkpoint-revertible"
    );
    assert!(
        !a_ops
            .iter()
            .any(|op| op["verb"] == "media.import" || op["verb"] == "edit.insert"),
        "post-search rejection must not admit an asset or insert a clip into A"
    );
}
