//! Assemble b-roll owner-transition regression: no cross-project materialization.

use super::transition_test_support::{
    broll_actor, create_project, transition_gate_test_lock, AssetsFetchTransitionGateReset,
    TEST_TIMEOUT,
};
use super::*;
use crate::dispatch::edit_tools::{
    install_assets_fetch_project_transition_gate, AssetsFetchProjectTransitionGate,
};
use std::time::Duration;

// Enrichment runs real media work after the deterministic transition gate releases.
// Bound inactivity, not the total duration of a healthy, progressing job.
const BACKGROUND_JOB_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

async fn wait_for_owned_jobs_to_settle(state: &AppState, job_ids: &[String]) {
    let mut events = state.events.subscribe();
    loop {
        let active = job_ids
            .iter()
            .filter_map(|job_id| state.jobs.get(job_id))
            .filter(|job| {
                matches!(
                    job.state,
                    crate::jobs::JobState::Queued | crate::jobs::JobState::Running
                )
            })
            .collect::<Vec<_>>();
        if active.is_empty() {
            return;
        }
        let progress = tokio::time::timeout(BACKGROUND_JOB_IDLE_TIMEOUT, async {
            loop {
                match events.recv().await {
                    Ok(crate::events::Event::JobProgress { job_id, .. })
                        if active.iter().any(|job| job.job_id == job_id) =>
                    {
                        return;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        panic!("job event stream closed while waiting for b-roll jobs")
                    }
                }
            }
        })
        .await;
        if progress.is_err() {
            let active = job_ids
                .iter()
                .filter_map(|job_id| state.jobs.get(job_id))
                .filter(|job| {
                    matches!(
                        job.state,
                        crate::jobs::JobState::Queued | crate::jobs::JobState::Running
                    )
                })
                .map(|job| {
                    format!(
                        "{} ({}, {:?}, {:.0}%)",
                        job.job_id,
                        job.kind,
                        job.state,
                        job.progress * 100.0
                    )
                })
                .collect::<Vec<_>>();
            if !active.is_empty() {
                panic!("b-roll background jobs had no progress before project switch: {active:?}");
            }
        }
    }
}

