use super::*;
use crate::jobs::{JobRecord, JobState};
use crate::state::AppState;
use serde_json::{json, Value};

async fn project() -> (tempfile::TempDir, AppState, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let response = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"previews", "dir":root.path().join("previews.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(response.ok, "{response:?}");
    let dir = state.project.read().await.as_ref().unwrap().dir.clone();
    (root, state, dir)
}

fn fixture(path: &Path, color: &str, video: bool) {
    let mut command = std::process::Command::new("ffmpeg");
    command
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c={color}:s=160x90:r=12"));
    if video {
        command.args([
            "-t", "1", "-c:v", "libx264", "-threads", "1", "-pix_fmt", "yuv420p",
        ]);
    } else {
        command.args(["-frames:v", "1", "-threads", "1"]);
    }
    assert!(command.arg(path).status().unwrap().success());
}

async fn dispatch(state: &AppState, verb: &str, args: Value) -> Value {
    let response = crate::dispatch::dispatch(state, verb, args, cut_core::Actor::system()).await;
    assert!(response.ok, "{verb}: {response:?}");
    response.result.unwrap()
}

async fn finished(state: &AppState, id: &str) -> JobRecord {
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            let job = state.jobs.get(id).unwrap();
            if matches!(job.state, JobState::Done | JobState::Failed) {
                break job;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("preview job must finish")
}

async fn import(state: &AppState, path: &Path) -> (String, JobRecord) {
    let imported = dispatch(state, "media.import", json!({"path":path})).await;
    let id = imported["asset_id"].as_str().unwrap().to_owned();
    let job = finished(state, imported["job_id"].as_str().unwrap()).await;
    (id, job)
}

fn assert_bound(dir: &Path, kind: CacheKind, asset: &str, hash: &str) {
    assert_eq!(
        rebuild_output_state(dir, kind, asset, hash).unwrap(),
        RebuildOutputState::ReadyVerified,
        "new output must carry its actual source identity"
    );
}

#[tokio::test]
async fn fresh_image_import_and_manual_thumbnail_have_verified_source_identity() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let hash = cut_core::hash_file(&source).unwrap();
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    assert_bound(&dir, CacheKind::Thumbnails, &id, &hash);
    let current = dispatch(
        &state,
        "project.cache_rebuild",
        json!({"asset_ids":[id],"estimate_only":true}),
    )
    .await;
    assert_eq!(current["scheduled_assets"], 0, "{current}");
    assert_eq!(current["skipped_assets"][0]["asset_id"], id);
    assert_eq!(current["skipped_assets"][0]["reason"], "already_current");
    std::fs::remove_file(dir.join(format!("filmstrip/{id}.jpg"))).unwrap();
    dispatch(&state, "media.filmstrip", json!({"asset":id})).await;
    assert_bound(&dir, CacheKind::Thumbnails, &id, &hash);
    std::fs::remove_file(dir.join(format!("filmstrip/{id}.jpg"))).unwrap();
    let queued = dispatch(&state, "project.cache_rebuild", json!({"asset_ids":[id]})).await;
    assert_eq!(queued["scheduled_outputs"], 1, "{queued}");
    let rebuilt = finished(&state, queued["job_id"].as_str().unwrap()).await;
    assert_eq!(rebuilt.state, JobState::Done, "{rebuilt:?}");
    assert_bound(&dir, CacheKind::Thumbnails, &id, &hash);
}

#[tokio::test]
async fn fresh_video_proxy_and_both_filmstrip_paths_have_verified_source_identity() {
    let (root, state, dir) = project().await;
    let source = root.path().join("video.mp4");
    fixture(&source, "red", true);
    let hash = cut_core::hash_file(&source).unwrap();
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let proxy = finished(
        &state,
        job.result.as_ref().unwrap()["proxy_job"].as_str().unwrap(),
    )
    .await;
    assert_eq!(proxy.state, JobState::Done, "{proxy:?}");
    assert_eq!(proxy.result.as_ref().unwrap()["proxy"], true, "{proxy:?}");
    assert_bound(&dir, CacheKind::Proxies, &id, &hash);
    assert_bound(&dir, CacheKind::Thumbnails, &id, &hash);
    std::fs::remove_file(dir.join(format!("filmstrip/{id}.jpg"))).unwrap();
    dispatch(&state, "media.filmstrip", json!({"asset":id})).await;
    assert_bound(&dir, CacheKind::Thumbnails, &id, &hash);
    let window = dispatch(
        &state,
        "media.filmstrip",
        json!({"asset":id,"range_ms":[0,500],"count":12,"h":80}),
    )
    .await;
    let ledger = read_ledger(&dir).unwrap();
    assert_eq!(
        ledger.entries[window["thumbs"].as_str().unwrap()]
            .source_hash
            .as_deref(),
        Some(hash.as_str())
    );
    // Young files remain protected by purge; rebuilding missing outputs needs no purge.
    for relative in [format!("proxies/{id}.mp4"), format!("filmstrip/{id}.jpg")] {
        std::fs::remove_file(dir.join(relative)).unwrap();
    }
    let queued = dispatch(&state, "project.cache_rebuild", json!({"asset_ids":[id]})).await;
    assert_eq!(queued["scheduled_outputs"], 2, "{queued}");
    let rebuilt = finished(&state, queued["job_id"].as_str().unwrap()).await;
    assert_eq!(rebuilt.state, JobState::Done, "{rebuilt:?}");
    assert_bound(&dir, CacheKind::Proxies, &id, &hash);
    assert_bound(&dir, CacheKind::Thumbnails, &id, &hash);
}

#[tokio::test]
async fn failed_import_ledger_write_publishes_neither_ready_identity_nor_asset_pointer() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    std::fs::create_dir(dir.join(LEDGER_NAME)).unwrap();
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Failed, "{job:?}");
    assert!(job.error.as_ref().unwrap().message.contains("ownership"));
    assert!(
        state.project.read().await.as_ref().unwrap().project.assets[&id]
            .filmstrip
            .is_none()
    );
    assert!(dir.join(LEDGER_NAME).is_dir());
    assert!(read_ledger(&dir).is_err());
}

