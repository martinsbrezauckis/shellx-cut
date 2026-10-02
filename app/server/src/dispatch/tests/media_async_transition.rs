use super::transition_test_support::{
    create_project, transition_gate_test_lock, ONE_BY_ONE_PNG, TEST_TIMEOUT,
};
use super::*;
use crate::dispatch::media::{install_media_import_transition_gate, MediaImportTransitionGate};
use std::sync::Arc;

struct GateReset;

impl Drop for GateReset {
    fn drop(&mut self) {
        install_media_import_transition_gate(None);
    }
}

#[tokio::test]
async fn media_import_keeps_a_job_with_a_until_project_b_can_open() {
    let _test_lock = transition_gate_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let a_path = root_path.join("a.cutproj");
    let b_path = root_path.join("b.cutproj");
    let source = root_path.join("source.png");
    std::fs::write(&source, ONE_BY_ONE_PNG).unwrap();
    let source = source.canonicalize().unwrap();
    let state = AppState::new();
    create_project(&state, "a", &a_path).await;
    create_project(&state, "b", &b_path).await;
    assert!(
        dispatch(
            &state,
            "project.open",
            json!({"path": a_path}),
            test_actor()
        )
        .await
        .ok
    );

    let gate = MediaImportTransitionGate {
        source: source.clone(),
        committed: Arc::new(tokio::sync::Notify::new()),
        resume: Arc::new(tokio::sync::Notify::new()),
        admitted: Arc::new(tokio::sync::Notify::new()),
        resume_admitted: Arc::new(tokio::sync::Notify::new()),
    };
    install_media_import_transition_gate(Some(gate.clone()));
    let _reset = GateReset;
    let committed = gate.committed.notified();
    let import_state = state.clone();
    let import = tokio::spawn(async move {
        dispatch(
            &import_state,
            "media.import",
            json!({"path": source, "proxy": false}),
            test_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, committed)
        .await
        .expect("A import did not commit");
    assert!(
        state.project_transition.try_lock().is_err(),
        "A import must own transition through job admission"
    );
    let open_state = state.clone();
    let open = tokio::spawn(async move {
        dispatch(
            &open_state,
            "project.open",
            json!({"path": b_path}),
            test_actor(),
        )
        .await
    });
    tokio::task::yield_now().await;
    assert!(!open.is_finished(), "B open must wait for A job admission");
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.name,
        "a"
    );
    gate.resume.notify_one();
    tokio::time::timeout(TEST_TIMEOUT, gate.admitted.notified())
        .await
        .expect("A import job was not admitted");
    assert!(state
        .jobs
        .list()
        .iter()
        .any(|job| job.kind == "import_chain"));
    assert!(
        !open.is_finished(),
        "B open must still wait after A job admission"
    );
    gate.resume_admitted.notify_one();
    let imported = tokio::time::timeout(TEST_TIMEOUT, import)
        .await
        .unwrap()
        .unwrap();
    assert!(imported.ok, "A import failed: {:?}", imported.error);
    let opened = tokio::time::timeout(TEST_TIMEOUT, open)
        .await
        .unwrap()
        .unwrap();
    assert!(opened.ok, "B open failed: {:?}", opened.error);
    let b = state.project.read().await;
    assert_eq!(b.as_ref().unwrap().project.name, "b");
    assert!(
        b.as_ref().unwrap().project.assets.is_empty(),
        "A import must not enrich B"
    );
}
