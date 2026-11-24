//! Regression coverage of the existing safe-file helpers through the new store.
use cut_perception::analysis_attempts::{load_attempts, AttemptRecorder};

#[test]
fn the_history_is_bounded_and_keeps_the_latest_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut last_id = String::new();
    for _ in 0..40 {
        let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], true);
        let record = recorder.finish(None);
        assert!(record.storage_error.is_none());
        last_id = record.attempt_id;
    }
    let records = load_attempts(dir.path(), "a1", "hash").unwrap();
    assert_eq!(records.len(), 32);
    assert!(records.iter().any(|r| r.attempt_id == last_id));
}

#[cfg(unix)]
#[test]
fn a_linked_ledger_or_receipts_directory_never_writes_outside_the_project() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("sentinel");
    std::fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, project.path().join("a1.analysis-attempts.json")).unwrap();
    let record = AttemptRecorder::new(project.path(), "a1", "hash", &["scenes"], true).finish(None);
    assert!(record.storage_error.is_some());
    assert_eq!(std::fs::read(&target).unwrap(), b"untouched");

    let link = project.path().join("linked-receipts");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(AttemptRecorder::new(&link, "a1", "hash", &["scenes"], true)
        .finish(None)
        .storage_error
        .is_some());
    assert!(!outside.path().join("a1.analysis-attempts.json").exists());
}

#[test]
fn a_shared_ledger_never_writes_the_other_name() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("sentinel");
    std::fs::write(&target, b"untouched").unwrap();
    std::fs::hard_link(&target, project.path().join("a1.analysis-attempts.json")).unwrap();
    assert!(
        AttemptRecorder::new(project.path(), "a1", "hash", &["scenes"], true)
            .finish(None)
            .storage_error
            .is_some()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
}