#[tokio::test]
async fn forged_ready_ledger_never_promotes_planted_output_to_engine_origin() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let hash = cut_core::hash_file(&source).unwrap();
    let path = dir.join(format!("filmstrip/{id}.jpg"));
    std::fs::write(&path, b"planted output").unwrap();
    let mut ledger = read_ledger(&dir).unwrap();
    ledger
        .entries
        .get_mut(&format!("filmstrip/{id}.jpg"))
        .unwrap()
        .source_hash = Some(hash);
    write_ledger(&dir, &ledger).unwrap();
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .project
        .assets
        .get_mut(&id)
        .unwrap()
        .filmstrip = None;
    let result = dispatch(
        &state,
        "project.cache_rebuild",
        json!({"asset_ids":[id],"estimate_only":true}),
    )
    .await;
    assert_eq!(
        result["scheduled_assets"], 0,
        "forged ledger must not admit planted bytes: {result}"
    );
    assert_eq!(result["counts"]["unowned_outputs"], 1, "{result}");
    assert_eq!(std::fs::read(&path).unwrap(), b"planted output");
    assert!(!cut_media::cache_trust::trusted(&path));
}

#[tokio::test]
async fn changed_source_is_named_then_stale_preview_rebuilds_after_identity_refresh() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let thumbnail = dir.join(format!("filmstrip/{id}.jpg"));
    let old_bytes = std::fs::read(&thumbnail).unwrap();
    fixture(&source, "green", false);
    let new_hash = cut_core::hash_file(&source).unwrap();

    let refused = dispatch(&state, "project.cache_rebuild", json!({"asset_ids":[id]})).await;
    assert_eq!(refused["scheduled_assets"], 0, "{refused}");
    assert_eq!(refused["counts"]["source_changed"], 1, "{refused}");
    assert_eq!(refused["skipped_assets"][0]["asset_id"], id);
    assert_eq!(refused["skipped_assets"][0]["reason"], "source_changed");
    assert_eq!(std::fs::read(&thumbnail).unwrap(), old_bytes);

    // Model an explicit source identity refresh while retaining the stale cache.
    // The rebuild must compare its recorded source, even if a pointer is present.
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .record_relink_asset(
            &id,
            &source.to_string_lossy(),
            &new_hash,
            false,
            cut_core::Actor::system(),
            None,
        )
        .unwrap();
    let queued = dispatch(&state, "project.cache_rebuild", json!({"asset_ids":[id]})).await;
    assert_eq!(queued["scheduled_assets"], 1, "{queued}");
    assert_eq!(queued["scheduled_outputs"], 1, "{queued}");
    let rebuilt = finished(&state, queued["job_id"].as_str().unwrap()).await;
    assert_eq!(rebuilt.state, JobState::Done, "{rebuilt:?}");
    assert_ne!(std::fs::read(&thumbnail).unwrap(), old_bytes);
    assert_bound(&dir, CacheKind::Thumbnails, &id, &new_hash);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_atomic_ledger_write_leaves_previous_record_and_pointer_unchanged() {
    use std::os::unix::fs::PermissionsExt;
    struct RestorePermissions(PathBuf, std::fs::Permissions);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.0, self.1.clone()).unwrap();
        }
    }
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let mut ledger = read_ledger(&dir).unwrap();
    ledger
        .entries
        .get_mut(&format!("filmstrip/{id}.jpg"))
        .unwrap()
        .source_hash = None;
    write_ledger(&dir, &ledger).unwrap();
    let old_ledger = std::fs::read(dir.join(LEDGER_NAME)).unwrap();
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .project
        .assets
        .get_mut(&id)
        .unwrap()
        .filmstrip = None;
    let old_pointer = None;
    let thumbnail = dir.join(format!("filmstrip/{id}.jpg"));
    std::fs::remove_file(&thumbnail).unwrap();
    let restore = RestorePermissions(dir.clone(), std::fs::metadata(&dir).unwrap().permissions());
    // The derived-output directory stays writable; publishing a ledger temp file fails.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let response = crate::dispatch::dispatch(
        &state,
        "media.filmstrip",
        json!({"asset":id}),
        cut_core::Actor::system(),
    )
    .await;
    drop(restore);
    assert!(
        !response.ok,
        "a failed atomic ledger write must not return a preview: {response:?}"
    );
    assert!(response
        .error
        .unwrap()
        .cause
        .contains("could not be published"));
    assert_eq!(std::fs::read(dir.join(LEDGER_NAME)).unwrap(), old_ledger);
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.assets[&id].filmstrip,
        old_pointer
    );
    assert!(
        thumbnail.is_file(),
        "the write failure happens after output generation"
    );
}

