use super::*;

const NOW_MS: u64 = 2 * CACHE_RETENTION_MS;

#[test]
fn inventory_counts_only_plain_files_without_following_links() {
    let root = tempfile::tempdir().unwrap();
    let proxy = root.path().join("a1.mp4");
    std::fs::write(&proxy, b"proxy").unwrap();
    let referenced = HashSet::new();

    let report = scan_category(root.path(), "proxies", &referenced, Some(NOW_MS));
    assert_eq!(report.files, 1);
    assert_eq!(report.bytes, 5);
    assert_eq!(report.reclaimable_files, 1);
    assert!(
        report.cleanup_blocked,
        "future-dated files block cleanup preview"
    );
    assert_eq!(report.status(), "ready");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&proxy, root.path().join("link")).unwrap();
        let report = scan_category(root.path(), "proxies", &referenced, Some(NOW_MS));
        assert_eq!(report.files, 1);
        assert_eq!(report.bytes, 5);
        assert_eq!(report.skipped_entries, 1);
        assert_eq!(report.status(), "partial");
    }
}

#[test]
fn inventory_does_not_descend_into_unowned_cache_subdirectories() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("other.bin"), b"outside the flat cache").unwrap();

    let report = scan_category(root.path(), "proxies", &HashSet::new(), Some(NOW_MS));
    assert_eq!(report.files, 0);
    assert_eq!(report.bytes, 0);
    assert_eq!(report.skipped_entries, 1);
    assert_eq!(report.status(), "partial");
}

#[test]
fn inventory_marks_only_recognized_unreferenced_outputs_reclaimable() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a1.mp4"), b"kept").unwrap();
    std::fs::write(root.path().join("a2.mp4"), b"orphan").unwrap();
    std::fs::write(root.path().join("foreign.bin"), b"not ours").unwrap();
    let referenced = HashSet::from([OsString::from("a1.mp4")]);

    let report = scan_category(root.path(), "proxies", &referenced, Some(NOW_MS));
    assert_eq!(report.files, 2);
    assert_eq!(report.bytes, 10);
    assert_eq!(report.reclaimable_files, 1);
    assert_eq!(report.reclaimable_bytes, 6);
    assert_eq!(report.skipped_entries, 1);
    assert_eq!(report.status(), "partial");
    assert!(is_generated_cache_name(
        "thumbnails",
        OsStr::new("a7_w0-10000_12x80.jpg")
    ));
    assert!(!is_generated_cache_name(
        "thumbnails",
        OsStr::new("a7_w0-any_12x80.jpg")
    ));
}

#[test]
fn cleanup_preview_separates_aged_from_recent_unreferenced_files() {
    let root = tempfile::tempdir().unwrap();
    let aged = root.path().join("a1.mp4");
    let recent = root.path().join("a2.mp4");
    std::fs::write(&aged, b"aged").unwrap();
    std::fs::write(&recent, b"recent").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&aged)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        .unwrap();
    std::fs::File::options()
        .write(true)
        .open(&recent)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(NOW_MS)),
        )
        .unwrap();

    let report = scan_category(root.path(), "proxies", &HashSet::new(), Some(NOW_MS));
    assert_eq!(report.reclaimable_bytes, 10);
    assert_eq!(report.reclaimable_files, 2);
    assert_eq!(report.aged_unreferenced_bytes, 4);
    assert_eq!(report.aged_unreferenced_files, 1);
}
