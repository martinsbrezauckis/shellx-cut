//! Installed-run findings: a title counted as footage without
//! a proxy (F8); a missing title movie named an internal asset id (F10); an
//! import cancelled by a project switch left its clip without a proxy for good.

use super::{dispatch_retry_pending, test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn has_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok()
}

async fn create(state: &AppState, root: &Path, name: &str) -> PathBuf {
    let dir = root.join(format!("{name}.cutproj"));
    let made = dispatch_retry_pending(
        state,
        "project.create",
        json!({"name": name, "dir": dir, "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(made.ok, "{name}: {:?}", made.error);
    dir
}

async fn open(state: &AppState, dir: &Path) {
    let opened = dispatch_retry_pending(
        state,
        "project.open",
        json!({"path": dir, "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);
}

async fn wait_kind(state: &AppState, kind: &str, after: &[String]) -> crate::jobs::JobRecord {
    for _ in 0..400 {
        if let Some(job) = state
            .jobs
            .list()
            .into_iter()
            .find(|j| j.kind == kind && !after.contains(&j.job_id))
        {
            return wait_job(state, &job.job_id, 60).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no {kind} job started");
}

fn video_asset(path: &Path) -> cut_core::Asset {
    cut_core::Asset {
        path: path.display().to_string(),
        // The real content hash: a proxy or a resumed import reads only the
        // file that was imported.
        hash: cut_core::hash_file(path).unwrap_or_else(|_| "sha256:missing".into()),
        probe: Some(
            json!({"kind": "video", "has_video": true, "width": 320, "height": 180, "duration_ms": 1000}),
        ),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

#[tokio::test]
async fn a_missing_title_movie_is_drawn_again_and_never_named_by_an_asset_id() {
    if !has_ffmpeg() {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "titled").await;
    let added = dispatch(
        &state,
        "title.add",
        json!({"text": "TITLE 147", "range_ms": [0, 1000], "preset": "title_card"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);
    let asset = added.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();
    let movie = PathBuf::from(
        state.project.read().await.as_ref().unwrap().project.assets[&asset]
            .path
            .clone(),
    );
    assert!(movie.exists());

    // Health: a title movie has no proxy to miss (F8, agent side).
    let health = dispatch(&state, "project.health", json!({}), test_actor()).await;
    let rows = health.result.unwrap()["media"]["assets"].clone();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["asset"] == asset.as_str())
        .unwrap()
        .clone();
    assert_eq!(row["proxy"], "not_applicable", "{row}");

    // Gone while the project is open: the next exact frame draws it again.
    std::fs::remove_file(&movie).unwrap();
    crate::dispatch::edit_tools::ensure_overlay_movies(&state)
        .await
        .unwrap();
    assert!(
        movie.exists(),
        "the title movie is drawn again at its own path"
    );
    let job = state
        .jobs
        .list()
        .into_iter()
        .find(|j| j.kind == "title_remake")
        .unwrap();
    assert!(job.result.unwrap().to_string().contains("TITLE 147"));

    // Gone while the project is closed: opening it draws it again.
    std::fs::remove_file(&movie).unwrap();
    create(&state, root.path(), "other").await;
    open(&state, &dir).await;
    // The reopened project's job list includes its earlier title_remake job, so
    // wait for the file itself, then for a second (this open's) job.
    for _ in 0..400 {
        if movie.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        movie.exists(),
        "opening the project draws the missing title again"
    );
    let remakes = state
        .jobs
        .list()
        .into_iter()
        .filter(|j| j.kind == "title_remake")
        .count();
    assert!(
        remakes >= 2,
        "one remake while open, one at the open: {remakes}"
    );
}

#[tokio::test]
async fn a_title_that_cannot_be_drawn_again_says_so_in_plain_words() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "lost").await;
    // A title movie on the timeline with no title.add in the history.
    let titles = dir.join("titles");
    std::fs::create_dir_all(&titles).unwrap();
    let movie = titles.join("title_0011223344556677.mov");
    let id = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .record_import(None, video_asset(&movie), cut_core::Actor::system(), None)
            .unwrap()
            .0
    };
    let placed = dispatch(
        &state,
        "edit.insert",
        json!({"asset": id, "track": "v1", "at_ms": 0, "src_range_ms": [0, 1000]}),
        test_actor(),
    )
    .await;
    assert!(placed.ok, "{:?}", placed.error);
    let error = crate::dispatch::edit_tools::ensure_overlay_movies(&state)
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .starts_with("a title could not be drawn: its rendered file is missing"),
        "{}",
        error.message
    );
    assert!(
        !error.message.contains(&id) && !error.message.contains("asset"),
        "{}",
        error.message
    );
    assert!(error
        .suggested_action
        .as_deref()
        .unwrap_or("")
        .contains("add the title again"));
}

#[tokio::test]
async fn opening_a_project_makes_the_proxies_its_footage_lacks_except_where_the_person_said_no() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "lacking").await;
    let media = root.path().join("media");
    std::fs::create_dir_all(&media).unwrap();
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        for name in ["cancelled.mp4", "no_proxy.mp4"] {
            std::fs::write(media.join(name), b"source bytes").unwrap();
        }
        // a1: imported, its proxy job cancelled by a project switch.
        store
            .record_import(
                Some("a1".into()),
                video_asset(&media.join("cancelled.mp4")),
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
        // a2: imported with proxy:false by the person.
        store
            .record_import_with(
                Some("a2".into()),
                video_asset(&media.join("no_proxy.mp4")),
                cut_core::Actor::system(),
                None,
                json!({"proxy": false}),
            )
            .unwrap();
        // a4: an import stopped before its probe (a switch right after the import).
        std::fs::write(media.join("unprobed.mp4"), b"source bytes").unwrap();
        let mut unprobed = video_asset(&media.join("unprobed.mp4"));
        unprobed.probe = None;
        store
            .record_import(Some("a4".into()), unprobed, cut_core::Actor::system(), None)
            .unwrap();
        // a5: footage whose file was replaced after the import (another file
        // at the recorded path): nothing is read from it.
        std::fs::write(media.join("replaced.mp4"), b"imported bytes").unwrap();
        let replaced = video_asset(&media.join("replaced.mp4"));
        std::fs::write(media.join("replaced.mp4"), b"other bytes").unwrap();
        store
            .record_import(Some("a5".into()), replaced, cut_core::Actor::system(), None)
            .unwrap();
        // a3: a title movie Cut drew (never a proxy).
        let titles = dir.join("titles");
        std::fs::create_dir_all(&titles).unwrap();
        std::fs::write(titles.join("title_aabbccddeeff0011.mov"), b"drawn").unwrap();
        store
            .record_import(
                Some("a3".into()),
                video_asset(&titles.join("title_aabbccddeeff0011.mov")),
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
        store.save().unwrap();
    }
    create(&state, root.path(), "other").await;
    let before: Vec<String> = state.jobs.list().into_iter().map(|j| j.job_id).collect();
    open(&state, &dir).await;
    let check = wait_kind(&state, "proxy_check", &before).await;
    let result: Value = check.result.unwrap();
    assert_eq!(result["proxy_check"]["made"], json!(["a1"]), "{result}");
    assert_eq!(
        result["proxy_check"]["imports_resumed"],
        json!(["a4"]),
        "{result}"
    );
    assert_eq!(
        result["proxy_check"]["source_changed"],
        json!(["a5"]),
        "{result}"
    );
    // The proxy is made through the usual visible job (one encode at a time).
    let proxy_job = wait_kind(&state, "proxy", &before).await;
    assert_eq!(proxy_job.kind, "proxy");
}