#[tokio::test]
async fn source_change_during_generation_cannot_publish_a_new_source_claim() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let source_identity = super::super::production::CacheSource::capture(&state, &dir, &id, None)
        .await
        .unwrap();
    let thumbnail = dir.join(format!("filmstrip/{id}.jpg"));
    let old_ledger = std::fs::read(dir.join(LEDGER_NAME)).unwrap();
    fixture(&source, "green", false);
    assert!(source_identity
        .record(&state, &dir, CacheKind::Thumbnails, &thumbnail)
        .await
        .is_err());
    assert_eq!(std::fs::read(dir.join(LEDGER_NAME)).unwrap(), old_ledger);
    assert!(
        !cut_media::cache_trust::trusted(&thumbnail),
        "failed identity must invalidate its origin claim"
    );
}

#[tokio::test]
async fn edited_pending_or_unknown_record_never_retires_planted_output() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let path = dir.join(format!("filmstrip/{id}.jpg"));
    std::fs::write(&path, b"planted output").unwrap();
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .project
        .assets
        .get_mut(&id)
        .unwrap()
        .filmstrip = None;
    for state_value in ["pending", "unknown"] {
        let mut ledger = read_ledger(&dir).unwrap();
        let entry = ledger
            .entries
            .get_mut(&format!("filmstrip/{id}.jpg"))
            .unwrap();
        entry.state = state_value.into();
        entry.output_retired = true;
        write_ledger(&dir, &ledger).unwrap();
        let result = dispatch(&state, "project.cache_rebuild", json!({"asset_ids":[id]})).await;
        assert_eq!(result["scheduled_assets"], 0, "{state_value}: {result}");
        assert_eq!(result["counts"]["unowned_outputs"], 1, "{result}");
        assert_eq!(std::fs::read(&path).unwrap(), b"planted output");
        assert!(!cut_media::cache_trust::trusted(&path));
    }
}

