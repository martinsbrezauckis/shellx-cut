use super::*;
use crate::state::AppState;
use base64::Engine;
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

fn write_test_png(path: &Path) {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn owned_output_removal_refuses_missing_mismatched_and_malformed_ledger_entries() {
    let root = tempfile::tempdir().unwrap();
    let store =
        cut_core::ProjectStore::create(&root.path().join("cache.cutproj"), "cache", None).unwrap();
    let project_dir = store.dir.clone();
    let proxy = project_dir.join("proxies/a1.mp4");
    std::fs::write(&proxy, b"proxy").unwrap();

    let missing = remove_owned_output(&project_dir, CacheKind::Proxies, "a1", "proxies/a1.mp4")
        .expect_err("a cache file without a ledger entry must be kept");
    assert_eq!(missing.code, cut_core::error_codes::CONFLICT);
    assert!(proxy.exists());

    std::fs::write(
        project_dir.join(LEDGER_NAME),
        serde_json::to_vec(&json!({
            "schema": LEDGER_SCHEMA,
            "entries": {
                "proxies/a1.mp4": {"kind": "proxies", "asset": "a2", "name": "a1.mp4"}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let mismatch = remove_owned_output(&project_dir, CacheKind::Proxies, "a1", "proxies/a1.mp4")
        .expect_err("an entry owned by another asset must be kept");
    assert_eq!(mismatch.code, cut_core::error_codes::CONFLICT);
    assert!(proxy.exists());

    std::fs::write(project_dir.join(LEDGER_NAME), b"not-json").unwrap();
    let malformed = remove_owned_output(&project_dir, CacheKind::Proxies, "a1", "proxies/a1.mp4")
        .expect_err("a malformed ledger must block cache deletion");
    assert_eq!(malformed.code, cut_core::error_codes::CONFLICT);
    assert!(proxy.exists());
}

#[test]
fn record_generated_refuses_a_filename_owned_by_a_different_asset() {
    let root = tempfile::tempdir().unwrap();
    let store =
        cut_core::ProjectStore::create(&root.path().join("cache.cutproj"), "cache", None).unwrap();
    let project_dir = store.dir.clone();
    let proxy = project_dir.join("proxies/a2.mp4");
    std::fs::write(&proxy, b"proxy").unwrap();

    let error = record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a2.mp4"))
        .expect_err("a1 must not be allowed to claim a2's proxy filename");
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(proxy.exists(), "recording must not alter the cache file");
    assert!(
        !project_dir.join(LEDGER_NAME).exists(),
        "the rejected ownership record must not be published"
    );
}

#[cfg(unix)]
#[test]
fn ledger_write_failure_after_unlink_leaves_the_pending_entry_visible() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let store =
        cut_core::ProjectStore::create(&root.path().join("cache.cutproj"), "cache", None).unwrap();
    let project_dir = store.dir.clone();
    let proxy = project_dir.join("proxies/a1.mp4");
    std::fs::write(&proxy, b"proxy").unwrap();
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();

    let original = std::fs::metadata(&project_dir).unwrap().permissions();
    std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = remove_owned_output(&project_dir, CacheKind::Proxies, "a1", "proxies/a1.mp4");
    std::fs::set_permissions(&project_dir, original).unwrap();

    match outcome.unwrap() {
        OwnedRemoval::UnlinkedLedgerPending(error) => {
            assert_eq!(error.code, cut_core::error_codes::CONFLICT);
        }
        OwnedRemoval::Retired => panic!("the protected project root must reject the ledger write"),
    }
    assert!(
        !proxy.exists(),
        "the outcome must truthfully retain that the file was already unlinked"
    );
    assert!(
        project_dir.join(LEDGER_NAME).exists(),
        "the stale entry remains inspectable instead of being hidden"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn media_remove_reports_ledger_publish_failure_after_its_project_op_commits() {
    use std::os::unix::fs::PermissionsExt;

    let (_root, state) = state_with_project().await;
    let (project_dir, proxy) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let proxy = store.proxies_dir().join("a1.mp4");
        std::fs::write(&proxy, b"proxy").unwrap();
        (store.dir.clone(), proxy)
    };
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();
    {
        let mut project = state.project.write().await;
        project
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: "/outside/source.mov".into(),
                    hash: "sha256:old".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: Some("proxies/a1.mp4".into()),
                    filmstrip: None,
                },
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
    }

    // The exclusive lease holds only cache cleanup. The remove task can commit
    // its project op, then blocks before it starts exclusive cleanup.
    let cleanup_blocker = state.cache_lifecycle_lease.write().await;
    let worker_state = state.clone();
    let task = tokio::spawn(async move {
        crate::dispatch::dispatch(
            &worker_state,
            "media.remove",
            json!({"asset": "a1"}),
            cut_core::Actor::system(),
        )
        .await
    });
    let mut committed = false;
    for _ in 0..100 {
        {
            let project = state.project.read().await;
            committed = project
                .as_ref()
                .is_some_and(|store| !store.project.assets.contains_key("a1"));
        }
        if committed {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        committed,
        "media.remove must commit before cache cleanup waits"
    );

    let original = std::fs::metadata(&project_dir).unwrap().permissions();
    std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    drop(cleanup_blocker);
    let removed = task.await.unwrap();
    std::fs::set_permissions(&project_dir, original).unwrap();

    assert!(
        removed.ok,
        "the committed removal is not rolled back by ledger publication: {:?}",
        removed.error
    );
    assert!(!proxy.exists(), "the exact cache output was removed");
    let warning = removed
        .warnings
        .as_ref()
        .and_then(|warnings| {
            warnings
                .iter()
                .find(|warning| warning.code == "cache_ownership_cleanup_pending")
        })
        .expect("ledger publication failure must be an in-band warning");
    assert!(warning.message.contains("proxy cache for asset 'a1'"));
    assert!(
        !warning.message.contains("proxies/a1.mp4"),
        "warnings must never disclose the internal cache path"
    );
    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(
        !preview.ok,
        "the stale ledger remains visible to cache inventory rather than being hidden"
    );
}

#[tokio::test]
async fn media_remove_waits_for_an_active_cache_producer_before_retiring_ledger_state() {
    let (_root, state) = state_with_project().await;
    let (project_dir, proxy) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let proxy = store.proxies_dir().join("a1.mp4");
        std::fs::write(&proxy, b"proxy").unwrap();
        (store.dir.clone(), proxy)
    };
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();
    {
        let mut project = state.project.write().await;
        project
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: "/outside/source.mov".into(),
                    hash: "sha256:old".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: Some("proxies/a1.mp4".into()),
                    filmstrip: None,
                },
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
    }

    let producer = state.cache_lifecycle_lease.read().await;
    let worker_state = state.clone();
    let task = tokio::spawn(async move {
        crate::dispatch::dispatch(
            &worker_state,
            "media.remove",
            json!({"asset": "a1"}),
            cut_core::Actor::system(),
        )
        .await
    });
    let mut committed = false;
    for _ in 0..100 {
        {
            let project = state.project.read().await;
            committed = project
                .as_ref()
                .is_some_and(|store| !store.project.assets.contains_key("a1"));
        }
        if committed {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(committed, "the project op must not wait for cache cleanup");
    assert!(
        !task.is_finished(),
        "exclusive cache cleanup must wait for the active shared producer lease"
    );
    drop(producer);
    let removed = task.await.unwrap();
    assert!(removed.ok, "remove: {:?}", removed.error);
    assert!(!proxy.exists());
}

#[tokio::test]
async fn media_remove_retires_only_its_exact_cache_outputs_and_keeps_preview_valid() {
    let (_root, state) = state_with_project().await;
    let (project_dir, proxy_a1, filmstrip_a1, proxy_a2) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let proxy_a1 = store.proxies_dir().join("a1.mp4");
        let filmstrip_a1 = store.dir.join("filmstrip/a1.jpg");
        let proxy_a2 = store.proxies_dir().join("a2.mp4");
        std::fs::write(&proxy_a1, b"a1 proxy").unwrap();
        std::fs::write(&filmstrip_a1, b"a1 filmstrip").unwrap();
        std::fs::write(&proxy_a2, b"a2 proxy").unwrap();
        (store.dir.clone(), proxy_a1, filmstrip_a1, proxy_a2)
    };
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();
    record_generated(
        &project_dir,
        CacheKind::Thumbnails,
        "a1",
        OsStr::new("a1.jpg"),
    )
    .unwrap();
    record_generated(&project_dir, CacheKind::Proxies, "a2", OsStr::new("a2.mp4")).unwrap();
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().unwrap();
        store
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: "/outside/source.mov".into(),
                    hash: "sha256:old".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: Some("proxies/a1.mp4".into()),
                    filmstrip: Some("filmstrip/a1.jpg".into()),
                },
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
    }

    let removed = crate::dispatch::dispatch(
        &state,
        "media.remove",
        json!({"asset": "a1"}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(removed.ok, "remove: {:?}", removed.error);
    let freed = removed.result.as_ref().unwrap()["freed"]
        .as_array()
        .unwrap();
    assert_eq!(freed.len(), 2);
    assert!(freed.iter().any(|item| item == "proxies/a1.mp4"));
    assert!(freed.iter().any(|item| item == "filmstrip/a1.jpg"));
    assert!(!proxy_a1.exists() && !filmstrip_a1.exists());
    assert!(
        proxy_a2.exists(),
        "a different asset's cache file is untouched"
    );

    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(
        preview.ok,
        "retiring the removed entries keeps cache inventory valid: {:?}",
        preview.error
    );
}

#[tokio::test]
async fn changed_hash_relink_retires_cache_entries_without_touching_other_outputs() {
    let (root, state) = state_with_project().await;
    let replacement = root.path().join("replacement.png");
    write_test_png(&replacement);
    let (project_dir, proxy_a1, filmstrip_a1, proxy_a2, transcript, perception) = {
        let project = state.project.read().await;
        let store = project.as_ref().unwrap();
        let proxy_a1 = store.proxies_dir().join("a1.mp4");
        let filmstrip_a1 = store.dir.join("filmstrip/a1.jpg");
        let proxy_a2 = store.proxies_dir().join("a2.mp4");
        let transcript = store.dir.join("receipts/a1.words.json");
        let perception = store.dir.join("receipts/a1.perception.json");
        std::fs::write(&proxy_a1, b"a1 proxy").unwrap();
        std::fs::write(&filmstrip_a1, b"a1 filmstrip").unwrap();
        std::fs::write(&proxy_a2, b"a2 proxy").unwrap();
        std::fs::write(&transcript, b"words").unwrap();
        std::fs::write(&perception, b"perception").unwrap();
        (
            store.dir.clone(),
            proxy_a1,
            filmstrip_a1,
            proxy_a2,
            transcript,
            perception,
        )
    };
    record_generated(&project_dir, CacheKind::Proxies, "a1", OsStr::new("a1.mp4")).unwrap();
    record_generated(
        &project_dir,
        CacheKind::Thumbnails,
        "a1",
        OsStr::new("a1.jpg"),
    )
    .unwrap();
    record_generated(&project_dir, CacheKind::Proxies, "a2", OsStr::new("a2.mp4")).unwrap();
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().unwrap();
        store
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: "/offline/original.png".into(),
                    hash: "sha256:old".into(),
                    probe: Some(json!({"kind": "image"})),
                    transcript: Some("receipts/a1.words.json".into()),
                    perception: Some("receipts/a1.perception.json".into()),
                    proxy: Some("proxies/a1.mp4".into()),
                    filmstrip: Some("filmstrip/a1.jpg".into()),
                },
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
    }

    let relinked = crate::dispatch::dispatch(
        &state,
        "media.relink",
        json!({"asset": "a1", "path": replacement}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(relinked.ok, "relink: {:?}", relinked.error);
    assert_eq!(relinked.result.as_ref().unwrap()["hash_changed"], true);
    assert!(!proxy_a1.exists() && !filmstrip_a1.exists());
    assert!(!transcript.exists() && !perception.exists());
    assert!(proxy_a2.exists(), "unrelated cache output is untouched");

    let preview = crate::dispatch::dispatch(
        &state,
        "project.cache_preview",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(
        preview.ok,
        "changed-hash relink must not leave a stale ownership ledger: {:?}",
        preview.error
    );
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
