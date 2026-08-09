use std::fs;

use super::CaptureRoot;

#[test]
fn creates_only_literal_capture_components() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();

    let root = CaptureRoot::for_project(&project).unwrap();
    let capture = root.create_capture_dir("cap-safe").unwrap();

    assert_eq!(capture, project.join("cache/screen_record/cap-safe"));
    assert!(capture.is_dir());
    assert!(root.create_capture_dir("../escape").is_err());
}

#[cfg(unix)]
#[test]
fn rejects_ancestor_and_capture_symlinks_without_touching_outside() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();

    symlink(outside.path(), project.join("cache")).unwrap();
    assert!(CaptureRoot::for_project(&project).is_err());
    assert!(!outside.path().join("screen_record").exists());
    fs::remove_file(project.join("cache")).unwrap();

    fs::create_dir(project.join("cache")).unwrap();
    symlink(outside.path(), project.join("cache/screen_record")).unwrap();
    assert!(CaptureRoot::for_project(&project).is_err());
    assert!(!outside.path().join("cap-linked").exists());
    fs::remove_file(project.join("cache/screen_record")).unwrap();

    let root = CaptureRoot::for_project(&project).unwrap();
    symlink(outside.path(), root.cache_dir().join("cap-linked")).unwrap();
    assert!(root.existing_capture_dir("cap-linked").is_err());
    assert!(root.create_capture_dir("cap-linked").is_err());
    assert!(!outside.path().join(".capture.json").exists());
}

#[cfg(unix)]
#[test]
fn marker_publication_rejects_a_link_without_mutating_its_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let sentinel = outside.path().join("sentinel.json");
    fs::write(&sentinel, b"outside marker sentinel").unwrap();

    let root = CaptureRoot::for_project(&project).unwrap();
    let capture = root.create_capture_dir("cap-marker").unwrap();
    symlink(&sentinel, capture.join(".capture.json")).unwrap();

    assert!(root
        .publish_new_capture_file("cap-marker", ".capture.json", b"inside")
        .is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside marker sentinel");
}