#[tokio::test]
async fn a_valid_planted_proxy_with_a_forged_ready_record_is_not_backfilled() {
    let (root, state, dir) = project().await;
    let source = root.path().join("video.mp4");
    fixture(&source, "red", true);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let proxy_job = finished(
        &state,
        job.result.as_ref().unwrap()["proxy_job"].as_str().unwrap(),
    )
    .await;
    assert_eq!(proxy_job.state, JobState::Done, "{proxy_job:?}");
    assert_bound(
        &dir,
        CacheKind::Proxies,
        &id,
        &cut_core::hash_file(&source).unwrap(),
    );
    let foreign_source = root.path().join("other.mp4");
    fixture(&foreign_source, "green", true);
    let foreign_dir = root.path().join("other-proxies");
    std::fs::create_dir(&foreign_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&foreign_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let foreign_proxy = cut_media::make_proxy(&foreign_source, &foreign_dir, &id).unwrap();
    let proxy = dir.join(format!("proxies/{id}.mp4"));
    std::fs::copy(foreign_proxy, &proxy).unwrap();
    let planted = std::fs::read(&proxy).unwrap();
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .project
        .assets
        .get_mut(&id)
        .unwrap()
        .proxy = None;
    let refused = dispatch(&state, "project.cache_rebuild", json!({"asset_ids":[id]})).await;
    assert_eq!(refused["scheduled_assets"], 0, "{refused}");
    assert_eq!(refused["skipped_assets"][0]["asset_id"], id);
    assert_eq!(refused["skipped_assets"][0]["reason"], "unowned_output");
    assert!(
        state.project.read().await.as_ref().unwrap().project.assets[&id]
            .proxy
            .is_none()
    );
    assert_eq!(std::fs::read(&proxy).unwrap(), planted);
    assert!(!cut_media::cache_trust::trusted(&proxy));
}

#[tokio::test]
async fn literal_old_style_record_stays_legacy_without_modification_or_adoption() {
    let (root, state, dir) = project().await;
    let source = root.path().join("image.png");
    fixture(&source, "red", false);
    let (id, job) = import(&state, &source).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let name = format!("{id}.jpg");
    let key = format!("filmstrip/{name}");
    let legacy = serde_json::to_vec(&json!({
        "schema":"shellx-cut/cache-ownership/1",
        "entries": {key.clone(): {"kind":"filmstrip", "asset":id, "name":name}}
    }))
    .unwrap();
    std::fs::write(dir.join(LEDGER_NAME), &legacy).unwrap();
    state
        .project
        .write()
        .await
        .as_mut()
        .unwrap()
        .project
        .assets
        .get_mut(&id)
        .unwrap()
        .filmstrip = None;
    let output = std::fs::read(dir.join(&key)).unwrap();
    for estimate in [true, false] {
        let refused = dispatch(
            &state,
            "project.cache_rebuild",
            json!({"asset_ids":[id],"estimate_only":estimate}),
        )
        .await;
        assert_eq!(refused["scheduled_assets"], 0, "{refused}");
        assert_eq!(refused["skipped_assets"][0]["reason"], "legacy_output");
        assert_eq!(std::fs::read(dir.join(LEDGER_NAME)).unwrap(), legacy);
        assert_eq!(std::fs::read(dir.join(&key)).unwrap(), output);
        assert!(
            state.project.read().await.as_ref().unwrap().project.assets[&id]
                .filmstrip
                .is_none()
        );
    }
}
