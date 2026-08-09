use super::*;

#[test]
fn normal_completion_suppresses_restart_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("screen_record");
    let capture = cache.join("cap");
    std::fs::create_dir_all(&capture).unwrap();
    begin(&capture, "cap").unwrap();
    let source = capture.join("source.mp4");
    std::fs::write(&source, b"normal output").unwrap();
    complete(&capture, &source).unwrap();
    let result = scan(&cache, "missing-ffmpeg", "missing-ffprobe");
    assert!(result.recovered.is_empty());
    assert!(result.deferred.is_empty());
    assert!(result.failed_closed.is_empty());
}

#[test]
fn sealed_project_repairs_the_missing_complete_receipt_without_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("screen_record");
    let capture = cache.join("cap");
    begin(&capture, "cap").unwrap();
    let source = capture.join("source.mp4");
    std::fs::write(&source, b"already backend-verified source").unwrap();
    let project = serde_json::json!({ "source_video": source });
    record_recovery::replace_synced(
        &capture.join("project.json"),
        &serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let result = scan(&cache, "missing-ffmpeg", "missing-ffprobe");
    assert!(result.recovered.is_empty());
    assert!(result.deferred.is_empty());
    assert!(result.failed_closed.is_empty());
    assert!(matches!(
        read_manifest(&capture).unwrap().receipt.unwrap().state,
        RecoveryState::Complete
    ));
}

