//! Proxies held back when a project opens are given back by a background check
//! (proxy_trust.rs). The check must be visible: while it runs `project.state`
//! says which assets wait (`proxy_check`), and its end is ONE `job_progress`
//! 1.0 of a `proxy_check` job, the event every client re-reads the project on.
//! Before this, the give-back was silent and the UI kept the project it read
//! during the hold: no proxies for the whole session.

use super::test_actor;
use crate::dispatch::dispatch;
use crate::events::Event;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

fn asset(path: &Path) -> cut_core::Asset {
    cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:fixture".into(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

async fn create(state: &AppState, root: &Path, name: &str) {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": name, "dir": root.join(format!("{name}.cutproj")), "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{name}: {:?}", created.error);
}

/// A project whose `count` assets each have a proxy this engine made.
async fn project_with_trusted_proxies(state: &AppState, root: &Path, count: usize) -> Vec<String> {
    create(state, root, "held").await;
    let mut ids = Vec::new();
    let mut guard = state.project.write().await;
    let store = guard.as_mut().unwrap();
    let proxies = store.dir.join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    for n in 0..count {
        let id = format!("a{}", n + 1);
        let proxy = proxies.join(format!("{id}.mp4"));
        std::fs::write(&proxy, format!("engine frames {n}")).unwrap();
        cut_media::cache_trust::remember_bound(
            &proxy,
            &crate::dispatch::proxy_binding(&id, "sha256:fixture"),
        );
        store
            .record_import(
                Some(id.clone()),
                asset(&root.join("source.mp4")),
                cut_core::Actor::system(),
                None,
            )
            .unwrap();
        store.project.assets.get_mut(&id).unwrap().proxy = Some(format!("proxies/{id}.mp4"));
        ids.push(id);
    }
    store.save().unwrap();
    ids
}

async fn state_of(state: &AppState) -> Value {
    let read = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(read.ok, "{:?}", read.error);
    read.result.unwrap()
}

/// Open `held.cutproj` again and collect the proxy-check completion events.
async fn reopen_and_wait(state: &AppState, root: &Path, ids: &[String]) -> (Value, usize) {
    create(state, root, "other").await;
    let mut events = state.events.subscribe();
    let opened = dispatch(
        state,
        "project.open",
        json!({"path": root.join("held.cutproj"), "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);

    // Truth during the hold: an asset is either already given back or listed
    // as being checked, never just "no proxy".
    let during = state_of(state).await;
    let waiting: Vec<String> = during["proxy_check"]["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        assert!(
            during["assets"][id]["proxy"].is_string() || waiting.contains(id),
            "{id} is neither given back nor listed as being checked: {}",
            during["proxy_check"]
        );
    }

    let mut done = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(Event::JobProgress { kind, progress, .. }))
                if kind == "proxy_check" && progress >= 1.0 =>
            {
                done += 1;
                // Anything else arriving shortly after would be a second refresh.
                let quiet = tokio::time::Instant::now() + Duration::from_millis(500);
                while let Ok(Ok(event)) = tokio::time::timeout_at(quiet, events.recv()).await {
                    if let Event::JobProgress { kind, progress, .. } = event {
                        if kind == "proxy_check" && progress >= 1.0 {
                            done += 1;
                        }
                    }
                }
                break;
            }
            Ok(Ok(_)) => continue,
            Ok(Err(error)) => panic!("event stream: {error}"),
            Err(_) => break,
        }
    }
    (state_of(state).await, done)
}

#[tokio::test]
async fn given_back_proxies_are_announced_once_and_reach_project_state() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let ids = project_with_trusted_proxies(&state, root.path(), 1).await;
    let (after, done) = reopen_and_wait(&state, root.path(), &ids).await;
    assert_eq!(
        done, 1,
        "the check's end is announced (job_progress 1.0 of the proxy_check job)"
    );
    assert_eq!(after["assets"]["a1"]["proxy"], "proxies/a1.mp4");
    assert!(
        after.get("proxy_check").is_none(),
        "no check pending after it ended"
    );
    let jobs = dispatch(&state, "jobs.list", json!({}), test_actor()).await;
    let listed = jobs.result.unwrap().to_string();
    assert!(
        listed.contains("given_back"),
        "jobs.list shows the finished check: {listed}"
    );
}

#[tokio::test]
async fn two_hundred_proxies_are_given_back_with_one_event() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let ids = project_with_trusted_proxies(&state, root.path(), 200).await;
    let (after, done) = reopen_and_wait(&state, root.path(), &ids).await;
    assert_eq!(
        done, 1,
        "one refresh for the whole batch, not one per asset"
    );
    for id in &ids {
        assert_eq!(after["assets"][id]["proxy"], format!("proxies/{id}.mp4"));
    }
}

#[tokio::test]
async fn the_pending_check_is_reported_for_the_open_project_only() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    create(&state, root.path(), "held").await;
    let dir = state.project.read().await.as_ref().unwrap().dir.clone();
    *state.proxy_check.lock().unwrap() = Some(crate::state::PendingProxyCheck {
        dir: dir.clone(),
        job_id: "job_x".into(),
        assets: vec!["a1".into()],
    });
    let read = state_of(&state).await;
    assert_eq!(
        read["proxy_check"],
        json!({"state": "checking", "job_id": "job_x", "assets": ["a1"]})
    );
    *state.proxy_check.lock().unwrap() = Some(crate::state::PendingProxyCheck {
        dir: root.path().join("elsewhere.cutproj"),
        job_id: "job_y".into(),
        assets: vec!["a1".into()],
    });
    assert!(state_of(&state).await.get("proxy_check").is_none());
}
