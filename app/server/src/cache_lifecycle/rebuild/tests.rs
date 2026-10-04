use super::super::ownership::complete_rebuild_output;
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

#[tokio::test]
async fn second_reservation_failure_retains_first_retirement_and_durable_partial_job() {
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
    let source = root.path().join("source.mp4");
    let source_bytes = b"original imported video bytes";
    std::fs::write(&source, source_bytes).unwrap();
    let hash = cut_core::hash_file(&source).unwrap();
    let project_dir = {
        let mut project = state.project.write().await;
        let store = project.as_mut().unwrap();
        store
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: source.to_string_lossy().into_owned(),
                    hash: hash.clone(),
                    probe: Some(json!({"kind": "video", "duration_ms": 1000})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
        store.dir.clone()
    };
    let old_hash = "sha256:previous-source";
    let proxy = project_dir.join("proxies/a1.mp4");
    let filmstrip = project_dir.join("filmstrip/a1.jpg");
    for (kind, path, bytes) in [
        (CacheKind::Proxies, &proxy, b"old proxy".as_slice()),
        (
            CacheKind::Thumbnails,
            &filmstrip,
            b"old filmstrip".as_slice(),
        ),
    ] {
        reserve_rebuild_output(&project_dir, kind, "a1", old_hash).unwrap();
        std::fs::write(path, bytes).unwrap();
        complete_rebuild_output(&project_dir, kind, "a1", old_hash).unwrap();
    }
    let before_filmstrip = std::fs::read(&filmstrip).unwrap();
    let before_filmstrip_entry = {
        let ledger = super::super::ownership::read_ledger(&project_dir).unwrap();
        ledger.entries.get("filmstrip/a1.jpg").cloned().unwrap()
    };
    let error = start_rebuild_with_reserver(
        &state,
        json!({"asset_ids": ["a1"]}),
        |dir, kind, asset, hash| {
            if kind == CacheKind::Thumbnails {
                return Err(CutError::new(
                    error_codes::IO,
                    "filmstrip reservation fault",
                    "test-only second reservation failure",
                ));
            }
            reserve_rebuild_output(dir, kind, asset, hash)
        },
    )
    .await
    .expect_err("the second reservation must fail after the proxy is retired");
    assert_eq!(error.code, error_codes::IO);
    assert!(error.message.contains("check Jobs"));
    assert_eq!(error.cause, "test-only second reservation failure");
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(cut_core::hash_file(&source).unwrap(), hash);
    assert!(!proxy.exists());
    assert_eq!(std::fs::read(&filmstrip).unwrap(), before_filmstrip);
    let ledger = super::super::ownership::read_ledger(&project_dir).unwrap();
    let proxy_entry = ledger.entries.get("proxies/a1.mp4").unwrap();
    assert_eq!(proxy_entry.state, "pending");
    assert!(proxy_entry.output_retired);
    assert_eq!(proxy_entry.source_hash.as_deref(), Some(hash.as_str()));
    let filmstrip_entry = ledger.entries.get("filmstrip/a1.jpg").unwrap();
    assert_eq!(
        serde_json::to_value(filmstrip_entry).unwrap(),
        serde_json::to_value(before_filmstrip_entry).unwrap()
    );
    assert!(state.cache_rebuild_active.lock().await.is_none());
    let jobs = state.jobs.list();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.state, JobState::Failed);
    assert!(!state.jobs.has_active_task_for_tests(&job.job_id));
    assert_eq!(job.error.as_ref().unwrap().code, error_codes::IO);
    assert_eq!(
        job.error.as_ref().unwrap().cause,
        "test-only second reservation failure"
    );
    assert_eq!(
        job.result.as_ref().unwrap()["ownership_verification"],
        "verified"
    );
    assert_eq!(job.result.as_ref().unwrap()["counts"]["pending_retired"], 1);
    assert_eq!(
        job.result.as_ref().unwrap()["counts"]["pending_unretired"],
        0
    );
    assert_eq!(
        job.result.as_ref().unwrap()["pending_outputs"][0]["asset_id"],
        "a1"
    );
    assert_eq!(
        job.result.as_ref().unwrap()["pending_outputs"][0]["kind"],
        "proxies"
    );
    assert!(job.persistence_error.is_none());

    let reopened = JobManager::new(EventBus::new());
    reopened.attach_project(&project_dir).unwrap();
    let recovered = reopened.get(&job.job_id).unwrap();
    assert_eq!(recovered.state, JobState::Failed);
    assert_eq!(recovered.result, job.result);
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Proxies, "a1", &hash).unwrap(),
        RebuildOutputState::Pending
    );
    let estimate = start_rebuild(&state, json!({"asset_ids": ["a1"], "estimate_only": true}))
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(estimate["status"], "estimated");
    assert_eq!(estimate["scheduled_assets"], 1);
    assert_eq!(estimate["scheduled_outputs"], 2);
    assert_eq!(state.jobs.list().len(), 1);
    assert!(state.cache_rebuild_active.lock().await.is_none());
    assert!(!state.jobs.has_active_task_for_tests(&job.job_id));
    reserve_rebuild_output(&project_dir, CacheKind::Proxies, "a1", &hash).unwrap();
    assert!(
        !proxy.exists(),
        "the exact pending retry must not recreate or adopt the retired proxy"
    );
    reserve_rebuild_output(&project_dir, CacheKind::Thumbnails, "a1", &hash).unwrap();
    assert!(!filmstrip.exists());
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Thumbnails, "a1", &hash).unwrap(),
        RebuildOutputState::Pending
    );
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
        estimate: json!({
            "basis": "source_hash_match_and_current_import_metadata",
            "assets": 1,
            "verified_source_bytes": 9,
            "proxy_outputs": 0,
            "filmstrip_outputs": 1,
            "proxy_duration_ms": 0,
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
    assert_eq!(result["estimate"]["verified_source_bytes"], 9);
}

#[tokio::test]
async fn estimate_only_verifies_rebuild_work_without_reserving_or_starting_a_job() {
    let (_root, state, _source, _hash) = state_with_image_asset().await;
    let result = start_rebuild(&state, json!({"asset_ids": ["a1"], "estimate_only": true}))
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(result["status"], "estimated");
    assert_eq!(result["scheduled_assets"], 1);
    assert_eq!(result["scheduled_outputs"], 1);
    assert_eq!(result["estimate"]["assets"], 1);
    assert_eq!(result["estimate"]["verified_source_bytes"], 9);
    assert_eq!(result["estimate"]["proxy_outputs"], 0);
    assert_eq!(result["estimate"]["filmstrip_outputs"], 1);
    assert!(
        state.jobs.list().is_empty(),
        "an estimate must not create a background worker"
    );
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    assert!(
        !project_dir
            .join(".shellx-cut-cache-ownership.json")
            .exists(),
        "an estimate must not reserve a cache output"
    );
}

#[tokio::test]
async fn rebuild_refuses_non_durable_admission_before_reserving_cache_output() {
    let (_root, state, _source, _hash) = state_with_image_asset().await;
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    let jobs_dir = project_dir.join("jobs");
    std::fs::remove_dir_all(&jobs_dir).unwrap();
    std::fs::write(&jobs_dir, b"not a directory").unwrap();

    let error = start_rebuild(&state, json!({"asset_ids": ["a1"]}))
        .await
        .expect_err("rebuild must refuse non-durable job admission");
    assert_eq!(error.code, error_codes::IO);
    assert!(
        state.jobs.list().is_empty(),
        "no rebuild worker may start in memory"
    );
    assert!(
        !project_dir
            .join(".shellx-cut-cache-ownership.json")
            .exists(),
        "failed admission must not reserve or retire cache output"
    );
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

#[tokio::test]
async fn changed_or_unavailable_source_with_a_pending_reservation_stays_recoverable() {
    let (_root, state, source, hash) = state_with_image_asset().await;
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    reserve_rebuild_output(&project_dir, CacheKind::Thumbnails, "a1", &hash).unwrap();
    std::fs::write(&source, b"source-v2").unwrap();
    let changed = start_rebuild(&state, json!({"asset_ids": ["a1"]}))
        .await
        .expect_err("a changed source cannot silently discard a pending output");
    assert_eq!(changed.code, error_codes::CONFLICT);
    assert!(changed.cause.contains("source is changed"));
    std::fs::remove_file(source).unwrap();

    let error = start_rebuild(&state, json!({"asset_ids": ["a1"]}))
        .await
        .expect_err("an unavailable source cannot silently discard a pending output");
    assert_eq!(error.code, error_codes::CONFLICT);
    assert_eq!(
        error.message,
        "cache rebuild requires pending source recovery"
    );
    assert!(
        error
            .cause
            .contains("restore the exact source, relink it, or remove the asset"),
        "the precise repair paths stay public: {}",
        error.cause
    );
    assert_eq!(
        rebuild_output_state(&project_dir, CacheKind::Thumbnails, "a1", &hash).unwrap(),
        RebuildOutputState::Pending,
        "the durable reservation remains visible until an exact recovery action"
    );
}

#[tokio::test]
async fn removed_asset_retires_its_pending_reservation_through_media_remove() {
    let (_root, state, _source, hash) = state_with_image_asset().await;
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    reserve_rebuild_output(&project_dir, CacheKind::Thumbnails, "a1", &hash).unwrap();

    let removed = crate::dispatch::dispatch(
        &state,
        "media.remove",
        json!({"asset": "a1"}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(removed.ok, "remove: {:?}", removed.error);
    assert!(
        crate::cache_lifecycle::pending_rebuild_outputs_for_asset(&project_dir, "a1")
            .unwrap()
            .is_empty(),
        "asset removal retires an exact pending ledger entry even without a metadata pointer"
    );
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(preview.ok, "retired reservation: {:?}", preview.error);
}

#[tokio::test]
async fn stale_removed_asset_reservation_is_repaired_before_an_empty_rebuild() {
    let (_root, state, _source, hash) = state_with_image_asset().await;
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    reserve_rebuild_output(&project_dir, CacheKind::Thumbnails, "a1", &hash).unwrap();
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().unwrap();
        store
            .record_remove_asset("a1", cut_core::Actor::system(), None)
            .unwrap();
    }

    let result = start_rebuild(&state, json!({}))
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(result["status"], "not_needed");
    assert!(
        crate::cache_lifecycle::pending_rebuild_outputs_for_asset(&project_dir, "a1")
            .unwrap()
            .is_empty(),
        "a restart-era orphan is retired under the lifecycle lease rather than blocking every preview"
    );
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(preview.ok, "orphan reconciliation: {:?}", preview.error);
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