#[tokio::test]
async fn assemble_broll_holds_its_owner_through_import_then_records_the_calling_actor_checkpoint() {
    let _test_lock = transition_gate_test_lock().lock().await;
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let a_path = root_path.join("a.cutproj");
    let b_path = root_path.join("b.cutproj");
    let search_dir = root_path.join("search");
    std::fs::create_dir_all(&search_dir).unwrap();
    let source = search_dir.join("pinned-broll.mp4");
    let maintained_fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/first-edit-sample.mp4");
    std::fs::copy(&maintained_fixture, &source).expect("copy maintained muxed media fixture");
    let source = source.canonicalize().unwrap();

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

    let gate = AssetsFetchProjectTransitionGate::new(source.display().to_string());
    install_assets_fetch_project_transition_gate(Some(gate.clone()));
    let _gate_reset = AssetsFetchTransitionGateReset;

    let project_pinned = gate.project_pinned.notified();
    let import_admitted = gate.import_admitted.notified();
    let broll_state = state.clone();
    let broll_dir = search_dir.clone();
    let broll = tokio::spawn(async move {
        dispatch(
            &broll_state,
            "assemble.broll",
            json!({
                "slots": [{"query": "pinned-broll", "at_ms": 100, "duration_ms": 500}],
                "provider": "local_folder",
                "kind": "video",
                "dir": broll_dir,
            }),
            broll_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, project_pinned)
        .await
        .expect("assemble.broll did not pin A before fetch admission");

    let open_started = std::sync::Arc::new(tokio::sync::Notify::new());
    let open_started_in_task = open_started.clone();
    let open_state = state.clone();
    let b_path_for_open = b_path.clone();
    let open = tokio::spawn(async move {
        open_started_in_task.notify_one();
        dispatch(
            &open_state,
            "project.open",
            json!({"path": b_path_for_open}),
            test_actor(),
        )
        .await
    });
    tokio::time::timeout(TEST_TIMEOUT, open_started.notified())
        .await
        .expect("project.open(B) task did not start before test timeout");
    tokio::task::yield_now().await;
    assert!(
        !open.is_finished(),
        "project.open(B) must wait while b-roll admits A's import"
    );
    assert!(
        state.project_transition.try_lock().is_err(),
        "assemble.broll must hold A's project-transition lock through fetch admission"
    );

    gate.continue_after_pin.notify_one();
    tokio::time::timeout(TEST_TIMEOUT, import_admitted)
        .await
        .expect("assemble.broll did not admit the A import before test timeout");
    assert!(
        !open.is_finished(),
        "project.open(B) must wait while b-roll confirms the A import is done"
    );
    assert!(
        state.project_transition.try_lock().is_err(),
        "assemble.broll must keep A's project-transition lock while its import is admitted"
    );
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.name,
        "a",
        "B cannot become current before the held A b-roll import is placed"
    );

    // The queued open has now proved it cannot cross the b-roll materialization
    // boundary.  It must not be released into project-switch draining: completing
    // the import starts independent proxy/enrichment work, whose cancellation is
    // deliberately fail-closed and belongs to the jobs tests rather than this
    // owner/checkpoint regression.
    open.abort();
    assert!(
        open.await
            .expect_err("the held project.open(B) task must be cancelled")
            .is_cancelled(),
        "the held B open must not run its project-switch drain before b-roll releases A"
    );
    gate.continue_after_admission.notify_one();

    let assembled = tokio::time::timeout(TEST_TIMEOUT, broll)
        .await
        .expect("assemble.broll video import did not finish before test timeout")
        .expect("assemble.broll task panicked");
    assert!(assembled.ok, "assemble.broll failed: {:?}", assembled.error);
    let assembled_result = assembled.result.unwrap();
    assert_eq!(assembled_result["status"], "ok");
    assert_eq!(assembled_result["slots_filled"], 1);
    assert_eq!(assembled_result["placed"][0]["at_ms"], 100);
    assert_eq!(assembled_result["placed"][0]["duration_ms"], 500);
    let asset_id = assembled_result["placed"][0]["asset_id"]
        .as_str()
        .expect("success result must name the admitted A asset")
        .to_string();
    let clip_id = assembled_result["placed"][0]["clip_id"]
        .as_str()
        .expect("success result must name the inserted A clip")
        .to_string();

    let broll_jobs = state.jobs.list();
    for expected_kind in ["import_chain", "proxy", "enrich"] {
        assert!(
            broll_jobs.iter().any(|job| job.kind == expected_kind),
            "assemble.broll must retain its {expected_kind} job for an explicit terminal wait"
        );
    }
    let broll_job_ids = broll_jobs
        .iter()
        .map(|job| job.job_id.clone())
        .collect::<Vec<_>>();
    wait_for_owned_jobs_to_settle(&state, &broll_job_ids).await;

    let opened_b = dispatch(
        &state,
        "project.open",
        json!({"path": b_path}),
        test_actor(),
    )
    .await;
    assert!(opened_b.ok, "B open failed: {:?}", opened_b.error);
    let b_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(b_state.ok, "B state failed: {:?}", b_state.error);
    assert!(
        b_state.result.unwrap()["assets"]
            .as_object()
            .unwrap()
            .is_empty(),
        "B must not receive the A-owned imported asset or inserted clip"
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
        "A reopen after assembly failed: {:?}",
        reopened_a.error
    );
    let a_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(a_state.ok, "A state failed: {:?}", a_state.error);
    let a_state = a_state.result.unwrap();
    assert!(
        a_state["assets"].get(&asset_id).is_some(),
        "reopening A must deserialize the exact asset returned by assemble.broll"
    );
    let broll_track = a_state["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == "broll")
        .expect("A must deserialize the auto-created b-roll track");
    let clips = broll_track["clips"].as_array().unwrap();
    assert_eq!(clips[0]["kind"], "gap");
    assert_eq!(
        clips[0]["duration_ms"], 100,
        "the serialized gap preserves the requested editorial start"
    );
    let inserted = clips
        .iter()
        .find(|clip| clip["id"] == clip_id)
        .expect("A must deserialize the exact clip returned by assemble.broll");
    assert_eq!(inserted["asset"], asset_id);
    assert_eq!(inserted["src_in_ms"], 0);
    assert_eq!(inserted["src_out_ms"], 500);
    let a_ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(a_ops.ok, "A ops failed: {:?}", a_ops.error);
    let a_ops = a_ops.result.unwrap()["ops"].as_array().unwrap().clone();
    let checkpoint = a_ops
        .iter()
        .find(|op| op["verb"] == "project.checkpoint")
        .expect("assemble.broll must create its automatic checkpoint");
    assert_eq!(checkpoint["actor"]["name"], "broll-owner");
    assert_eq!(checkpoint["actor"]["via"], "test");
    assert!(
        a_ops.iter().any(|op| op["verb"] == "edit.insert"),
        "the successfully imported A video must be explicitly inserted"
    );
}
