use super::*;
use crate::matte::native_runtime::PreparedRvmBinding;
use crate::matte::output::bake_seed;
#[cfg(unix)]
use crate::matte::output::StagedOutput;
use std::path::PathBuf;

fn matte() -> ClipMatte {
    serde_json::from_str(r#"{"mode":"remove","model":"rvm","quality":"good"}"#).unwrap()
}
fn stats() -> MatteStats {
    serde_json::from_value(
        serde_json::json!({"frames":2,"fps":24.0,"width":16,"height":16,
        "coverage_mean":0.5,"cov_min":0.4,"cov_max":0.6,"temporal_flicker":0.1}),
    )
    .unwrap()
}
fn binding(receipt: &str) -> PreparedMatteBinding {
    PreparedMatteBinding::Rvm(PreparedRvmBinding {
        contract: "test/v1".into(),
        manifest_sha256: "a".repeat(64),
        receipt_sha256: receipt.repeat(64),
        model_id: "rvm".into(),
        model_sha256: "c".repeat(64),
    })
}
fn leaves(project: &Path) -> (PathBuf, PathBuf) {
    let alpha = cut_core::matte_cache::cache_dir(project, true)
        .unwrap()
        .join(matte().cache_filename("sha256:a2"));
    (alpha.clone(), alpha.with_extension("json"))
}

#[test]
fn all_path_spellings_fail_before_a_bake_and_leave_outside_sentinel_intact() {
    let root = tempfile::tempdir().unwrap();
    let sentinel = root.path().join("outside");
    std::fs::write(&sentinel, b"untouched").unwrap();
    for hash in [
        "../../../outside",
        "/outside",
        "C:\\outside",
        "a\\..\\outside",
        "a:stream",
        "sha256:a2:stream",
    ] {
        assert!(ensure(root.path(), hash, &matte(), None, |_, _| panic!(
            "must not bake"
        ))
        .is_err());
        assert!(cut_core::matte_cache::alpha_path(root.path(), hash, &matte()).is_err());
    }
    assert_eq!(std::fs::read(sentinel).unwrap(), b"untouched");
}

#[test]
fn normal_cache_hits_and_exact_prepared_receipt_binding_survive() {
    let root = tempfile::tempdir().unwrap();
    let first = binding("b");
    let baked = ensure(
        root.path(),
        "sha256:a2",
        &matte(),
        Some(first.clone()),
        |_, alpha| {
            assert_eq!(alpha.extension().unwrap(), "mkv");
            std::fs::write(alpha, b"alpha")?;
            Ok(stats())
        },
    )
    .unwrap();
    assert!(!baked.cached);
    let cached = ensure(root.path(), "sha256:a2", &matte(), Some(first), |_, _| {
        panic!("warm cache")
    })
    .unwrap();
    assert!(cached.cached);
    let changed = ensure(
        root.path(),
        "sha256:a2",
        &matte(),
        Some(binding("d")),
        |_, alpha| {
            std::fs::write(alpha, b"changed")?;
            Ok(stats())
        },
    )
    .unwrap();
    assert!(!changed.cached);
    assert_eq!(changed.native_runtime, Some(binding("d")));
    assert_eq!(std::fs::read(leaves(root.path()).0).unwrap(), b"changed");
}

#[test]
fn failed_and_empty_bakes_preserve_previous_finals_and_clean_only_owned_stages() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, receipt) = leaves(root.path());
    std::fs::write(&alpha, b"old alpha").unwrap();
    std::fs::write(&receipt, b"invalid receipt").unwrap();
    let unrelated = alpha.parent().unwrap().join("keep.txt");
    std::fs::write(&unrelated, b"keep").unwrap();
    assert!(
        ensure(root.path(), "sha256:a2", &matte(), None, |_, staged| {
            std::fs::write(staged, b"partial")?;
            Err(io_err("controlled", "failed bake"))
        })
        .is_err()
    );
    assert!(
        ensure(root.path(), "sha256:a2", &matte(), None, |_, staged| {
            std::fs::write(staged, b"")?;
            Ok(stats())
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&alpha).unwrap(), b"old alpha");
    assert_eq!(std::fs::read(&receipt).unwrap(), b"invalid receipt");
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
    assert!(std::fs::read_dir(alpha.parent().unwrap())
        .unwrap()
        .all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".cut-matte-")));
}

