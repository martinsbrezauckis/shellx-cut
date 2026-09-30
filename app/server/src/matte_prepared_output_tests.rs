use super::*;
use crate::matte::premium_runtime::PreparedMatanyoneBinding;

fn runtime(script: &Path) -> PreparedMatanyoneRuntime {
    PreparedMatanyoneRuntime {
        python: "/usr/bin/python3".into(),
        rvm_script: script.into(),
        matanyone_script: script.into(),
        sam2_script: script.into(),
        rvm_model: "rvm.fixture".into(),
        matanyone_model: "matanyone.fixture".into(),
        sam2_model: "sam2.fixture".into(),
        binding: PreparedMatanyoneBinding {
            contract: "test/v1".into(),
            manifest_sha256: "a".repeat(64),
            receipt_sha256: "b".repeat(64),
            rvm_model_id: "rvm".into(),
            rvm_model_sha256: "c".repeat(64),
            matanyone_model_id: "matanyone".into(),
            matanyone_model_sha256: "d".repeat(64),
            sam2_model_id: "sam2".into(),
            sam2_model_sha256: "e".repeat(64),
        },
    }
}

#[cfg(unix)]
#[test]
fn prepared_seed_runners_keep_masks_identity_and_owned_png_outputs() {
    let root = tempfile::tempdir().unwrap();
    let dir = cut_core::matte_cache::cache_dir(root.path(), true).unwrap();
    let script = root.path().join("mock_seed.py");
    std::fs::write(&script,"import sys,pathlib\nargs=sys.argv\nout=args[args.index('--first-frame-mask')+1] if '--first-frame-mask' in args else args[2]\nassert out.endswith('.png')\nassert '--model' in args or '--checkpoint' in args\npathlib.Path(out).write_bytes(b'controlled prepared seed')\n").unwrap();
    let rt = runtime(&script);
    let seed = MatteSeed {
        at_ms: 17,
        point: Some([2, 3]),
        bbox: None,
    };
    let auto = rvm_seed_mask(&dir, Path::new("source.fixture"), "sha256:a2", &rt).unwrap();
    let picked =
        sam2_seed_mask(&dir, Path::new("source.fixture"), "sha256:a2", &seed, &rt).unwrap();
    assert_ne!(auto, picked);
    assert_eq!(std::fs::read(&auto).unwrap(), b"controlled prepared seed");
    assert_eq!(std::fs::read(&picked).unwrap(), b"controlled prepared seed");
    let mut missing = rt.clone();
    missing.python = "absent interpreter".into();
    assert_eq!(
        rvm_seed_mask(&dir, Path::new("unused"), "sha256:a2", &missing).unwrap(),
        auto
    );
    assert_eq!(
        sam2_seed_mask(&dir, Path::new("unused"), "sha256:a2", &seed, &missing).unwrap(),
        picked
    );
    let mut changed = missing;
    changed.binding.receipt_sha256 = "f".repeat(64);
    assert!(rvm_seed_mask(&dir, Path::new("unused"), "sha256:a2", &changed).is_err());
}

#[cfg(unix)]
#[test]
fn prepared_seed_link_is_refused_without_launch_and_outside_target_is_untouched() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let dir = cut_core::matte_cache::cache_dir(root.path(), true).unwrap();
    let rt = runtime(Path::new("absent script"));
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"sentinel").unwrap();
    let seed = MatteSeed {
        at_ms: 0,
        point: Some([2, 3]),
        bbox: None,
    };
    for kind in ["rvm".to_string(), format!("sam2-{}", seed.short_hash())] {
        symlink(&outside, prepared_seed_path(&dir, "sha256:a2", &kind, &rt)).unwrap();
    }
    assert!(rvm_seed_mask(&dir, Path::new("unused"), "sha256:a2", &rt)
        .unwrap_err()
        .message
        .contains("unsafe"));
    assert!(
        sam2_seed_mask(&dir, Path::new("unused"), "sha256:a2", &seed, &rt)
            .unwrap_err()
            .message
            .contains("unsafe")
    );
    assert_eq!(std::fs::read(outside).unwrap(), b"sentinel");
}