#[test]
fn sealed_normal_project_retries_torn_tail_archive_without_remuxing_source() {
    use std::io::Write;

    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("screen_record");
    let capture = cache.join("cap");
    begin(&capture, "cap").unwrap();
    let source = capture.join("source.mp4");
    let project = capture.join("project.json");
    std::fs::write(&source, b"already verified normal source").unwrap();
    record_recovery::replace_synced(&project, br#"{"source_video":"source.mp4"}"#).unwrap();
    let source_before = std::fs::read(&source).unwrap();
    let project_before = std::fs::read(&project).unwrap();
    let torn = b"{\"kind\":";
    std::fs::OpenOptions::new()
        .append(true)
        .open(capture.join(MANIFEST_FILE))
        .unwrap()
        .write_all(torn)
        .unwrap();
    // Model a power loss after the durable tail archive but before the manifest
    // replacement. A retry must retain normal source/project identity and seal
    // the same Complete receipt rather than remuxing to recovered.mp4.
    std::fs::create_dir(capture.join("quarantine")).unwrap();
    std::fs::write(
        capture.join("quarantine/capture.manifest.torn-tail.jsonl"),
        torn,
    )
    .unwrap();

    let first = scan(&cache, "missing-ffmpeg", "missing-ffprobe");
    assert!(first.recovered.is_empty());
    assert!(first.deferred.is_empty());
    assert!(first.failed_closed.is_empty());
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert_eq!(std::fs::read(&project).unwrap(), project_before);
    assert_eq!(
        std::fs::read(capture.join("quarantine/capture.manifest.torn-tail.jsonl")).unwrap(),
        torn
    );
    assert_eq!(
        read_manifest(&capture).unwrap().receipt.unwrap().state,
        RecoveryState::Complete
    );
    let second = scan(&cache, "missing-ffmpeg", "missing-ffprobe");
    assert!(second.recovered.is_empty());
    assert!(second.deferred.is_empty());
    assert!(second.failed_closed.is_empty());
}

#[test]
fn corrupt_manifest_status_is_path_safe_while_private_file_is_quarantined() {
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("cap");
    std::fs::create_dir_all(&capture).unwrap();
    std::fs::write(capture.join(MANIFEST_FILE), b"not-json\n").unwrap();
    let scan = scan(temp.path(), "missing-ffmpeg", "missing-ffprobe");
    assert_eq!(scan.failed_closed, ["cap: manifest_invalid"]);
    assert!(!scan
        .failed_closed
        .join(" ")
        .contains(&temp.path().display().to_string()));
    assert!(capture
        .join("quarantine/capture.manifest.invalid.jsonl")
        .is_file());
    assert!(!capture.join(MANIFEST_FILE).exists());
}

#[test]
fn status_page_requires_emitted_cursor_and_serializes_snake_case_receipts() {
    let temp = tempfile::tempdir().unwrap();
    for id in ["capture-a", "capture-b"] {
        let root = temp.path().join(id);
        begin(&root, id).unwrap();
        let mut owner = ManifestOwner::open(&root).unwrap();
        owner
            .publish_receipt(RecoveryReceipt {
                state: RecoveryState::Complete,
                recovered_segments: 0,
                lost_tail_ms: Some(0),
                lost_tail_lower_bound_ms: 0,
                lost_tail_upper_bound_ms: Some(0),
                audio_first_packet_offset_ms: None,
                source: Some("source.mp4".into()),
                note: "test".into(),
            })
            .unwrap();
    }
    let first = status_page(temp.path(), None, 1).unwrap();
    assert_eq!(first.captures[0].capture_id, "capture-a");
    assert_eq!(
        first.captures[0]
            .status
            .receipt
            .as_ref()
            .unwrap()
            .source
            .as_deref(),
        Some("source.mp4")
    );
    assert_eq!(
        serde_json::to_value(&first.captures[0]).unwrap()["receipt"]["state"],
        "complete"
    );
    assert!(status_page(temp.path(), Some("capture-never"), 1).is_err());
    let second = status_page(temp.path(), first.next_cursor.as_deref(), 100).unwrap();
    assert_eq!(second.captures[0].capture_id, "capture-b");
}

#[cfg(unix)]
#[test]
fn scan_never_follows_quarantine_or_normal_completion_symlinks() {
    use std::os::unix::fs::symlink;

    let cache = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let corrupt = cache.path().join("corrupt");
    std::fs::create_dir_all(&corrupt).unwrap();
    std::fs::write(corrupt.join(MANIFEST_FILE), b"not-json\n").unwrap();
    symlink(outside.path(), corrupt.join("quarantine")).unwrap();
    let initial = scan(cache.path(), "missing-ffmpeg", "missing-ffprobe");
    assert_eq!(initial.failed_closed, ["corrupt: manifest_invalid"]);
    assert!(corrupt.join(MANIFEST_FILE).is_file());
    assert!(!outside
        .path()
        .join("capture.manifest.invalid.jsonl")
        .exists());

    let sealed = cache.path().join("sealed");
    begin(&sealed, "sealed").unwrap();
    let outside_source = outside.path().join("source.mp4");
    std::fs::write(&outside_source, b"outside remains untouched").unwrap();
    symlink(&outside_source, sealed.join("source.mp4")).unwrap();
    std::fs::write(
        sealed.join("project.json"),
        r#"{"source_video":"source.mp4"}"#,
    )
    .unwrap();
    let second = scan(cache.path(), "missing-ffmpeg", "missing-ffprobe");
    assert!(second.deferred.contains(&"sealed".into()));
    assert!(read_manifest(&sealed).unwrap().receipt.is_none());
    assert_eq!(
        std::fs::read(&outside_source).unwrap(),
        b"outside remains untouched"
    );
    symlink(outside.path(), cache.path().join("linked-capture")).unwrap();
    assert!(status_page(cache.path(), None, 10)
        .unwrap()
        .captures
        .iter()
        .all(|item| {
            item.capture_id != "linked-capture"
                && (item.capture_id != "sealed" || item.status.receipt.is_none())
        }));
}

#[cfg(unix)]
#[test]
fn scan_and_status_never_follow_a_manifest_symlink() {
    use std::os::unix::fs::symlink;

    let cache = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_capture = outside.path().join("outside");
    begin(&outside_capture, "outside").unwrap();
    let outside_manifest = outside_capture.join(MANIFEST_FILE);
    let before = std::fs::read(&outside_manifest).unwrap();
    let capture = cache.path().join("capture");
    std::fs::create_dir(&capture).unwrap();
    symlink(&outside_manifest, capture.join(MANIFEST_FILE)).unwrap();
    std::fs::create_dir(cache.path().join("missing")).unwrap();

    let scan = scan(cache.path(), "missing-ffmpeg", "missing-ffprobe");
    assert!(scan
        .failed_closed
        .contains(&"capture: manifest_invalid".into()));
    assert!(scan
        .failed_closed
        .contains(&"missing: manifest_invalid".into()));
    let page = status_page(cache.path(), None, 10).unwrap();
    assert_eq!(page.captures.len(), 2);
    assert!(page.captures.iter().all(|item| {
        matches!(
            serde_json::to_value(item).unwrap()["state"].as_str(),
            Some("corrupt")
        )
    }));
    assert_eq!(std::fs::read(&outside_manifest).unwrap(), before);
}
