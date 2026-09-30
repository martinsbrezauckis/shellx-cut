use super::*;

#[test]
fn stop_diagnostic_reads_bounded_unicode_tail_and_preserves_last_600_characters() {
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        "{}{} \n",
        "previous bytes ".repeat(20_000),
        "é界".repeat(400)
    );
    std::fs::write(dir.path().join("record.log"), text.as_bytes()).unwrap();
    assert_eq!(tail(dir.path()).unwrap(), "é界".repeat(300));
    std::fs::write(dir.path().join("record.log"), b" \n").unwrap();
    assert!(diagnostic(dir.path()).contains("is empty"));
    std::fs::write(dir.path().join("record.log"), b"actual terminal error\n").unwrap();
    assert!(diagnostic(dir.path()).ends_with("actual terminal error"));
}

#[cfg(unix)]
#[test]
fn stop_diagnostic_refuses_linked_logs_without_disclosing_outside_text() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("synthetic-diagnostic.txt");
    std::fs::write(&sentinel, b"outside private diagnostic sentinel").unwrap();
    let log = dir.path().join("record.log");
    symlink(&sentinel, &log).unwrap();
    assert_eq!(diagnostic(dir.path()), "record.log diagnostic unavailable");
    std::fs::remove_file(&log).unwrap();
    std::fs::hard_link(&sentinel, &log).unwrap();
    assert!(tail(dir.path()).is_none());
    std::fs::remove_file(&log).unwrap();
    symlink(outside.path().join("missing"), &log).unwrap();
    assert!(tail(dir.path()).is_none());
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"outside private diagnostic sentinel"
    );
}

#[test]
fn large_sparse_log_never_requires_whole_file_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("record.log");
    let mut file = std::fs::File::create(&path).unwrap();
    file.set_len(1 << 30).unwrap();
    file.seek(SeekFrom::End(-1600)).unwrap();
    use std::io::Write;
    file.write_all("é界".repeat(320).as_bytes()).unwrap();
    assert!(tail(dir.path()).unwrap().ends_with(&"é界".repeat(300)));
}
