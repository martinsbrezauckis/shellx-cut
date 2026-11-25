//! A project received from someone else: its project.json cache can carry any
//! probe, strip pointer or proxy pointer, and its folders any file at a
//! derived name. After an open, none of those is taken as this engine's work.

use super::{dispatch_retry_pending, test_actor};
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn ffmpeg(args: &[&str]) -> bool {
    std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
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
    state.project.read().await.as_ref().unwrap().dir.clone()
}

/// Switch away (which saves the project), let the sender change the saved
/// cache, then open it again.
async fn reopen(state: &AppState, root: &Path, dir: &Path, change: impl Fn(&mut Value)) {
    create(state, root, "elsewhere").await;
    let mut cache: Value =
        serde_json::from_slice(&std::fs::read(dir.join("project.json")).unwrap()).unwrap();
    change(&mut cache);
    std::fs::write(
        dir.join("project.json"),
        serde_json::to_vec_pretty(&cache).unwrap(),
    )
    .unwrap();
    let opened = dispatch_retry_pending(
        state,
        "project.open",
        json!({"path": dir, "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);
}

/// Poll the open project until `done` holds or `secs` pass; the last value.
async fn settle<T>(
    state: &AppState,
    secs: u64,
    read: impl Fn(&cut_core::Project) -> T,
    done: impl Fn(&T) -> bool,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let value = read(&state.project.read().await.as_ref().unwrap().project);
        if done(&value) || Instant::now() > deadline {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_received_probe_is_read_again_from_the_source() {
    let root = tempfile::tempdir().unwrap();
    let clip = root.path().join("tone.mp4");
    if !ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=30:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        "-shortest",
        "-pix_fmt",
        "yuv420p",
        clip.to_str().unwrap(),
    ]) {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let state = AppState::new();
    let dir = create(&state, root.path(), "received").await;
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": clip, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let id = imported.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();
    let probed = settle(&state, 30, |p| p.assets[&id].probe.clone(), |p| p.is_some()).await;
    assert_eq!(probed.unwrap()["has_audio"], true);
    let _ = dispatch(&state, "project.save", json!({}), test_actor()).await;
    // The sender's cache: no sound, a still image.
    reopen(&state, root.path(), &dir, |cache| {
        cache["assets"][&id]["probe"]["has_audio"] = json!(false);
        cache["assets"][&id]["probe"]["kind"] = json!("image");
    })
    .await;
    let fact = settle(
        &state,
        30,
        |p| p.assets[&id].probe.clone().unwrap_or(Value::Null),
        |p| p["has_audio"] == json!(true),
    )
    .await;
    assert_eq!(fact["has_audio"], true, "the source has sound: {fact}");
    assert_eq!(fact["kind"], "video", "the source moves: {fact}");
}

#[tokio::test]
async fn a_received_strip_pointer_never_names_another_place() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "received").await;
    let media = root.path().join("clip.mp4");
    std::fs::write(&media, b"source bytes").unwrap();
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let asset = cut_core::Asset {
            path: media.display().to_string(),
            hash: "sha256:fixture".into(),
            probe: Some(json!({"kind": "video", "duration_ms": 1000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        };
        store
            .record_import(Some("a1".into()), asset, cut_core::Actor::system(), None)
            .unwrap();
        store.save().unwrap();
    }
    reopen(&state, root.path(), &dir, |cache| {
        cache["assets"]["a1"]["filmstrip"] = json!("/attacker.example/seen.png");
    })
    .await;
    let strip = state.project.read().await.as_ref().unwrap().project.assets["a1"]
        .filmstrip
        .clone();
    assert_ne!(strip.as_deref(), Some("/attacker.example/seen.png"));
}

#[tokio::test]
async fn another_assets_proxy_is_not_given_to_this_asset() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "received").await;
    let proxies = dir.join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    std::fs::write(proxies.join("a2.mp4"), b"frames of a2").unwrap();
    cut_media::cache_trust::remember(&proxies.join("a2.mp4"));
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        for id in ["a1", "a2"] {
            let asset = cut_core::Asset {
                path: root.path().join(format!("{id}.mp4")).display().to_string(),
                hash: "sha256:fixture".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            };
            store
                .record_import(Some(id.into()), asset, cut_core::Actor::system(), None)
                .unwrap();
        }
        store.save().unwrap();
    }
    reopen(&state, root.path(), &dir, |cache| {
        cache["assets"]["a1"]["proxy"] = json!("proxies/a2.mp4");
    })
    .await;
    let proxy = settle(&state, 10, |p| p.assets["a1"].proxy.clone(), |_| false).await;
    assert_ne!(
        proxy.as_deref(),
        Some("proxies/a2.mp4"),
        "a1 never plays a2's proxy"
    );
}

#[tokio::test]
async fn a_planted_present_title_movie_is_drawn_again() {
    if !ffmpeg(&["-version"]) {
        eprintln!("skipped: ffmpeg not found");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "received").await;
    let added = dispatch(
        &state,
        "title.add",
        json!({"text": "TITLE 147", "range_ms": [0, 1000], "preset": "title_card"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);
    let id = added.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();
    let movie =
        PathBuf::from(&state.project.read().await.as_ref().unwrap().project.assets[&id].path);
    // The sender's movie at the title's name.
    std::fs::write(&movie, b"the sender's picture").unwrap();
    reopen(&state, root.path(), &dir, |_| {}).await;
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::fs::read(&movie).is_ok_and(|b| b == b"the sender's picture")
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_ne!(
        std::fs::read(&movie).unwrap(),
        b"the sender's picture",
        "the title is drawn again from its settings"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_is_never_visible_before_its_title_check_is_marked() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = create(&state, root.path(), "received").await;
    create(&state, root.path(), "elsewhere").await;
    // While the mark cannot be set, the opened project must not be visible:
    // a media read in that moment would skip the title check.
    let marks = crate::dispatch::edit_tools::hold_open_checks();
    let opening = tokio::spawn({
        let state = state.clone();
        let dir = dir.clone();
        async move {
            dispatch(
                &state,
                "project.open",
                json!({"path": dir, "cancel_running_jobs": true}),
                test_actor(),
            )
            .await
        }
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut visible_unmarked = false;
    while Instant::now() < deadline && !visible_unmarked {
        // try_read: the fixed open holds the write lock while it waits for the
        // mark. A thread sleep, not a timer: the runtime's workers can be
        // blocked on the held marks, and a timer would then never fire.
        visible_unmarked = state
            .project
            .try_read()
            .is_ok_and(|guard| guard.as_ref().is_some_and(|store| store.dir == dir));
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(marks);
    let opened = opening.await.unwrap();
    assert!(opened.ok, "{:?}", opened.error);
    assert!(
        !visible_unmarked,
        "the project was visible before its open check was marked"
    );
}
