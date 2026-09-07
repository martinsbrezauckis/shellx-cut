use super::*;
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use serde_json::json;
use std::ffi::OsStr;

async fn state_with_project() -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name": "cache", "dir": root.path().join("cache.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "project create: {:?}", created.error);
    (root, state)
}

#[tokio::test]
async fn successful_close_invalidates_an_unconsumed_cache_purge_plan() {
    let (_root, state) = state_with_project().await;
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(preview.ok, "preview: {:?}", preview.error);
    let plan_id = preview.result.unwrap()["plan_id"].clone();

    let close = crate::dispatch::dispatch(
        &state,
        "project.close",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(close.ok, "close: {:?}", close.error);
    assert!(state.cache_purge_plan.lock().await.is_none());
    let stale = crate::dispatch::dispatch(
        &state,
        "project.cache_purge",
        json!({"plan_id": plan_id, "confirm": true}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!stale.ok, "a prior-project opaque plan cannot be consumed");
}

#[tokio::test]
async fn purge_refuses_non_durable_admission_without_consuming_its_plan() {
    let (_root, state) = state_with_project().await;
    let (project_dir, output) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let output = store.proxies_dir().join("a1.mp4");
        std::fs::write(&output, b"proxy").unwrap();
        (store.dir.clone(), output)
    };
    std::fs::File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        .unwrap();
    crate::cache_lifecycle::record_generated(
        &project_dir,
        CacheKind::Proxies,
        "a1",
        OsStr::new("a1.mp4"),
    )
    .unwrap();
    let preview = preview(&state).await.unwrap();
    let plan_id = preview.result.as_ref().unwrap()["plan_id"].clone();
    assert_eq!(preview.result.as_ref().unwrap()["purgeable"]["files"], 1);
    let jobs_dir = project_dir.join("jobs");
    std::fs::remove_dir_all(&jobs_dir).unwrap();
    std::fs::write(&jobs_dir, b"not a directory").unwrap();

    let error = start_purge(&state, json!({"plan_id": plan_id, "confirm": true}))
        .await
        .expect_err("purge must refuse non-durable job admission");
    assert_eq!(error.code, error_codes::IO);
    assert!(
        state.cache_purge_plan.lock().await.is_some(),
        "a rejected admission must retain the one-use deletion plan"
    );
    assert!(
        output.exists(),
        "a rejected purge must not delete cache output"
    );
    assert!(
        state.jobs.list().is_empty(),
        "no purge worker may start in memory"
    );
}

#[tokio::test]
async fn post_delete_failure_carries_a_strict_partial_reconciliation() {
    let (_root, state) = state_with_project().await;
    let (project_dir, output) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let output = store.proxies_dir().join("a1.mp4");
        std::fs::write(&output, b"proxy").unwrap();
        (store.dir.clone(), output)
    };
    std::fs::File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        .unwrap();
    crate::cache_lifecycle::record_generated(
        &project_dir,
        CacheKind::Proxies,
        "a1",
        OsStr::new("a1.mp4"),
    )
    .unwrap();
    let preview = preview(&state).await.unwrap();
    assert_eq!(preview.result.as_ref().unwrap()["purgeable"]["files"], 1);
    let plan = state.cache_purge_plan.lock().await.clone().unwrap();
    match crate::cache_lifecycle::remove_owned_output(
        &project_dir,
        CacheKind::Proxies,
        "a1",
        "proxies/a1.mp4",
    )
    .unwrap()
    {
        crate::cache_lifecycle::OwnedRemoval::Retired => {}
        outcome => panic!("expected exact output retirement, got {outcome:?}"),
    }
    let job = state.jobs.create("cache_purge");
    fail_with_reconciliation(
        &state,
        &job.job_id,
        &plan,
        1,
        5,
        CutError::new(
            error_codes::IO,
            "cache cleanup stopped after a deletion",
            "fixture failure after exact retirement",
        ),
    )
    .await;
    let record = state.jobs.get(&job.job_id).unwrap();
    assert!(matches!(record.state, crate::jobs::JobState::Failed));
    let reconciliation = &record.result.unwrap()["reconciliation"];
    assert_eq!(reconciliation["removed"]["files"], 1);
    assert_eq!(reconciliation["after"]["files"], 0);
    assert_eq!(reconciliation["after_basis"], "strict_scan");
    assert_eq!(reconciliation["balanced"], true);
}

#[tokio::test]
async fn unavailable_post_delete_measurement_keeps_the_known_delta() {
    let state = AppState::new();
    let plan = CachePurgePlan {
        plan_id: "cache_plan_1".into(),
        project_dir: std::path::PathBuf::from("/unavailable/cache.cutproj"),
        project_revision: Some("op_000001".into()),
        created_ms: 1,
        roots: Vec::new(),
        snapshot: Vec::new(),
        targets: vec![FileIdentity {
            kind: CacheKind::Proxies,
            name: "a1.mp4".into(),
            bytes: 5,
            modified_ms: 1,
        }],
        before: CacheMeasurement { files: 1, bytes: 5 },
    };
    let job = state.jobs.create("cache_purge");
    fail_with_reconciliation(
        &state,
        &job.job_id,
        &plan,
        1,
        5,
        CutError::new(
            error_codes::IO,
            "cache cleanup stopped after a deletion",
            "fixture failure after exact retirement",
        ),
    )
    .await;
    let reconciliation = &state.jobs.get(&job.job_id).unwrap().result.unwrap()["reconciliation"];
    assert_eq!(reconciliation["after"]["files"], 0);
    assert_eq!(reconciliation["after_basis"], "exclusive_lease_delta");
    assert_eq!(reconciliation["ledger_recovery_required"], false);
}
