use super::*;
use crate::state::AppState;

fn asset(path: &Path, kind: &str, proxy: Option<&str>, filmstrip: Option<&str>) -> cut_core::Asset {
    cut_core::Asset {
        path: path.display().to_string(),
        hash: "sha256:test".into(),
        probe: Some(json!({"kind": kind})),
        transcript: None,
        perception: None,
        proxy: proxy.map(str::to_string),
        filmstrip: filmstrip.map(str::to_string),
    }
}

async fn state_with_project() -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"health", "dir": root.path().join("health.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "project create: {:?}", created.error);
    (root, state)
}

#[tokio::test]
async fn pages_media_by_revision_without_paths_or_global_counts() {
    let (root, state) = state_with_project().await;
    let source_a1 = root.path().join("one.mp4");
    let source_a3 = root.path().join("three.wav");
    std::fs::write(&source_a1, b"one").unwrap();
    std::fs::write(&source_a3, b"three").unwrap();
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().unwrap();
        std::fs::write(store.proxies_dir().join("a1.mp4"), b"proxy").unwrap();
        std::fs::write(store.dir.join("filmstrip/a1.jpg"), b"filmstrip").unwrap();
        store.project.assets.insert(
            "a1".into(),
            asset(
                &source_a1,
                "video",
                Some("proxies/a1.mp4"),
                Some("filmstrip/a1.jpg"),
            ),
        );
        store.project.assets.insert(
            "a2".into(),
            asset(
                &root.path().join("missing.mp4"),
                "video",
                Some("proxies/a2.mp4"),
                Some("filmstrip/a2.jpg"),
            ),
        );
        store
            .project
            .assets
            .insert("a3".into(), asset(&source_a3, "audio", None, None));
        store.project.assets.insert(
            "a10".into(),
            asset(&root.path().join("ten.mp4"), "video", None, None),
        );
    }

    let first = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({"limit": 2}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(first.ok, "health: {:?}", first.error);
    let first = first.result.unwrap();
    let revision = first["project_revision"].as_str().unwrap().to_string();
    assert_eq!(first["journal"]["status"], "verified");
    assert_eq!(first["media"]["asset_count"], 4);
    assert_eq!(first["media"]["checked_count"], 2);
    assert_eq!(first["media"]["assets"][0]["asset"], "a1");
    assert_eq!(first["media"]["assets"][1]["asset"], "a2");
    assert_eq!(first["media"]["page"]["offline"], 1);
    assert_eq!(first["media"]["assets"][0]["proxy"], "available");
    assert_eq!(first["media"]["assets"][1]["filmstrip"], "missing");
    assert!(!first.to_string().contains(&source_a1.display().to_string()));
    assert_eq!(first["media"]["next_cursor"], "a2");
    assert_eq!(first["media"]["has_more"], true);

    let second = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({"limit": 2, "cursor":"a2", "revision":revision}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(second.ok, "second page: {:?}", second.error);
    let second = second.result.unwrap();
    assert_eq!(second["media"]["assets"][0]["asset"], "a3");
    assert_eq!(second["media"]["assets"][1]["asset"], "a10");
    assert_eq!(second["media"]["assets"][0]["proxy"], "not_applicable");
    assert_eq!(second["media"]["assets"][0]["filmstrip"], "not_applicable");
    assert_eq!(second["media"]["has_more"], false);
    assert!(second["media"].get("next_cursor").is_none());
    assert_eq!(second["media"]["cursor"], "a2");
}

#[tokio::test]
async fn first_page_reports_only_rebuildable_cache_bytes() {
    let (_root, state) = state_with_project().await;
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().unwrap();
        let source = store.dir.join("source.mp4");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(store.proxies_dir().join("a1.mp4"), b"proxy").unwrap();
        std::fs::write(store.dir.join("filmstrip/a1.jpg"), b"thumb").unwrap();
        std::fs::write(store.proxies_dir().join("a2.mp4"), b"orphan").unwrap();
        std::fs::create_dir_all(store.dir.join("exports")).unwrap();
        std::fs::write(store.dir.join("exports/final.mp4"), b"not cache").unwrap();
        store.project.assets.insert(
            "a1".into(),
            asset(
                &source,
                "video",
                Some("proxies/a1.mp4"),
                Some("filmstrip/a1.jpg"),
            ),
        );
    }

    let first = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({"limit": 1}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    assert_eq!(first["editing_cache"]["status"], "ready");
    assert_eq!(first["editing_cache"]["bytes"], 16);
    assert_eq!(first["editing_cache"]["files"], 3);
    assert_eq!(first["editing_cache"]["reclaimable_bytes"], 6);
    assert_eq!(first["editing_cache"]["reclaimable_files"], 1);
    assert_eq!(first["editing_cache"]["cleanup_preview"]["status"], "ready");
    assert_eq!(
        first["editing_cache"]["cleanup_preview"]["minimum_age_ms"],
        86_400_000
    );
    assert_eq!(
        first["editing_cache"]["cleanup_preview"]["aged_unreferenced_files"],
        0
    );
    assert_eq!(
        first["editing_cache"]["cleanup_preview"]["recent_unreferenced_bytes"],
        6
    );
    assert_eq!(first["editing_cache"]["categories"][0]["kind"], "proxies");
    assert_eq!(
        first["editing_cache"]["categories"][1]["kind"],
        "thumbnails"
    );
    assert!(first["editing_cache"].get("latest_modified_ms").is_some());
    assert!(!first.to_string().contains("final.mp4"));

    let revision = first["project_revision"].as_str().unwrap();
    let continuation = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({"limit": 1, "cursor": "a1", "revision": revision}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    assert!(continuation.get("editing_cache").is_none());
}

#[tokio::test]
async fn identity_drift_is_journal_attention_and_refuses_asset_membership() {
    let (_root, state) = state_with_project().await;
    let journal = {
        let project = state.project.read().await;
        project.as_ref().unwrap().log.path.clone()
    };
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(journal)
        .unwrap()
        .write_all(b"{\"external\":true}\n")
        .unwrap();

    let health = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(
        health.ok,
        "identity loss is an honest read, not a false error"
    );
    let health = health.result.unwrap();
    assert!(health.get("project_revision").is_none());
    assert_eq!(health["journal"]["status"], "attention");
    assert_eq!(
        health["journal"]["notices"][0]["code"],
        "identity_revalidation_failed"
    );
    assert_eq!(health["media"]["status"], "unavailable");
    assert_eq!(health["media"]["asset_count"], 0);
    assert_eq!(health["media"]["checked_count"], 0);
}

#[tokio::test]
async fn stale_revision_and_unbound_cursor_fail_closed() {
    let (_root, state) = state_with_project().await;
    let first = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    let old_revision = first["project_revision"].as_str().unwrap().to_string();
    let marker = crate::dispatch::dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 1, "label": "revision changes"}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(marker.ok, "marker: {:?}", marker.error);
    let stale = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({"cursor":"a1", "revision":old_revision}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!stale.ok);
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let unbound = crate::dispatch::dispatch(
        &state,
        "project.health",
        json!({"cursor":"a1"}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(!unbound.ok);
    assert_eq!(unbound.error.unwrap().code, error_codes::INVALID_ARGS);
}
