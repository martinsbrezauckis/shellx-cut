use super::*;
use crate::events::EventBus;
use crate::jobs::{JobManager, JobState};
use crate::state::AppState;
use serde_json::json;

async fn state_with_image_asset() -> (tempfile::TempDir, AppState, std::path::PathBuf, String) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name": "cache-rebuild", "dir": root.path().join("cache-rebuild.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "project create: {:?}", created.error);
    let source = root.path().join("source.png");
    std::fs::write(&source, b"source-v1").unwrap();
    let hash = cut_core::hash_file(&source).unwrap();
    {
        let mut project = state.project.write().await;
        project
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: source.to_string_lossy().into_owned(),
                    hash: hash.clone(),
                    probe: Some(json!({"kind": "image"})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
    }
    (root, state, source, hash)
}

#[test]
fn missing_and_stale_outputs_become_durable_pending_reservations() {
    let root = tempfile::tempdir().unwrap();
    let store =
        cut_core::ProjectStore::create(&root.path().join("cache.cutproj"), "cache", None).unwrap();
    let project_dir = store.dir;
    let source_v1 = "sha256:source-v1";
    let proxy = project_dir.join("proxies/a1.mp4");

    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Proxies, "a1", source_v1).unwrap(),
        RebuildOutputState::MissingOrStale
    );
    reserve_rebuild_output(&project_dir, CacheKind::Proxies, "a1", source_v1).unwrap();
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Proxies, "a1", source_v1).unwrap(),
        RebuildOutputState::Pending
    );

    std::fs::write(&proxy, b"completed proxy bytes").unwrap();
    complete_rebuild_output(&project_dir, CacheKind::Proxies, "a1", source_v1).unwrap();
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Proxies, "a1", source_v1).unwrap(),
        RebuildOutputState::ReadyVerified
    );
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Proxies, "a1", "sha256:source-v2").unwrap(),
        RebuildOutputState::MissingOrStale
    );

    reserve_rebuild_output(&project_dir, CacheKind::Proxies, "a1", "sha256:source-v2").unwrap();
    assert!(
        !proxy.exists(),
        "the prior output is retired only after its pending ownership record is durable"
    );
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Proxies, "a1", "sha256:source-v2").unwrap(),
        RebuildOutputState::Pending
    );
}

#[tokio::test]
async fn active_rebuild_request_deduplicates_without_rescanning_or_spawning() {
    let (_root, state, _source, _hash) = state_with_image_asset().await;
    *state.cache_rebuild_active.lock().await = Some(CacheRebuildActive {
        job_id: "job_777".into(),
        assets: vec!["a1".into()],
        scheduled_outputs: 1,
        counts: json!({
            "fresh_assets": 0,
            "source_changed": 0,
            "source_unavailable": 0,
            "unowned_outputs": 0,
            "legacy_outputs": 0,
            "unsupported_assets": 0,
        }),
    });

    let result = start_rebuild(&state, json!({}))
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(result["status"], "already_queued");
    assert_eq!(result["job_id"], "job_777");
    assert_eq!(result["scheduled_assets"], 1);
    assert_eq!(result["scheduled_outputs"], 1);
    assert_eq!(result["counts"]["fresh_assets"], 0);
}

#[tokio::test]
async fn source_revision_change_is_reported_without_reserving_or_starting_a_worker() {
    let (_root, state, source, _hash) = state_with_image_asset().await;
    std::fs::write(&source, b"source-v2").unwrap();

    let result = start_rebuild(&state, json!({"asset_ids": ["a1"]}))
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(result["status"], "not_needed");
    assert_eq!(result["counts"]["source_changed"], 1);
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    assert!(
        !project_dir
            .join(".shellx-cut-cache-ownership.json")
            .exists(),
        "a changed source never receives a cache reservation"
    );
}

#[test]
fn pending_reservation_survives_cancelled_restart_and_is_resumable() {
    let root = tempfile::tempdir().unwrap();
    let store =
        cut_core::ProjectStore::create(&root.path().join("cache.cutproj"), "cache", None).unwrap();
    let project_dir = store.dir;
    reserve_rebuild_output(
        &project_dir,
        CacheKind::Thumbnails,
        "a1",
        "sha256:source-v1",
    )
    .unwrap();

    {
        let jobs = JobManager::new(EventBus::new());
        jobs.attach_project(&project_dir).unwrap();
        let job = jobs.create("cache_rebuild");
        jobs.progress(&job.job_id, 0.25, Some("rebuilding cache".into()));
        assert_eq!(jobs.get(&job.job_id).unwrap().state, JobState::Running);
    }
    let recovered = JobManager::new(EventBus::new());
    recovered.attach_project(&project_dir).unwrap();
    assert_eq!(recovered.get("job_001").unwrap().state, JobState::Failed);
    assert_eq!(
        rebuild_output_state(
            &project_dir,
            CacheKind::Thumbnails,
            "a1",
            "sha256:source-v1"
        )
        .unwrap(),
        RebuildOutputState::Pending
    );
    reserve_rebuild_output(
        &project_dir,
        CacheKind::Thumbnails,
        "a1",
        "sha256:source-v1",
    )
    .unwrap();
}

#[tokio::test]
async fn rebuild_selection_is_bounded_before_any_project_scan() {
    let state = AppState::new();
    let asset_ids = (1..=CACHE_REBUILD_ASSET_LIMIT + 1)
        .map(|number| format!("a{number}"))
        .collect::<Vec<_>>();
    let error = start_rebuild(&state, json!({"asset_ids": asset_ids}))
        .await
        .expect_err("an oversized request must be refused before opening a project");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
}
