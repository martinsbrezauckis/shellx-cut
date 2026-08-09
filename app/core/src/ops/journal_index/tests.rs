use super::*;
use std::time::Instant;

fn record(sequence: u64) -> OpRecord {
    OpRecord {
        op_id: OpRecord::format_id(sequence),
        ts: "2026-08-08T00:00:00.000Z".into(),
        actor: Actor::system(),
        verb: "edit.add_marker".into(),
        args: serde_json::json!({"at_ms": sequence, "label": "index"}),
        rationale: None,
        effects: vec![],
        inverse: None,
        status: OpStatus::Applied,
    }
}

#[test]
fn append_extends_the_prefix_identity_without_rehashing_the_prior_log() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ops.jsonl");
    std::fs::write(&path, b"x").unwrap();
    let first = record(0);
    let mut index = JournalIndex::new(1, &path, RequestIndex::default(), vec![first]).unwrap();
    let before = index.view().prefix_hash(1).unwrap();
    index.reset_metrics();
    let second = record(1);
    let encoded = serde_json::to_vec(&second).unwrap();

    index.record_durable_append(second, &encoded);
    let view = index.view();
    assert_eq!(view.records().len(), 2);
    assert_eq!(view.prefix_hash(1).as_deref(), Some(before.as_str()));
    assert_ne!(view.prefix_hash(2).as_deref(), Some(before.as_str()));
    assert_eq!(index.metrics().full_prefix_rehashes, 0);
    assert_eq!(index.metrics().appended_identity_updates, 1);
}

#[test]
fn metrics_disclose_when_a_retained_view_forces_vector_cow() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ops.jsonl");
    std::fs::write(&path, b"x").unwrap();
    let mut index = JournalIndex::new(1, &path, RequestIndex::default(), vec![record(0)]).unwrap();
    let retained = index.view();
    index.reset_metrics();

    let second = record(1);
    index.record_durable_append(second.clone(), &serde_json::to_vec(&second).unwrap());

    assert_eq!(retained.records().len(), 1);
    assert_eq!(index.metrics().record_vector_cow_clones, 1);
    assert_eq!(index.metrics().prefix_vector_cow_clones, 1);
}

#[test]
fn cursor_page_on_100k_index_is_bounded_and_never_rehashes_the_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ops.jsonl");
    std::fs::write(&path, b"").unwrap();
    let records = (0..100_000).map(record).collect();
    let mut index = JournalIndex::new(100_000, &path, RequestIndex::default(), records).unwrap();
    index.reset_metrics();

    let started = Instant::now();
    let page = index
        .page_after(Some("op_099900"), 100, 128 * 1024)
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(page.ops.len(), 100);
    assert_eq!(page.ops.first().unwrap().op_id, "op_099901");
    assert_eq!(page.ops.last().unwrap().op_id, "op_100000");
    assert!(!page.has_more);
    assert_eq!(page.next_cursor, None);
    assert!(page.encoded_bytes > 0 && page.encoded_bytes <= 128 * 1024);
    let metrics = index.metrics();
    assert_eq!(metrics.paged_records_examined, 100);
    assert_eq!(metrics.full_journal_rereads, 0);
    assert_eq!(metrics.full_prefix_rehashes, 0);
    eprintln!(
        "SYNC_BENCH index_ops=100000 returned_ops={} examined_ops={} encoded_bytes={} elapsed_us={}",
        page.ops.len(),
        metrics.paged_records_examined,
        page.encoded_bytes,
        elapsed.as_micros(),
    );
}
