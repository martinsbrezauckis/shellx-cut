use super::*;

#[test]
fn new_project_reports_verified_cache_and_no_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "health", None).unwrap();
    let health = store.open_health();
    assert_eq!(health.cache, ProjectCacheHealth::Matched);
    assert_eq!(health.snapshot, ProjectSnapshotHealth::NotPresent);
    assert!(health.journal_tail_recovery.is_none());
}

#[test]
fn strict_open_preserves_cache_snapshot_and_tail_recovery_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "health", None).unwrap();
    let project_dir = store.dir.clone();
    drop(store);

    std::fs::remove_file(project_dir.join("project.json")).unwrap();
    let rebuilt = ProjectStore::open(&project_dir).unwrap();
    assert_eq!(rebuilt.open_health().cache, ProjectCacheHealth::Rebuilt);
    assert_eq!(
        rebuilt.open_health().snapshot,
        ProjectSnapshotHealth::NotPresent
    );
    drop(rebuilt);

    let snapshots = project_dir.join(".history-snapshots");
    std::fs::create_dir_all(&snapshots).unwrap();
    std::fs::write(snapshots.join("snapshot-000000000001.json"), b"not JSON").unwrap();
    let rejected = ProjectStore::open(&project_dir).unwrap();
    assert_eq!(
        rejected.open_health().snapshot,
        ProjectSnapshotHealth::Rejected
    );
    drop(rejected);

    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(project_dir.join("ops.jsonl"))
        .unwrap()
        .write_all(b"{\"torn\":")
        .unwrap();
    let recovered = ProjectStore::open(&project_dir).unwrap();
    let tail = recovered
        .open_health()
        .journal_tail_recovery
        .as_ref()
        .expect("torn final line must be quarantined and disclosed");
    assert!(tail.discarded_end > tail.discarded_start);
    assert!(project_dir.join(&tail.quarantine_file).is_file());
    assert!(project_dir.join(&tail.note_file).is_file());
}

#[test]
fn oversized_project_cache_is_skipped_and_rebuilt_from_journal() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "cache-limit", None).unwrap();
    let project_dir = store.dir.clone();
    let expected = store.project.clone();
    drop(store);

    let cache = std::fs::File::create(project_dir.join("project.json")).unwrap();
    cache.set_len(MAX_REPLAY_CACHE_BYTES + 1).unwrap();
    let reopened = ProjectStore::open(&project_dir).unwrap();
    assert_eq!(reopened.open_health().cache, ProjectCacheHealth::Rebuilt);
    assert_eq!(reopened.project, expected);
}

#[test]
fn oversized_snapshot_is_skipped_and_journal_replays() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "snapshot-limit", None).unwrap();
    let project_dir = store.dir.clone();
    let expected = store.project.clone();
    drop(store);

    let snapshots = project_dir.join(".history-snapshots");
    std::fs::create_dir_all(&snapshots).unwrap();
    let snapshot_path = snapshots.join("snapshot-000000000001.json");
    let snapshot = std::fs::File::create(&snapshot_path).unwrap();
    snapshot.set_len(MAX_REPLAY_CACHE_BYTES + 1).unwrap();

    let reopened = ProjectStore::open(&project_dir).unwrap();
    assert_eq!(
        reopened.open_health().snapshot,
        ProjectSnapshotHealth::Rejected
    );
    assert_eq!(reopened.project, expected);
    assert_eq!(
        std::fs::metadata(&snapshot_path).unwrap().len(),
        MAX_REPLAY_CACHE_BYTES + 1,
        "an oversized snapshot must survive replay and cache refresh"
    );
}
