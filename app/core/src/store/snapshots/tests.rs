use super::*;
use serde_json::json;

fn actor() -> Actor {
    Actor::system()
}

fn create_record() -> OpRecord {
    OpRecord {
        op_id: OpRecord::format_id(0),
        ts: "2026-08-08T00:00:00.000Z".into(),
        actor: actor(),
        verb: "project.create".into(),
        args: json!({"name":"history","settings": ProjectSettings::default()}),
        rationale: None,
        effects: vec![],
        inverse: None,
        status: OpStatus::Applied,
    }
}

fn marker_record(sequence: usize) -> OpRecord {
    let marker = json!({"id": format!("m{sequence}"), "at_ms": sequence, "label": "mark"});
    OpRecord {
        op_id: OpRecord::format_id(sequence as u64),
        ts: "2026-08-08T00:00:00.000Z".into(),
        actor: actor(),
        verb: "edit.add_marker".into(),
        args: json!({"at_ms": sequence, "label": "mark"}),
        rationale: None,
        effects: vec![crate::edit::fx(None, json!({"added_marker": marker}))],
        inverse: None,
        status: OpStatus::Applied,
    }
}

fn marker_log(count: usize) -> Vec<OpRecord> {
    let mut ops = Vec::with_capacity(count + 1);
    ops.push(create_record());
    ops.extend((1..=count).map(marker_record));
    ops
}

fn journal(ops: &[OpRecord]) -> crate::ops::JournalView {
    crate::ops::JournalView::test_from_records(ops.to_vec()).unwrap()
}

#[test]
fn nearest_verified_snapshot_replays_only_the_suffix_with_cold_parity() {
    let temp = tempfile::tempdir().unwrap();
    let ops = marker_log(6);
    let prefix = 4;
    let materialized = rebuild_from_log(&ops[..prefix]).unwrap();
    let prefix_journal = journal(&ops[..prefix]);
    write(temp.path(), &prefix_journal, &materialized).unwrap();

    let full_journal = journal(&ops);
    let (rebuilt, stats) = rebuild(temp.path(), &full_journal, ops.len()).unwrap();
    assert_eq!(rebuilt, rebuild_from_log(&ops).unwrap());
    assert_eq!(stats.snapshot_prefix, prefix);
    assert_eq!(stats.replayed_ops, ops.len() - prefix);
    assert!(!stats.rejected_snapshot);
}

#[test]
fn corrupt_and_stale_snapshots_are_rejected_then_rebuilt_from_the_journal() {
    let temp = tempfile::tempdir().unwrap();
    let mut ops = marker_log(4);
    let materialized = rebuild_from_log(&ops).unwrap();
    let initial_journal = journal(&ops);
    write(temp.path(), &initial_journal, &materialized).unwrap();
    let root = snapshot_dir(temp.path());
    let path = snapshot_path(&root, ops.len());

    std::fs::write(&path, b"not json").unwrap();
    let current_journal = journal(&ops);
    let (rebuilt, corrupt_stats) = rebuild(temp.path(), &current_journal, ops.len()).unwrap();
    assert_eq!(rebuilt, rebuild_from_log(&ops).unwrap());
    assert_eq!(corrupt_stats.snapshot_prefix, 0);
    assert!(corrupt_stats.rejected_snapshot);

    write(temp.path(), &current_journal, &rebuilt).unwrap();
    ops[2].args["label"] = json!("changed in the journal");
    let stale_journal = journal(&ops);
    let (rebuilt, stale_stats) = rebuild(temp.path(), &stale_journal, ops.len()).unwrap();
    assert_eq!(rebuilt, rebuild_from_log(&ops).unwrap());
    assert_eq!(stale_stats.snapshot_prefix, 0);
    assert!(stale_stats.rejected_snapshot);

    write(temp.path(), &stale_journal, &rebuilt).unwrap();
    let (_, repaired_stats) = rebuild(temp.path(), &stale_journal, ops.len()).unwrap();
    assert_eq!(repaired_stats.snapshot_prefix, ops.len());
    assert!(!repaired_stats.rejected_snapshot);
}

#[test]
fn deleting_snapshots_falls_back_to_full_journal_replay() {
    let temp = tempfile::tempdir().unwrap();
    let ops = marker_log(5);
    let materialized = rebuild_from_log(&ops).unwrap();
    let journal = journal(&ops);
    write(temp.path(), &journal, &materialized).unwrap();
    std::fs::remove_dir_all(snapshot_dir(temp.path())).unwrap();

    let (rebuilt, stats) = rebuild(temp.path(), &journal, ops.len()).unwrap();
    assert_eq!(rebuilt, rebuild_from_log(&ops).unwrap());
    assert_eq!(stats.snapshot_prefix, 0);
    assert_eq!(stats.replayed_ops, ops.len());
}

