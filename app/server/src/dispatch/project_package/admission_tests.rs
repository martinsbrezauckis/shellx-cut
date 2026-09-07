//! Focused durable-admission and project-transition package regressions.

use super::transition_gate::{
    install_package_project_transition_gate, PackageProjectTransitionGate,
};
use super::*;
use crate::dispatch::dispatch;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

fn actor() -> Actor {
    Actor {
        kind: cut_core::ActorKind::Agent,
        name: "portable-admission-test".into(),
        via: "test".into(),
        request: None,
    }
}

async fn wait_for_job(state: &AppState, job_id: &str) -> crate::jobs::JobRecord {
    for _ in 0..100 {
        let record = state.jobs.get(job_id).expect("package job exists");
        if matches!(
            record.state,
            crate::jobs::JobState::Done | crate::jobs::JobState::Failed
        ) {
            return record;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("portable package job did not finish")
}

struct PackageTransitionGateReset;

impl Drop for PackageTransitionGateReset {
    fn drop(&mut self) {
        install_package_project_transition_gate(None);
    }
}

#[tokio::test]
async fn package_refuses_non_durable_admission_before_creating_a_destination() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("packages");
    fs::create_dir(&destination).unwrap();
    let state = AppState::new();
    assert!(
        dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await
        .ok
    );
    let planned = dispatch(
        &state,
        "project.package_plan",
        json!({"destination": destination, "name": "durability"}),
        actor(),
    )
    .await;
    assert!(planned.ok, "package plan failed: {:?}", planned.error);
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    let jobs_dir = project_dir.join("jobs");
    fs::remove_dir_all(&jobs_dir).unwrap();
    fs::write(&jobs_dir, b"not a directory").unwrap();

    let created = dispatch(
        &state,
        "project.package_create",
        json!({
            "destination": destination,
            "name": "durability",
            "plan_hash": planned.result.unwrap()["plan_hash"],
        }),
        actor(),
    )
    .await;
    assert!(!created.ok, "non-durable package admission must fail");
    assert_eq!(created.error.unwrap().code, error_codes::IO);
    assert!(
        state.jobs.list().is_empty(),
        "no package job may start in memory"
    );
    assert!(
        fs::read_dir(destination).unwrap().next().is_none(),
        "a rejected package job must not create a final or staging destination"
    );
}

#[tokio::test]
async fn package_create_pins_a_project_through_durable_job_admission() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("packages");
    fs::create_dir(&destination).unwrap();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let state = AppState::new();
    for (name, path) in [("a", &a_path), ("b", &b_path)] {
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": path}),
            actor(),
        )
        .await;
        assert!(created.ok, "{name} create failed: {:?}", created.error);
    }
    let a_revision = ProjectStore::open(&a_path)
        .unwrap()
        .log
        .current_revision()
        .unwrap();
    let b_revision = ProjectStore::open(&b_path)
        .unwrap()
        .log
        .current_revision()
        .unwrap();
    assert_eq!(
        a_revision, b_revision,
        "fixture requires colliding local revisions"
    );
    let reopened_a = dispatch(&state, "project.open", json!({"path": a_path}), actor()).await;
    assert!(reopened_a.ok, "A reopen failed: {:?}", reopened_a.error);
    let planned = dispatch(
        &state,
        "project.package_plan",
        json!({"destination": destination, "name": "held-transition"}),
        actor(),
    )
    .await;
    assert!(planned.ok, "package plan failed: {:?}", planned.error);
    let plan_hash = planned.result.unwrap()["plan_hash"].clone();

    let gate = PackageProjectTransitionGate::new("held-transition");
    install_package_project_transition_gate(Some(gate.clone()));
    let _gate_reset = PackageTransitionGateReset;
    let project_pinned = gate.project_pinned.notified();
    let job_admitted = gate.job_admitted.notified();
    let package_state = state.clone();
    let package = tokio::spawn(async move {
        dispatch(
            &package_state,
            "project.package_create",
            json!({
                "destination": destination,
                "name": "held-transition",
                "plan_hash": plan_hash,
            }),
            actor(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), project_pinned)
        .await
        .expect("package did not pin project A before timeout");
    assert!(
        state.project_transition.try_lock().is_err(),
        "package must own project-transition before preparing A"
    );

    let open_started = Arc::new(tokio::sync::Notify::new());
    let open_started_in_task = open_started.clone();
    let open_state = state.clone();
    let open = tokio::spawn(async move {
        open_started_in_task.notify_one();
        dispatch(
            &open_state,
            "project.open",
            json!({"path": b_path}),
            actor(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), open_started.notified())
        .await
        .expect("project.open(B) task did not start");
    tokio::task::yield_now().await;
    assert!(
        !open.is_finished(),
        "project.open(B) must wait while package admission owns A"
    );
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.name,
        "a",
        "B cannot become current before A's package job is durable"
    );

    gate.continue_after_pin.notify_one();
    tokio::time::timeout(Duration::from_secs(2), job_admitted)
        .await
        .expect("package did not admit its durable A job before timeout");
    let job_id = state
        .jobs
        .list()
        .into_iter()
        .find(|job| job.kind == PACKAGE_JOB_KIND)
        .expect("A package job must be present before B can switch")
        .job_id;
    let job = wait_for_job(&state, &job_id).await;
    assert_eq!(job.state, crate::jobs::JobState::Done, "{:?}", job.error);
    assert!(
        !open.is_finished(),
        "B must still wait until package admission returns"
    );

    gate.continue_after_admission.notify_one();
    let created = tokio::time::timeout(Duration::from_secs(2), package)
        .await
        .expect("package request did not return")
        .expect("package task panicked");
    assert!(created.ok, "package create failed: {:?}", created.error);
    let opened_b = tokio::time::timeout(Duration::from_secs(2), open)
        .await
        .expect("project.open(B) did not return")
        .expect("project.open(B) task panicked");
    assert!(opened_b.ok, "B open failed: {:?}", opened_b.error);
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.name,
        "b"
    );
    assert!(
        state.jobs.list().is_empty(),
        "B must not inherit A's package job record"
    );
}

#[tokio::test]
async fn final_package_publication_refuses_a_different_project_with_same_revision() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("packages");
    fs::create_dir(&destination).unwrap();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let state = AppState::new();
    for (name, path) in [("a", &a_path), ("b", &b_path)] {
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name": name, "dir": path}),
                actor(),
            )
            .await
            .ok
        );
    }
    assert!(
        dispatch(&state, "project.open", json!({"path": a_path}), actor(),)
            .await
            .ok
    );
    let prepared = prepare(
        &state,
        destination.to_string_lossy().into_owned(),
        "identity-guard".into(),
        None,
    )
    .await
    .unwrap();
    let b_revision = ProjectStore::open(&b_path)
        .unwrap()
        .log
        .current_revision()
        .unwrap();
    assert_eq!(
        prepared.source.project_revision,
        b_revision.unwrap(),
        "fixture requires the same local operation revision"
    );
    assert!(
        dispatch(&state, "project.open", json!({"path": b_path}), actor(),)
            .await
            .ok
    );
    let stage = root.path().join("private-stage.cutproj");
    let target = destination.join("identity-guard.cutproj");
    fs::create_dir(&stage).unwrap();
    // The package worker performs its final source check in spawn_blocking,
    // where the synchronous read guard is legal. Keep this direct guard test
    // on that same boundary instead of invoking blocking_read from Tokio.
    let publish_state = state.clone();
    let publish_source = prepared.source.clone();
    let publish_stage = stage.clone();
    let publish_target = target.clone();
    let error = tokio::task::spawn_blocking(move || {
        source_revision_then_publish(
            &publish_state,
            &publish_source,
            &crate::jobs::JobCancellation::test_active(),
            &publish_stage,
            &publish_target,
        )
    })
    .await
    .expect("final package publication worker must not panic")
    .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(
        stage.exists(),
        "A stage must not publish through B's revision"
    );
    assert!(!target.exists(), "B must not receive A's package output");
}
