//! Assemble b-roll same-project revision-transition regression.

use super::transition_test_support::{
    broll_actor, create_project, transition_gate_test_lock, AssetsFetchTransitionGateReset,
    TEST_TIMEOUT,
};
use super::*;
use crate::dispatch::edit_tools::{install_assemble_broll_search_gate, AssembleBrollSearchGate};

#[tokio::test]
async fn assemble_broll_reports_the_latest_origin_revision_after_a_prior_slot() {
    let _test_lock = transition_gate_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let project_path = root.path().join("a.cutproj");
    let search_dir = root.path().join("search");
    std::fs::create_dir_all(&search_dir).unwrap();
    let source = search_dir.join("first-broll-second-broll.mp4");
    let maintained_fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/first-edit-sample.mp4");
    std::fs::copy(&maintained_fixture, &source).expect("copy maintained muxed media fixture");

    let state = AppState::new();
    create_project(&state, "a", &project_path).await;
    let gate = AssembleBrollSearchGate::new("second-broll");
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
                "slots": [
                    {"query": "first-broll", "at_ms": 100, "duration_ms": 500},
                    {"query": "second-broll", "at_ms": 700, "duration_ms": 500}
                ],
                "provider": "local_folder",
                "kind": "video",
                "dir": broll_dir,
            }),
            broll_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, search_completed)
        .await
        .expect("second b-roll search did not complete after the first slot");

    let before_change = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(
        before_change.ok,
        "A state before change failed: {:?}",
        before_change.error
    );
    let expected_origin_revision = before_change.result.unwrap()["project_revision"].clone();
    let before_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(
        before_ops.ok,
        "A ops before change failed: {:?}",
        before_ops.error
    );
    assert!(
        before_ops.result.unwrap()["ops"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["verb"] == "edit.insert"),
        "the first slot must finish before the second search is gated"
    );

    let intervening = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 600, "label": "intervening edit"}),
        test_actor(),
    )
    .await;
    assert!(
        intervening.ok,
        "intervening A edit failed: {:?}",
        intervening.error
    );
    let current_revision = intervening
        .project_revision
        .clone()
        .expect("intervening A edit must advance its revision");

    gate.continue_after_search.notify_one();
    let rejected = tokio::time::timeout(TEST_TIMEOUT, broll)
        .await
        .expect("same-project revision conflict did not settle")
        .expect("b-roll task panicked");
    assert!(
        rejected.ok,
        "partial failure must stay in the result envelope: {:?}",
        rejected.error
    );
    let result = rejected.result.unwrap();
    assert_eq!(result["status"], "failed");
    assert_eq!(result["slot"], 1);
    assert_eq!(result["failed_step"], "project-binding");
    assert_eq!(result["error"]["code"], error_codes::CONFLICT);
    assert_eq!(result["placed"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["origin_checkpoint"]["project"]["project_identity"]["project_name"],
        "a"
    );
    assert_eq!(
        result["origin_checkpoint"]["project"]["project_revision"], expected_origin_revision,
        "the delayed conflict must retain the latest A pin after its first slot"
    );
    assert_ne!(
        result["origin_checkpoint"]["project"]["project_revision"], current_revision,
        "the response must expose the accepted revision to compare with the intervening edit"
    );
    let guidance = result["recovery"]["guidance"].as_str().unwrap_or_default();
    assert!(guidance.contains("compare") && guidance.contains("intervening"));
    assert!(!guidance.contains("revision match") && !guidance.contains("undo"));

    let final_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(
        final_ops.ok,
        "A ops after conflict failed: {:?}",
        final_ops.error
    );
    let final_ops_result = final_ops.result.unwrap();
    let final_ops = final_ops_result["ops"].as_array().unwrap();
    assert_eq!(
        final_ops
            .iter()
            .filter(|op| op["verb"] == "edit.insert")
            .count(),
        1,
        "the second slot must not insert after the same-project revision conflict"
    );
}
