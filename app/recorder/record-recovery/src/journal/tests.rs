use super::{read, Entry, OpenCheckpoint, MAX_MANIFEST_BYTES};
use crate::{
    recover_interrupted, CaptureStart, Checkpoint, CheckpointFacts, ManifestError, ManifestOwner,
    OwnerState, MANIFEST_FILE,
};
use std::fs;
use tempfile::tempdir;

fn checkpoint(file: String) -> Checkpoint {
    Checkpoint {
        sequence: 0,
        file,
        bytes: 1,
        sha256: "not-read".into(),
        media: None,
        held_last_frame_ms: None,
        facts: CheckpointFacts {
            start_ms: 0,
            end_ms: 1,
            event_offset_ms: 0,
            audio_offset_ms: None,
        },
    }
}

fn write_journal(root: &std::path::Path, entries: Vec<Entry>) {
    let body = entries
        .into_iter()
        .map(|entry| serde_json::to_string(&entry).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join(MANIFEST_FILE), format!("{body}\n")).unwrap();
}

#[test]
fn oversized_manifest_is_rejected_without_parsing_or_repairing() {
    let root = tempdir().unwrap();
    let path = root.path().join(MANIFEST_FILE);
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_MANIFEST_BYTES + 1)
        .unwrap();
    assert!(
        matches!(read(root.path()), Err(ManifestError::Invalid(detail)) if detail.contains("size limit"))
    );
    assert_eq!(fs::metadata(path).unwrap().len(), MAX_MANIFEST_BYTES + 1);
}

fn start() -> Entry {
    Entry::Start(CaptureStart::new("cap", 100))
}

fn open(staging: String) -> Entry {
    Entry::Open(OpenCheckpoint {
        sequence: 0,
        staging,
        start_ms: 0,
    })
}

fn assert_rejected_before_recovery(root: &std::path::Path, expected: &str) {
    for error in [
        read(root).unwrap_err(),
        recover_interrupted(root, "not-invoked", "not-invoked", OwnerState::Dead).unwrap_err(),
    ] {
        assert!(matches!(error, ManifestError::Corrupt(ref detail) if detail == expected));
    }
    assert!(!root.join("quarantine").exists());
}

#[test]
fn rejects_absolute_traversal_and_nested_checkpoint_paths_before_recovery() {
    let outside = tempdir().unwrap();
    let outside_file = outside.path().join("outside.mp4");
    fs::write(&outside_file, "outside remains untouched").unwrap();
    for file in [
        outside_file.to_string_lossy().into_owned(),
        "checkpoints/../outside.mp4".into(),
        "other/segment-000000.mp4".into(),
    ] {
        let root = tempdir().unwrap();
        write_journal(
            root.path(),
            vec![
                start(),
                open(".checkpoint-000000.open.mp4".into()),
                Entry::Checkpoint(checkpoint(file)),
            ],
        );
        assert_rejected_before_recovery(root.path(), "invalid checkpoint sequence/path");
        assert_eq!(
            fs::read_to_string(&outside_file).unwrap(),
            "outside remains untouched"
        );
    }
}

#[test]
fn rejects_absolute_traversal_and_nested_open_paths_before_recovery() {
    let outside = tempdir().unwrap();
    let outside_file = outside.path().join("outside.mp4");
    fs::write(&outside_file, "outside remains untouched").unwrap();
    for staging in [
        outside_file.to_string_lossy().into_owned(),
        "../outside.open.mp4".into(),
        "nested/.checkpoint-000000.open.mp4".into(),
    ] {
        let root = tempdir().unwrap();
        write_journal(root.path(), vec![start(), open(staging)]);
        assert_rejected_before_recovery(root.path(), "invalid open checkpoint");
        assert_eq!(
            fs::read_to_string(&outside_file).unwrap(),
            "outside remains untouched"
        );
    }
}

#[test]
fn rejects_unsafe_start_capture_id() {
    let root = tempdir().unwrap();
    assert!(ManifestOwner::begin(root.path(), CaptureStart::new("../cap", 100)).is_err());
    let mut start = CaptureStart::new("../cap", 100);
    start.schema = crate::contract::SCHEMA.into();
    write_journal(root.path(), vec![Entry::Start(start)]);
    assert_rejected_before_recovery(root.path(), "invalid or duplicate start");
}

#[cfg(unix)]
#[test]
fn checkpoint_symlink_is_rejected_before_probe_or_quarantine() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_file = outside.path().join("outside.mp4");
    fs::write(&outside_file, "outside remains untouched").unwrap();
    fs::create_dir(root.path().join("checkpoints")).unwrap();
    let link = root.path().join("checkpoints/segment-000000.mp4");
    symlink(&outside_file, &link).unwrap();
    write_journal(
        root.path(),
        vec![
            start(),
            open(".checkpoint-000000.open.mp4".into()),
            Entry::Checkpoint(checkpoint("checkpoints/segment-000000.mp4".into())),
        ],
    );
    let result = recover_interrupted(root.path(), "not-invoked", "not-invoked", OwnerState::Dead)
        .unwrap()
        .unwrap();
    assert_eq!(result.receipt.state, crate::RecoveryState::Interrupted);
    assert!(result.quarantined.is_none());
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_to_string(&outside_file).unwrap(),
        "outside remains untouched"
    );
    assert!(!root.path().join("quarantine").exists());
}
