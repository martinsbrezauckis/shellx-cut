//! Deterministic ownership regression for a held provider import versus project.open.

use super::transition_test_support::{
    create_project, transition_gate_test_lock, AssetsFetchTransitionGateReset, ONE_BY_ONE_PNG,
    TEST_TIMEOUT,
};
use super::*;
use crate::dispatch::edit_tools::{
    install_assets_fetch_project_transition_gate, AssetsFetchProjectTransitionGate,
};
use crate::events::Event;

#[tokio::test]
async fn assets_fetch_pins_project_until_import_job_admission_then_project_open_switches_cleanly() {
    let _test_lock = transition_gate_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let search_dir = root.path().join("search");
    std::fs::create_dir_all(&search_dir).unwrap();
    let source = search_dir.join("held-project-a.png");
    std::fs::write(&source, ONE_BY_ONE_PNG).unwrap();

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

    let mut events = state.events.subscribe();
    let gate = AssetsFetchProjectTransitionGate::new(source.display().to_string());
    install_assets_fetch_project_transition_gate(Some(gate.clone()));
    let _gate_reset = AssetsFetchTransitionGateReset;

    let project_pinned = gate.project_pinned.notified();
    let import_admitted = gate.import_admitted.notified();
    let fetch_state = state.clone();
    let fetch_source = source.clone();
    let fetch_search_dir = search_dir.clone();
    let fetch = tokio::spawn(async move {
        dispatch(
            &fetch_state,
            "assets.fetch",
            json!({
                "provider": "local_folder",
                "id": fetch_source,
                "kind": "image",
                "dir": fetch_search_dir,
            }),
            test_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, project_pinned)
        .await
        .expect("held fetch did not pin the project before test timeout");

    assert!(
        state.project_transition.try_lock().is_err(),
        "the held fetch must own the project-transition lock before provider resolve"
    );
    let open_started = std::sync::Arc::new(tokio::sync::Notify::new());
    let open_started_in_task = open_started.clone();
    let open_state = state.clone();
    let open = tokio::spawn(async move {
        open_started_in_task.notify_one();
        dispatch(
            &open_state,
            "project.open",
            json!({"path": b_path}),
            test_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, open_started.notified())
        .await
        .expect("project.open task did not start before test timeout");
    tokio::task::yield_now().await;
    assert!(
        !open.is_finished(),
        "project.open(B) must wait while A fetch owns the transition"
    );
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.name,
        "a",
        "B cannot become current before the held A fetch is admitted"
    );

    gate.continue_after_pin.notify_one();
    tokio::time::timeout(TEST_TIMEOUT, import_admitted)
        .await
        .expect("held fetch did not admit the A import job before test timeout");
    assert!(
        state
            .jobs
            .list()
            .iter()
            .any(|job| job.kind == "import_chain"),
        "the A import chain must be admitted before the queued project switch can drain it"
    );
    assert!(
        !open.is_finished(),
        "project.open(B) must still wait until assets.fetch returns its admitted A import"
    );

    gate.continue_after_admission.notify_one();
    let fetched = tokio::time::timeout(TEST_TIMEOUT, fetch)
        .await
        .expect("assets.fetch did not complete before test timeout")
        .expect("assets.fetch task panicked");
    assert!(fetched.ok, "A fetch failed: {:?}", fetched.error);
    let fetch_op_ids = fetched.op_ids.as_ref().expect("assets.fetch op id");
    assert_eq!(fetch_op_ids.len(), 1, "assets.fetch emits one import op");
    let fetch_op_id = fetch_op_ids[0].clone();
    let opened_b = tokio::time::timeout(TEST_TIMEOUT, open)
        .await
        .expect("project.open(B) did not complete before test timeout")
        .expect("project.open task panicked");
    assert!(
        opened_b.ok,
        "B open must switch cleanly: {:?}",
        opened_b.error
    );

    let b_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(b_state.ok, "B state failed: {:?}", b_state.error);
    assert_eq!(b_state.result.as_ref().unwrap()["name"], "b");
    assert!(
        b_state.result.as_ref().unwrap()["assets"]
            .as_object()
            .unwrap()
            .is_empty(),
        "A's fetched asset must never materialize in B"
    );
    let b_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(b_ops.ok, "B ops failed: {:?}", b_ops.error);
    assert_eq!(
        b_ops.result.as_ref().unwrap()["ops"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "B history must retain only B's project.create operation"
    );
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    let import_event = received
        .iter()
        .position(|event| matches!(event, Event::OpApplied { op } if op.op_id == fetch_op_id))
        .expect("A import must publish its durable OpApplied event");
    let b_changed = received
        .iter()
        .position(|event| matches!(event, Event::ProjectChanged { open: true, name: Some(name) } if name == "b"))
        .expect("B project.open must publish ProjectChanged");
    assert!(
        import_event < b_changed,
        "the A import event must occur before B becomes current; B receives no A mutation event"
    );
}
