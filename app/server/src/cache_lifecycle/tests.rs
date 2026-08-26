use super::*;
use crate::state::AppState;
use serde_json::json;
use std::ffi::OsStr;
use std::path::Path;

async fn state_with_project() -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"cache", "dir": root.path().join("cache.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "project create: {:?}", created.error);
    (root, state)
}

fn age(path: &Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        .unwrap();
}

#[tokio::test]
async fn preview_and_confirmed_job_only_remove_aged_owned_unreferenced_cache() {
    let (_root, state) = state_with_project().await;
    let (project_dir, path, source, export, capture, receipt) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let path = store.proxies_dir().join("a1.mp4");
        let source = store.dir.join("source.mp4");
        let export = store.dir.join("exports/final.mp4");
        let capture = store.dir.join("captures/cap-001/source.mp4");
        let receipt = store.dir.join("receipts/a1.words.json");
        std::fs::write(&path, b"proxy").unwrap();
        std::fs::create_dir_all(export.parent().unwrap()).unwrap();
        std::fs::create_dir_all(capture.parent().unwrap()).unwrap();
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&export, b"export").unwrap();
        std::fs::write(&capture, b"capture").unwrap();
        std::fs::write(&receipt, b"receipt").unwrap();
        (store.dir.clone(), path, source, export, capture, receipt)
    };
    age(&path);
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(preview.ok, "preview: {:?}", preview.error);
    let preview = preview.result.unwrap();
    assert_eq!(preview["status"], "ready");
    assert_eq!(preview["purgeable"]["files"], 1);
    let started = crate::dispatch::dispatch(
        &state,
        "project.cache_purge",
        json!({"plan_id": preview["plan_id"], "confirm": true}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(started.ok, "start: {:?}", started.error);
    let job_id = started.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    for _ in 0..50 {
        if state.jobs.get(&job_id).is_some_and(|job| {
            matches!(
                job.state,
                crate::jobs::JobState::Done | crate::jobs::JobState::Failed
            )
        }) {
            break;
        }
        tokio::task::yield_now().await;
    }
    let job = state.jobs.get(&job_id).unwrap();
    assert!(
        matches!(job.state, crate::jobs::JobState::Done),
        "job: {job:?}"
    );
    assert!(!path.exists());
    assert!(source.exists());
    assert!(export.exists());
    assert!(capture.exists());
    assert!(receipt.exists());
}

#[tokio::test]
async fn legacy_or_foreign_or_symlink_entries_block_without_a_plan() {
    let (_root, state) = state_with_project().await;
    let project_dir = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let foreign = store.proxies_dir().join("a9.mp4");
        std::fs::write(foreign, b"legacy").unwrap();
        store.dir.clone()
    };
    let legacy = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!legacy.ok);
    assert!(state.cache_purge_plan.lock().await.is_none());
    std::fs::remove_file(project_dir.join("proxies/a9.mp4")).unwrap();
    std::fs::create_dir(project_dir.join("proxies/partial")).unwrap();
    let partial = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!partial.ok);
    std::fs::remove_dir(project_dir.join("proxies/partial")).unwrap();
    #[cfg(unix)]
    {
        let target = project_dir.join("outside.mp4");
        std::fs::write(&target, b"outside").unwrap();
        std::os::unix::fs::symlink(&target, project_dir.join("proxies/a9.mp4")).unwrap();
        let link = crate::dispatch::dispatch(
            &state,
            "project.cache_preview",
            json!({}),
            cut_core::Actor::system(),
        )
        .await;
        assert!(!link.ok);
    }
}

#[tokio::test]
async fn changed_file_invalidates_the_confirmed_plan_before_removal() {
    let (_root, state) = state_with_project().await;
    let (project_dir, path) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let path = store.proxies_dir().join("a1.mp4");
        std::fs::write(&path, b"proxy").unwrap();
        (store.dir.clone(), path)
    };
    age(&path);
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    std::fs::write(&path, b"changed").unwrap();
    let started = crate::dispatch::dispatch(
        &state,
        "project.cache_purge",
        json!({"plan_id": preview["plan_id"], "confirm": true}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    let job_id = started["job_id"].as_str().unwrap().to_string();
    for _ in 0..50 {
        if state.jobs.get(&job_id).is_some_and(|job| {
            matches!(
                job.state,
                crate::jobs::JobState::Done | crate::jobs::JobState::Failed
            )
        }) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        state.jobs.get(&job_id).unwrap().state,
        crate::jobs::JobState::Failed
    ));
    assert!(path.exists());
}

#[tokio::test]
async fn preview_refuses_to_race_a_cooperating_cache_producer() {
    let (_root, state) = state_with_project().await;
    let _producer = state.cache_lifecycle_lease.read().await;
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!preview.ok);
    assert!(state.cache_purge_plan.lock().await.is_none());
}

#[tokio::test]
async fn purge_requires_literal_confirmation_before_it_can_consume_a_plan() {
    let (_root, state) = state_with_project().await;
    let rejected = crate::dispatch::dispatch(
        &state,
        "project.cache_purge",
        json!({"plan_id": "cache_plan_1", "confirm": false}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!rejected.ok);
    assert_eq!(
        rejected.error.unwrap().code,
        cut_core::error_codes::INVALID_ARGS
    );
    assert!(state.cache_purge_plan.lock().await.is_none());
}
