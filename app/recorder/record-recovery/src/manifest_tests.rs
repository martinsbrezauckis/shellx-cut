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

/// Item 140 sweep: a received capture folder can carry its manifest as a
/// second name (hard link) of another capture's manifest elsewhere. Opening it
/// for the recovery receipt must not append to that other file.
#[test]
fn manifest_hard_link_is_not_appended_to() {
    let outside = tempfile::tempdir().unwrap();
    let outside_root = outside.path().join("outside-capture");
    let owner =
        crate::ManifestOwner::begin(&outside_root, crate::CaptureStart::new("outside", 100))
            .unwrap();
    drop(owner);
    let outside_manifest = outside_root.join(crate::MANIFEST_FILE);
    let before = std::fs::read(&outside_manifest).unwrap();

    let capture = tempfile::tempdir().unwrap();
    std::fs::hard_link(&outside_manifest, capture.path().join(crate::MANIFEST_FILE)).unwrap();
    let Err(error) = crate::ManifestOwner::open(capture.path()) else {
        panic!("a manifest with a second name is refused");
    };
    assert!(error.to_string().contains("second name"), "{error}");
    let _ = crate::recover_interrupted(
        capture.path(),
        "not-invoked",
        "not-invoked",
        crate::OwnerState::Dead,
    );
    assert_eq!(std::fs::read(&outside_manifest).unwrap(), before);
}

#[test]
fn a_file_with_a_second_name_is_reported_as_such() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one");
    std::fs::write(&one, b"x").unwrap();
    assert!(crate::has_single_name(&std::fs::File::open(&one).unwrap()).unwrap());
    std::fs::hard_link(&one, dir.path().join("two")).unwrap();
    assert!(!crate::has_single_name(&std::fs::File::open(&one).unwrap()).unwrap());
}
