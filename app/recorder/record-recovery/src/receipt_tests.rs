use std::fs;

use tempfile::tempdir;

use crate::contract::OpenCheckpoint;
use crate::journal::Entry;
use crate::{
    read_manifest, CaptureStart, Checkpoint, CheckpointFacts, ManifestOwner, RecoveryReceipt,
    RecoveryState, MANIFEST_FILE,
};

fn receipt(state: RecoveryState) -> RecoveryReceipt {
    RecoveryReceipt {
        state,
        recovered_segments: 0,
        lost_tail_ms: Some(0),
        lost_tail_lower_bound_ms: 0,
        lost_tail_upper_bound_ms: Some(0),
        audio_first_packet_offset_ms: None,
        source: Some("source.mp4".into()),
        note: "test".into(),
    }
}

#[test]
fn parser_rejects_parseable_impossible_receipts() {
    let cases = [
        receipt(RecoveryState::Recovered),
        RecoveryReceipt {
            recovered_segments: 1,
            source: None,
            ..receipt(RecoveryState::Quarantined)
        },
        RecoveryReceipt {
            recovered_segments: 1,
            source: None,
            ..receipt(RecoveryState::Interrupted)
        },
        RecoveryReceipt {
            source: Some("../escape.mp4".into()),
            ..receipt(RecoveryState::Complete)
        },
    ];
    for receipt in cases {
        let root = tempdir().unwrap();
        let entries = [
            Entry::Start(CaptureStart::new("cap", 100)),
            Entry::Receipt(receipt),
        ];
        let bytes = entries
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        fs::write(root.path().join(MANIFEST_FILE), format!("{bytes}\n")).unwrap();
        assert!(read_manifest(root.path()).is_err());
    }
}

#[test]
fn writer_rejects_terminal_receipt_with_open_segment() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("cap", 100)).unwrap();
    let _open = owner.begin_segment(0, 0).unwrap();
    assert!(owner
        .publish_receipt(receipt(RecoveryState::Complete))
        .is_err());
    drop(owner);
    assert!(read_manifest(root.path()).unwrap().receipt.is_none());
}

#[test]
fn parser_rejects_receipts_that_name_the_wrong_terminal_artifact() {
    let complete = tempdir().unwrap();
    let entries = [
        Entry::Start(CaptureStart::new("cap", 100)),
        Entry::Receipt(RecoveryReceipt {
            source: Some("other.mp4".into()),
            ..receipt(RecoveryState::Complete)
        }),
    ];
    write_entries(complete.path(), &entries);
    assert!(read_manifest(complete.path()).is_err());

    let recovered = tempdir().unwrap();
    let entries = [
        Entry::Start(CaptureStart::new("cap", 100)),
        Entry::Open(OpenCheckpoint {
            sequence: 0,
            staging: ".checkpoint-000000.open.mp4".into(),
            start_ms: 0,
        }),
        Entry::Checkpoint(Checkpoint {
            sequence: 0,
            file: "checkpoints/segment-000000.mp4".into(),
            bytes: 1,
            sha256: "test".into(),
            media: None,
            facts: CheckpointFacts {
                start_ms: 0,
                end_ms: 1,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
        }),
        Entry::Receipt(RecoveryReceipt {
            recovered_segments: 1,
            source: Some("source.mp4".into()),
            ..receipt(RecoveryState::Recovered)
        }),
    ];
    write_entries(recovered.path(), &entries);
    assert!(read_manifest(recovered.path()).is_err());
}

#[test]
fn parser_rejects_receipt_loss_that_contradicts_committed_checkpoint_facts() {
    let root = tempdir().unwrap();
    let entries = [
        Entry::Start(CaptureStart::new("cap", 100)),
        Entry::Open(OpenCheckpoint {
            sequence: 0,
            staging: ".checkpoint-000000.open.mp4".into(),
            start_ms: 0,
        }),
        Entry::Checkpoint(Checkpoint {
            sequence: 0,
            file: "checkpoints/segment-000000.mp4".into(),
            bytes: 1,
            sha256: "test".into(),
            media: None,
            facts: CheckpointFacts {
                start_ms: 0,
                end_ms: 100,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
        }),
        Entry::Receipt(RecoveryReceipt {
            state: RecoveryState::Interrupted,
            recovered_segments: 0,
            lost_tail_ms: Some(0),
            lost_tail_lower_bound_ms: 0,
            lost_tail_upper_bound_ms: Some(0),
            audio_first_packet_offset_ms: None,
            source: None,
            note: "forged zero loss".into(),
        }),
    ];
    write_entries(root.path(), &entries);
    assert!(read_manifest(root.path()).is_err());
}

fn write_entries(root: &std::path::Path, entries: &[Entry]) {
    let body = entries
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    fs::write(root.join(MANIFEST_FILE), format!("{body}\n")).unwrap();
}
