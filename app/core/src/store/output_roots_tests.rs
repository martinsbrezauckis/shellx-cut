//! Project admission must precede cache, journal and directory mutations.

use super::ProjectStore;
use crate::error_codes;

#[cfg(unix)]
#[test]
fn opening_project_rejects_linked_output_roots_before_any_side_effect() {
    for relative in [
        "jobs",
        "jobs/quarantine",
        "request-receipts",
        "indexes",
        "dub",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = ProjectStore::create(root.path(), "jobs-boundary", None).unwrap();
        let project_dir = store.dir.clone();
        drop(store);
        let outside = root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("preferences.json"), b"{\"keep\":true}").unwrap();
        let link = project_dir.join(relative);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        // Opening would ordinarily rebuild this cache and create receipts.
        std::fs::write(project_dir.join("project.json"), b"invalid cache").unwrap();
        std::fs::remove_dir(project_dir.join("receipts")).unwrap();
        let journal = std::fs::read(project_dir.join("ops.jsonl")).unwrap();

        let error = ProjectStore::open(&project_dir).err().unwrap();
        assert_eq!(error.code, error_codes::INVALID_ARGS, "{relative}");
        assert_eq!(
            std::fs::read(project_dir.join("project.json")).unwrap(),
            b"invalid cache"
        );
        assert_eq!(
            std::fs::read(project_dir.join("ops.jsonl")).unwrap(),
            journal
        );
        assert!(!project_dir.join("receipts").exists());
        assert_eq!(
            std::fs::read(outside.join("preferences.json")).unwrap(),
            b"{\"keep\":true}"
        );
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    }
}

#[test]
fn opening_project_accepts_absent_and_plain_output_roots() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "jobs-boundary", None).unwrap();
    let project_dir = store.dir.clone();
    let expected = store.project.clone();
    drop(store);
    assert_eq!(ProjectStore::open(&project_dir).unwrap().project, expected);
    std::fs::create_dir_all(project_dir.join("jobs/quarantine")).unwrap();
    std::fs::create_dir(project_dir.join("request-receipts")).unwrap();
    std::fs::create_dir(project_dir.join("indexes")).unwrap();
    std::fs::create_dir(project_dir.join("dub")).unwrap();
    assert_eq!(ProjectStore::open(&project_dir).unwrap().project, expected);
}

#[test]
fn opening_project_rejects_non_directory_output_roots() {
    for relative in [
        "jobs",
        "jobs/quarantine",
        "request-receipts",
        "indexes",
        "dub",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = ProjectStore::create(root.path(), "jobs-boundary", None).unwrap();
        let project_dir = store.dir.clone();
        drop(store);
        let path = project_dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"keep").unwrap();
        let error = ProjectStore::open(&project_dir).err().unwrap();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
}

#[cfg(unix)]
#[test]
fn opening_project_rejects_dangling_request_receipt_root() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "receipt-boundary", None).unwrap();
    let project_dir = store.dir.clone();
    drop(store);
    let outside = root.path().join("absent-outside");
    std::os::unix::fs::symlink(&outside, project_dir.join("request-receipts")).unwrap();
    let journal = std::fs::read(project_dir.join("ops.jsonl")).unwrap();
    assert_eq!(
        ProjectStore::open(&project_dir).err().unwrap().code,
        error_codes::INVALID_ARGS
    );
    assert!(!outside.exists());
    assert_eq!(
        std::fs::read(project_dir.join("ops.jsonl")).unwrap(),
        journal
    );
}

#[cfg(unix)]
#[test]
fn opening_project_rejects_dangling_indexes_root() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "index-boundary", None).unwrap();
    let project_dir = store.dir.clone();
    drop(store);
    let outside = root.path().join("absent-outside");
    std::os::unix::fs::symlink(&outside, project_dir.join("indexes")).unwrap();
    assert_eq!(
        ProjectStore::open(&project_dir).err().unwrap().code,
        error_codes::INVALID_ARGS
    );
    assert!(!outside.exists());
}