#[test]
fn controlled_http_body_uses_the_same_atomic_cache_boundary() {
    let root = tempfile::tempdir().unwrap();
    let header = serde_json::to_string(&stats()).unwrap();
    let baked = ensure(root.path(), "sha256:a2", &matte(), None, |_, staged| {
        crate::matte::receive_http_alpha(staged, &mut std::io::Cursor::new(b"HTTP alpha"), &header)
    })
    .unwrap();
    assert!(!baked.cached);
    assert_eq!(std::fs::read(leaves(root.path()).0).unwrap(), b"HTTP alpha");
    assert!(
        ensure(root.path(), "different", &matte(), None, |_, staged| {
            crate::matte::receive_http_alpha(staged, &mut std::io::Cursor::new(b"bad"), "malformed")
        })
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn controlled_local_runner_receives_real_extension_quality_and_model_without_final_write() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("mock.py");
    let model = root.path().join("model.fixture");
    let source = root.path().join("source.fixture");
    std::fs::write(&script, format!("import sys,pathlib\nassert sys.argv[2].endswith('.mkv')\nassert '--downsample' in sys.argv and '0.25' in sys.argv\nassert sys.argv[sys.argv.index('--model')+1] == {:?}\npathlib.Path(sys.argv[2]).write_bytes(b'controlled local alpha')\nprint({:?})\n", model.to_str().unwrap(), serde_json::to_string(&stats()).unwrap())).unwrap();
    let mut fast = matte();
    fast.quality = cut_core::MatteQuality::Fast;
    let baked = ensure(root.path(), "rawhex", &fast, None, |_, staged| {
        crate::matte::bake_local(
            Path::new("/usr/bin/python3"),
            &script,
            &model,
            &source,
            staged,
            &fast,
            false,
        )
    })
    .unwrap();
    assert!(!baked.cached);
    let final_path = root
        .path()
        .join("cache/matte")
        .join(fast.cache_filename("rawhex"));
    assert_eq!(
        std::fs::read(final_path).unwrap(),
        b"controlled local alpha"
    );
}

#[test]
fn seed_stage_keeps_png_extension_hits_cache_and_cleans_failed_runner_output() {
    let root = tempfile::tempdir().unwrap();
    let dir = cut_core::matte_cache::cache_dir(root.path(), true).unwrap();
    for name in [
        "sha256=a2.seed.png",
        "sha256=a2.0123456789abcdef.seed.png",
        "asset-prepared-identity.seed.png",
    ] {
        let out = dir.join(name);
        let baked = bake_seed(&out, |stage| {
            assert_eq!(stage.extension().unwrap(), "png");
            std::fs::write(stage, b"seed")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(baked, out);
        bake_seed(&out, |_| panic!("warm seed")).unwrap();
    }
    let failed = dir.join("failed.seed.png");
    assert!(bake_seed(&failed, |stage| {
        std::fs::write(stage, b"partial")?;
        Err(io_err("seed", "failure"))
    })
    .is_err());
    assert!(!failed.exists());
    assert!(std::fs::read_dir(dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".cut-matte-")));
}

#[cfg(unix)]
#[test]
fn linked_dangling_and_hardlinked_alpha_receipt_and_seed_are_refused() {
    use std::os::unix::fs::symlink;
    for kind in ["symlink", "dangling", "hardlink"] {
        for which in ["alpha", "receipt", "seed"] {
            let root = tempfile::tempdir().unwrap();
            let (alpha, receipt) = leaves(root.path());
            let outside = root.path().join("outside");
            std::fs::write(&outside, b"sentinel").unwrap();
            let leaf = match which {
                "alpha" => alpha,
                "receipt" => receipt,
                _ => alpha.parent().unwrap().join("seed.png"),
            };
            match kind {
                "hardlink" => std::fs::hard_link(&outside, &leaf).unwrap(),
                "dangling" => symlink(root.path().join("absent"), &leaf).unwrap(),
                _ => symlink(&outside, &leaf).unwrap(),
            }
            let result = if which == "seed" {
                bake_seed(&leaf, |_| panic!("unsafe seed")).map(|_| ())
            } else {
                ensure(root.path(), "sha256:a2", &matte(), None, |_, _| {
                    panic!("unsafe leaf")
                })
                .map(|_| ())
            };
            assert!(result.is_err(), "{kind}/{which}");
            assert_eq!(std::fs::read(outside).unwrap(), b"sentinel");
        }
    }
}

#[cfg(unix)]
#[test]
fn linked_cache_parents_and_late_linked_publication_are_refused() {
    use std::os::unix::fs::symlink;
    for component in ["cache", "matte"] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = if component == "cache" {
            root.path().join("cache")
        } else {
            std::fs::create_dir(root.path().join("cache")).unwrap();
            root.path().join("cache/matte")
        };
        symlink(outside.path(), link).unwrap();
        assert!(
            ensure(root.path(), "sha256:a2", &matte(), None, |_, _| panic!(
                "linked parent"
            ))
            .is_err()
        );
        assert!(cut_core::matte_cache::alpha_path(root.path(), "sha256:a2", &matte()).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
    let root = tempfile::tempdir().unwrap();
    let (alpha, _) = leaves(root.path());
    let stage = StagedOutput::new(&alpha).unwrap();
    stage.write(b"completed").unwrap();
    let sentinel = root.path().join("outside");
    std::fs::write(&sentinel, b"sentinel").unwrap();
    symlink(&sentinel, &alpha).unwrap();
    assert!(stage.publish().is_err());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"sentinel");
}

#[cfg(unix)]
#[test]
fn legacy_prefixed_cache_renders_offline_and_migrates_without_rebake() {
    let root = tempfile::tempdir().unwrap();
    let (portable, _) = leaves(root.path());
    let legacy = portable.parent().unwrap().join("sha256:a2.rvm.good.mkv");
    std::fs::write(&legacy, b"legacy alpha").unwrap();
    std::fs::write(
        legacy.with_extension("json"),
        serde_json::to_vec(&stats()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        cut_core::matte_cache::alpha_path(root.path(), "sha256:a2", &matte()).unwrap(),
        legacy
    );
    assert!(
        ensure(root.path(), "sha256:a2", &matte(), None, |_, _| panic!(
            "migration must be offline"
        ))
        .unwrap()
        .cached
    );
    assert_eq!(std::fs::read(&portable).unwrap(), b"legacy alpha");
    assert_eq!(
        cut_core::matte_cache::alpha_path(root.path(), "sha256:a2", &matte()).unwrap(),
        portable
    );
}
