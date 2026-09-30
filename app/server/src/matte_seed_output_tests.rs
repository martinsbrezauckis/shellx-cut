//! Static link regression and legitimate warm-cache control for ordinary seeds.
use super::*;

fn matte() -> ClipMatte {
    serde_json::from_str(r#"{"model":"matanyone","mode":"remove","quality":"good"}"#).unwrap()
}

#[test]
fn ordinary_rvm_and_sam2_seeds_use_portable_plain_warm_cache_leaves() {
    let root = tempfile::tempdir().unwrap();
    let dir = cut_core::matte_cache::cache_dir(root.path(), true).unwrap();
    let seed = MatteSeed {
        at_ms: 0,
        point: Some([5, 5]),
        bbox: None,
    };
    let runtime = MatanyoneRuntime {
        python: "unused".into(),
        runner: "unused".into(),
        model: "unused".into(),
    };
    let auto = dir.join("sha256=a2.seed.png");
    let picked = dir.join(format!("sha256=a2.{}.seed.png", seed.short_hash()));
    std::fs::write(&auto, b"auto").unwrap();
    std::fs::write(&picked, b"picked").unwrap();
    assert_eq!(
        rvm_seed_mask(&dir, Path::new("unused"), "sha256:a2").unwrap(),
        auto
    );
    assert_eq!(
        sam2_seed_mask(&dir, Path::new("unused"), "sha256:a2", &seed, &runtime).unwrap(),
        picked
    );
    assert!(rvm_seed_mask(&dir, Path::new("unused"), "../outside").is_err());
    let mut m = matte();
    m.seed = Some(seed);
    assert_eq!(
        resolve_seed_mask(&dir, Path::new("unused"), "sha256:a2", &m, &runtime).unwrap(),
        picked
    );
}

#[cfg(unix)]
#[test]
fn ordinary_seed_static_links_fail_before_runtime_resolution() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let dir = cut_core::matte_cache::cache_dir(root.path(), true).unwrap();
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"sentinel").unwrap();
    let seed = MatteSeed {
        at_ms: 0,
        point: Some([5, 5]),
        bbox: None,
    };
    let runtime = MatanyoneRuntime {
        python: "unused".into(),
        runner: "unused".into(),
        model: "unused".into(),
    };
    symlink(&outside, dir.join("sha256=a2.seed.png")).unwrap();
    symlink(
        &outside,
        dir.join(format!("sha256=a2.{}.seed.png", seed.short_hash())),
    )
    .unwrap();
    assert!(rvm_seed_mask(&dir, Path::new("unused"), "sha256:a2")
        .unwrap_err()
        .message
        .contains("unsafe"));
    assert!(
        sam2_seed_mask(&dir, Path::new("unused"), "sha256:a2", &seed, &runtime)
            .unwrap_err()
            .message
            .contains("unsafe")
    );
    assert_eq!(std::fs::read(outside).unwrap(), b"sentinel");
}
