//! Which held proxies an opened project gets back.
use super::*;

fn asset() -> cut_core::Asset {
    cut_core::Asset {
        path: "/outside/source.mov".into(),
        hash: "sha256:fixture".into(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

#[test]
fn only_a_proxy_this_engine_made_is_given_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(dir.path(), "proxies", None).unwrap();
    let proxies = store.dir.join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    // a1: a proxy file planted under the engine's own name.
    std::fs::write(proxies.join("a1.mp4"), b"planted frames").unwrap();
    // a2: a proxy this engine made (recorded), unchanged since.
    std::fs::write(proxies.join("a2.mp4"), b"engine frames").unwrap();
    cut_media::cache_trust::remember_bound(
        &proxies.join("a2.mp4"),
        &proxy_binding("a2", "sha256:fixture"),
    );
    // a3: made here, then changed by someone else.
    std::fs::write(proxies.join("a3.mp4"), b"engine frames 3").unwrap();
    cut_media::cache_trust::remember_bound(
        &proxies.join("a3.mp4"),
        &proxy_binding("a3", "sha256:fixture"),
    );
    std::fs::write(proxies.join("a3.mp4"), b"replaced frames").unwrap();
    // a4: made here, its pointer lost when the project was saved early.
    std::fs::write(proxies.join("a4.mp4"), b"engine frames 4").unwrap();
    cut_media::cache_trust::remember_bound(
        &proxies.join("a4.mp4"),
        &proxy_binding("a4", "sha256:fixture"),
    );
    for id in ["a1", "a2", "a3", "a4"] {
        store
            .record_import(Some(id.into()), asset(), cut_core::Actor::system(), None)
            .unwrap();
        if id != "a4" {
            store.project.assets.get_mut(id).unwrap().proxy = Some(format!("proxies/{id}.mp4"));
        }
    }

    let held = hold_proxies(&mut store);
    assert!(store
        .project
        .assets
        .values()
        .all(|asset| asset.proxy.is_none()));
    let (mut trusted, mut remake) = sort_held(&held);
    trusted.sort();
    remake.sort();

    assert_eq!(
        trusted,
        [
            ("a2".to_string(), "proxies/a2.mp4".to_string()),
            ("a4".to_string(), "proxies/a4.mp4".to_string())
        ]
    );
    assert_eq!(remake, ["a1", "a3"]);
}

#[test]
fn another_assets_proxy_is_never_given_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(dir.path(), "cross", None).unwrap();
    let proxies = store.dir.join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    // b1's proxy, made here for b1's source.
    std::fs::write(proxies.join("b1.mp4"), b"b1 frames").unwrap();
    cut_media::cache_trust::remember_bound(
        &proxies.join("b1.mp4"),
        &proxy_binding("b1", "sha256:fixture"),
    );
    for id in ["a1", "b1", "c1", "d1"] {
        store
            .record_import(Some(id.into()), asset(), cut_core::Actor::system(), None)
            .unwrap();
    }
    // a1 points at b1's proxy.
    store.project.assets.get_mut("a1").unwrap().proxy = Some("proxies/b1.mp4".into());
    // c1's own name is a second name (hard link) of b1's proxy.
    std::fs::hard_link(proxies.join("b1.mp4"), proxies.join("c1.mp4")).unwrap();
    store.project.assets.get_mut("c1").unwrap().proxy = Some("proxies/c1.mp4".into());
    // d1's own name is a symbolic link to b1's proxy.
    #[cfg(unix)]
    std::os::unix::fs::symlink(proxies.join("b1.mp4"), proxies.join("d1.mp4")).unwrap();
    // b1's own pointer.
    store.project.assets.get_mut("b1").unwrap().proxy = Some("proxies/b1.mp4".into());
    let held = hold_proxies(&mut store);
    let (trusted, mut remake) = sort_held(&held);
    remake.sort();
    let given: Vec<&str> = trusted.iter().map(|(id, _)| id.as_str()).collect();
    assert!(!given.contains(&"a1"), "another asset's proxy: {trusted:?}");
    assert!(
        !given.contains(&"c1"),
        "a second name of another asset's proxy: {trusted:?}"
    );
    assert!(
        !given.contains(&"d1"),
        "a link to another asset's proxy: {trusted:?}"
    );
    // b1's proxy is a plain file now? Its hard link makes it a second name too.
    assert!(remake.contains(&"a1".to_string()));
}

#[test]
fn a_proxy_made_for_other_source_content_is_not_given_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(dir.path(), "resourced", None).unwrap();
    let proxies = store.dir.join("proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    std::fs::write(proxies.join("a1.mp4"), b"frames of the old source").unwrap();
    cut_media::cache_trust::remember_bound(
        &proxies.join("a1.mp4"),
        &proxy_binding("a1", "sha256:old"),
    );
    store
        .record_import(Some("a1".into()), asset(), cut_core::Actor::system(), None)
        .unwrap();
    store.project.assets.get_mut("a1").unwrap().proxy = Some("proxies/a1.mp4".into());
    let held = hold_proxies(&mut store);
    let (trusted, remake) = sort_held(&held);
    assert!(
        trusted.is_empty(),
        "made for sha256:old, the asset says sha256:fixture"
    );
    assert_eq!(remake, ["a1"]);
}