#[test]
fn a_rejected_snapshot_is_healed_from_the_journal_on_open() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(temp.path(), "history", None).unwrap();
    store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 10, "label": "one"}),
            actor(),
            None,
        )
        .unwrap();
    let ops = store.log.read_all().unwrap();
    let journal = store.log.replay_view().unwrap();
    write(&store.dir, &journal, &store.project).unwrap();
    let snapshot = snapshot_path(&snapshot_dir(&store.dir), ops.len());
    std::fs::write(&snapshot, b"corrupted snapshot").unwrap();
    let dir = store.dir.clone();
    drop(store);

    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.markers.len(), 1);
    let healed: Snapshot = serde_json::from_slice(&std::fs::read(snapshot).unwrap()).unwrap();
    assert_eq!(healed.prefix_len, reopened.log.read_all().unwrap().len());
}

#[test]
fn fixed_interval_snapshot_write_uses_the_indexed_durable_journal() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("history.cutproj");
    std::fs::create_dir_all(&dir).unwrap();
    let ops = marker_log(SNAPSHOT_INTERVAL - 1);
    let journal = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).unwrap()))
        .collect::<String>();
    std::fs::write(dir.join("ops.jsonl"), journal).unwrap();
    let log = OpLog::open(&dir.join("ops.jsonl")).unwrap();
    let project = rebuild_from_log(&ops).unwrap();

    write_if_due(&dir, &log, &project);
    assert!(snapshot_path(&snapshot_dir(&dir), ops.len()).is_file());
}

#[test]
fn reconstructed_navigation_survives_a_materialized_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(temp.path(), "history", None).unwrap();
    for at_ms in [10, 20, 30] {
        store
            .apply(
                "edit.add_marker",
                json!({"at_ms": at_ms, "label": format!("m{at_ms}")}),
                actor(),
                None,
            )
            .unwrap();
    }
    store.undo(actor()).unwrap();
    store.undo(actor()).unwrap();
    store.redo(actor()).unwrap();
    let journal = store.log.replay_view().unwrap();
    write(&store.dir, &journal, &store.project).unwrap();
    let dir = store.dir.clone();
    drop(store);

    let mut reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.markers.len(), 2);
    assert!(reopened.undo_available());
    assert!(reopened.redo_available());
    reopened.undo(actor()).unwrap();
    assert_eq!(reopened.project.markers.len(), 1);
    reopened.redo(actor()).unwrap();
    assert_eq!(reopened.project.markers.len(), 2);
}

#[test]
fn post_open_hundred_thousand_operation_undo_and_redo_use_only_indexed_history() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("history.cutproj");
    std::fs::create_dir_all(&dir).unwrap();
    let ops = marker_log(100_000);
    let journal_text = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).unwrap()))
        .collect::<String>();
    std::fs::write(dir.join("ops.jsonl"), journal_text).unwrap();

    let snapshot_prefix = ops.len() / SNAPSHOT_INTERVAL * SNAPSHOT_INTERVAL;
    let snapshot_project = rebuild_from_log(&ops[..snapshot_prefix]).unwrap();
    let snapshot_journal = journal(&ops[..snapshot_prefix]);
    write(&dir, &snapshot_journal, &snapshot_project).unwrap();

    let mut store = ProjectStore::open(&dir).unwrap();
    assert_eq!(store.project.markers.len(), 100_000);

    store.log.reset_replay_metrics();
    reset_test_replay_metrics();
    store.undo(actor()).unwrap();
    let undo_journal = store.log.replay_metrics();
    let undo_replay = test_replay_metrics();
    assert_eq!(store.project.markers.len(), 99_999);
    assert_eq!(undo_replay.rebuilds, 1);
    assert_eq!(undo_replay.replayed_ops, 1_696);
    assert!(undo_replay.replayed_ops < SNAPSHOT_INTERVAL);
    assert_eq!(undo_journal.full_journal_rereads, 0);
    assert_eq!(undo_journal.full_prefix_rehashes, 0);
    assert_eq!(undo_journal.record_vector_cow_clones, 0);
    assert_eq!(undo_journal.prefix_vector_cow_clones, 0);
    assert_eq!(undo_journal.appended_identity_updates, 1);

    store.log.reset_replay_metrics();
    reset_test_replay_metrics();
    store.redo(actor()).unwrap();
    let redo_journal = store.log.replay_metrics();
    let redo_replay = test_replay_metrics();
    assert_eq!(store.project.markers.len(), 100_000);
    assert_eq!(redo_replay.rebuilds, 1);
    assert_eq!(redo_replay.replayed_ops, 1_697);
    assert!(redo_replay.replayed_ops < SNAPSHOT_INTERVAL);
    assert_eq!(redo_journal.full_journal_rereads, 0);
    assert_eq!(redo_journal.full_prefix_rehashes, 0);
    assert_eq!(redo_journal.record_vector_cow_clones, 0);
    assert_eq!(redo_journal.prefix_vector_cow_clones, 0);
    assert_eq!(redo_journal.appended_identity_updates, 1);
}
