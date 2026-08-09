#[cfg(unix)]
use std::fs;

#[cfg(unix)]
use tempfile::tempdir;

#[cfg(unix)]
use crate::{
    read_manifest, recover_interrupted, CaptureStart, ManifestOwner, OwnerState, MANIFEST_FILE,
};

#[cfg(unix)]
#[test]
fn manifest_symlink_is_rejected_before_read_append_or_recovery() {
    use std::os::unix::fs::symlink;

    let outside = tempdir().unwrap();
    let outside_root = outside.path().join("outside-capture");
    let owner = ManifestOwner::begin(&outside_root, CaptureStart::new("outside", 100)).unwrap();
    drop(owner);
    let outside_manifest = outside_root.join(MANIFEST_FILE);
    let before = fs::read(&outside_manifest).unwrap();

    let capture = tempdir().unwrap();
    symlink(&outside_manifest, capture.path().join(MANIFEST_FILE)).unwrap();
    assert!(read_manifest(capture.path()).is_err());
    assert!(ManifestOwner::open(capture.path()).is_err());
    assert!(recover_interrupted(
        capture.path(),
        "not-invoked",
        "not-invoked",
        OwnerState::Dead,
    )
    .is_err());
    assert_eq!(fs::read(&outside_manifest).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn manifest_begin_rejects_a_capture_root_symlink() {
    use std::os::unix::fs::symlink;

    let outside = tempdir().unwrap();
    let root = tempdir().unwrap();
    let linked = root.path().join("linked-capture");
    symlink(outside.path(), &linked).unwrap();
    assert!(ManifestOwner::begin(&linked, CaptureStart::new("capture", 100)).is_err());
    assert!(!outside.path().join(MANIFEST_FILE).exists());
}
