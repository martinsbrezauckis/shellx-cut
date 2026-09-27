use super::*;

#[test]
fn oversized_sparse_evidence_index_is_missing_in_status() {
    let (_directory, snapshot) = fixture();
    let path = crate::dispatch::media_intelligence::model::index_path(&snapshot.dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(path)
        .unwrap()
        .set_len(crate::dispatch::media_intelligence::model::MAX_EVIDENCE_INDEX_BYTES + 1)
        .unwrap();

    assert!(load_index(&snapshot.dir).is_none());
    let status =
        status_value_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Legacy).unwrap();
    assert!(status["index_id"].is_null());
}

#[test]
fn evidence_index_writer_refuses_oversize_without_replacing_cache() {
    let (_directory, snapshot) = fixture();
    let index = build_fixture_index(&snapshot);
    let bytes = serde_json::to_vec_pretty(&index).unwrap();
    crate::dispatch::media_intelligence::build::publish_index_with_limit(
        &snapshot,
        &index,
        bytes.len() as u64,
    )
    .unwrap();
    let path = crate::dispatch::media_intelligence::model::index_path(&snapshot.dir);
    let before = std::fs::read(&path).unwrap();
    let error = crate::dispatch::media_intelligence::build::publish_index_with_limit(
        &snapshot,
        &index,
        bytes.len() as u64 - 1,
    )
    .unwrap_err();
    assert!(error.message.contains("byte limit"));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn oversized_sparse_receipt_is_not_hashed_or_parsed() {
    let (_directory, snapshot) = fixture();
    let bindings =
        current_bindings_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Legacy).unwrap();
    assert!(
        bindings[0].transcript_sha256.is_some(),
        "ordinary receipt hashes"
    );
    let path = snapshot.dir.join("receipts/a1.words.json");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(crate::dispatch::media_intelligence::model::MAX_EVIDENCE_RECEIPT_BYTES + 1)
        .unwrap();

    let current =
        current_bindings_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Legacy).unwrap();
    assert!(current[0].transcript_sha256.is_none());
    let index = build_index(
        &snapshot,
        bindings,
        kinds(&["transcript"]),
        crate::jobs::JobCancellation::test_active(),
    )
    .unwrap();
    assert!(index.entries.is_empty());
}
