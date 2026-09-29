use super::*;
use crate::ops::{Actor, ActorKind, OpLog, OpStatus};

fn op(sequence: u64) -> OpRecord {
    OpRecord {
        op_id: OpRecord::format_id(sequence),
        ts: "2026-08-08T00:00:00.000Z".into(),
        actor: Actor {
            kind: ActorKind::Agent,
            name: "journal-test".into(),
            via: "test".into(),
            request: None,
        },
        verb: "edit.add_marker".into(),
        args: serde_json::json!({"at_ms": sequence}),
        rationale: None,
        effects: Vec::new(),
        inverse: None,
        status: OpStatus::Applied,
    }
}

fn line(record: &OpRecord, newline: bool) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(record).unwrap();
    if newline {
        bytes.push(b'\n');
    }
    bytes
}

#[test]
fn torn_final_record_is_quarantined_and_next_id_does_not_collide() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let mut bytes = line(&op(0), true);
    let valid_end = bytes.len() as u64;
    let torn = br#"{"op_id":"op_000002","ts":"2026"#;
    bytes.extend_from_slice(torn);
    std::fs::write(&path, &bytes).unwrap();

    let log = OpLog::open(&path).expect("a malformed final record is recoverable");
    let recovery = log.recovery().expect("recovery must be disclosed");
    assert_eq!(recovery.discarded_start, valid_end);
    assert_eq!(recovery.discarded_end, bytes.len() as u64);
    assert_eq!(
        std::fs::read(dir.path().join(&recovery.quarantine_file)).unwrap(),
        torn
    );
    let note: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(&recovery.note_file)).unwrap())
            .unwrap();
    assert_eq!(note["discarded_start"], valid_end);
    assert_eq!(note["discarded_end"], bytes.len() as u64);
    assert_eq!(log.read_all().unwrap(), vec![op(0)]);
    assert_eq!(log.next_id().unwrap(), "op_000002");

    log.append(&op(1)).unwrap();
    assert_eq!(
        log.read_all()
            .unwrap()
            .iter()
            .map(|record| record.op_id.as_str())
            .collect::<Vec<_>>(),
        vec!["op_000001", "op_000002"]
    );
}

#[test]
fn malformed_middle_record_fails_closed_without_changing_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let mut bytes = line(&op(0), true);
    bytes.extend_from_slice(b"{not-json}\n");
    bytes.extend_from_slice(&line(&op(1), true));
    std::fs::write(&path, &bytes).unwrap();

    let error = OpLog::open(&path).unwrap_err();
    assert!(error.message.contains("malformed middle"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "fail-closed open must not create recovery sidecars"
    );
}

#[test]
fn oversized_journal_is_rejected_without_recovery_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let file = File::create(&path).unwrap();
    file.set_len(MAX_JOURNAL_BYTES + 1).unwrap();

    let error = OpLog::open(&path).unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGS);
    assert!(error.message.contains("exceeds the supported size"));
    assert!(error.suggested_action.is_some());
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        MAX_JOURNAL_BYTES + 1
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn oversized_final_record_is_not_treated_as_a_recoverable_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let mut file = File::create(&path).unwrap();
    let valid = line(&op(0), true);
    file.write_all(&valid).unwrap();
    file.set_len(valid.len() as u64 + MAX_RECORD_BYTES as u64 + 1)
        .unwrap();

    let error = OpLog::open(&path).unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGS);
    assert!(error.message.contains("oversized record"));
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        valid.len() as u64 + MAX_RECORD_BYTES as u64 + 1
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn append_refuses_a_record_that_cannot_be_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let log = OpLog::open(&path).unwrap();
    let mut record = op(0);
    record.args = serde_json::json!({"payload": "x".repeat(MAX_RECORD_BYTES)});

    let error = log.append(&record).unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGS);
    assert!(error.message.contains("record exceeds"));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
}

#[test]
fn large_valid_record_still_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let log = OpLog::open(&path).unwrap();
    let mut record = op(0);
    record.rationale = Some("r".repeat(1024 * 1024));
    log.append(&record).unwrap();

    let reopened = OpLog::open(&path).unwrap();
    assert_eq!(reopened.read_all().unwrap(), vec![record]);
}

#[test]
fn canonical_newline_requires_room_within_the_journal_limit() {
    let error = ensure_newline_room(128, 128).unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGS);
    assert!(error.suggested_action.is_some());
    ensure_newline_room(127, 128).unwrap();
}

#[test]
fn complete_final_record_without_newline_is_preserved_before_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    std::fs::write(&path, line(&op(0), false)).unwrap();

    let log = OpLog::open(&path).unwrap();
    assert!(log.recovery().is_none());
    log.append(&op(1)).unwrap();
    assert_eq!(log.read_all().unwrap(), vec![op(0), op(1)]);
}

#[test]
fn append_refuses_an_external_journal_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let log = OpLog::open(&path).unwrap();
    std::fs::write(&path, b" \n").unwrap();

    let error = log.append(&op(0)).unwrap_err();
    assert_eq!(error.code, codes::CONFLICT);
    assert!(error.message.contains("changed outside"));
}

#[cfg(unix)]
#[test]
fn linked_journal_is_rejected_before_tail_recovery_touches_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("unrelated.jsonl");
    let torn = b"{not-json";
    std::fs::write(&outside, torn).unwrap();
    let project = dir.path().join("project.cutproj");
    std::fs::create_dir(&project).unwrap();
    std::os::unix::fs::symlink(&outside, project.join("ops.jsonl")).unwrap();

    assert!(OpLog::open(&project.join("ops.jsonl")).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), torn);
    assert_eq!(std::fs::read_dir(&project).unwrap().count(), 1);
}
