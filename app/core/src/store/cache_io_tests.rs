use super::*;

const SENTINEL: &str = r#"{"synthetic_private_value":"outside-project-only"}"#;

fn project_without_probe(base: &Path) -> PathBuf {
    let mut store = ProjectStore::create(base, "cache-control", None).unwrap();
    store
        .record_import(
            Some("a1".into()),
            Asset {
                path: "missing-test-media.mp4".into(),
                hash: "sha256:synthetic".into(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            Actor::system(),
            None,
        )
        .unwrap();
    let dir = store.dir.clone();
    drop(store);
    std::fs::remove_file(dir.join("project.json")).unwrap();
    dir
}

#[test]
fn plain_probe_cache_is_recovered_on_project_open() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project_without_probe(temp.path());
    let probe = json!({"duration_ms": 1200, "streams": []});
    std::fs::write(
        dir.join("receipts/a1.probe.json"),
        serde_json::to_vec(&probe).unwrap(),
    )
    .unwrap();
    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.assets["a1"].probe, Some(probe));
}

#[cfg(unix)]
#[test]
fn linked_probe_cache_cannot_import_outside_json_into_project_state() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project_without_probe(temp.path());
    let outside = temp.path().join("synthetic-private.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    let cache = dir.join("receipts/a1.probe.json");
    std::os::unix::fs::symlink(&outside, &cache).unwrap();
    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.assets["a1"].probe, None);
    assert!(!serde_json::to_string(&reopened.project)
        .unwrap()
        .contains("outside-project-only"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), SENTINEL);
    assert!(std::fs::symlink_metadata(&cache)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(windows)]
#[test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
fn windows_symlink_probe_cache_cannot_import_outside_json_into_project_state() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project_without_probe(temp.path());
    let outside = temp.path().join("synthetic-private.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    let cache = dir.join("receipts/a1.probe.json");
    std::os::windows::fs::symlink_file(&outside, &cache).unwrap();
    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.assets["a1"].probe, None);
    assert!(!serde_json::to_string(&reopened.project)
        .unwrap()
        .contains("outside-project-only"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), SENTINEL);
    assert!(std::fs::symlink_metadata(&cache)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn hardlinked_probe_cache_cannot_import_outside_json_into_project_state() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project_without_probe(temp.path());
    let outside = temp.path().join("synthetic-private.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    std::fs::hard_link(&outside, dir.join("receipts/a1.probe.json")).unwrap();
    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.assets["a1"].probe, None);
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), SENTINEL);
}

#[cfg(unix)]
#[test]
fn selected_project_root_may_be_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let dir = project_without_probe(temp.path());
    let alias = temp.path().join("chosen-alias.cutproj");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    let reopened = ProjectStore::open(&alias).unwrap();
    assert_eq!(reopened.dir, dir.canonicalize().unwrap());
    assert!(reopened.project.assets.contains_key("a1"));
}

#[test]
fn bounded_cache_keeps_missing_plain_and_oversized_outcomes() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache.json");
    assert!(matches!(
        read_bounded_cache(&cache, 2),
        BoundedCacheRead::Unavailable
    ));
    std::fs::write(&cache, b"{}").unwrap();
    assert!(
        matches!(read_bounded_cache(&cache, 2), BoundedCacheRead::Bytes(bytes) if bytes == b"{}")
    );
    assert!(matches!(
        read_bounded_cache(&cache, 1),
        BoundedCacheRead::Oversized
    ));
    assert!(matches!(
        read_bounded_cache(temp.path(), 2),
        BoundedCacheRead::Unavailable
    ));
}

#[test]
fn plain_file_state_separates_missing_plain_and_shared_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("report.json");
    assert_eq!(plain_file_state(&path), PlainFileState::Missing);
    std::fs::write(&path, b"{}").unwrap();
    assert_eq!(plain_file_state(&path), PlainFileState::Plain);
    std::fs::hard_link(&path, temp.path().join("second-name.json")).unwrap();
    assert_eq!(plain_file_state(&path), PlainFileState::NotPlain);
    assert_eq!(plain_file_state(temp.path()), PlainFileState::NotPlain);
}

#[cfg(unix)]
#[test]
fn plain_file_state_refuses_a_link_without_following_it() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    let link = temp.path().join("report.json");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert_eq!(plain_file_state(&link), PlainFileState::NotPlain);
    assert!(matches!(
        read_bounded_cache(&link, 1024),
        BoundedCacheRead::Unavailable
    ));
}

#[test]
fn replace_project_file_replaces_a_shared_name_without_writing_through() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    let path = temp.path().join("report.json");
    std::fs::hard_link(&outside, &path).unwrap();
    replace_project_file(&path, b"{}").unwrap();
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), SENTINEL);
    assert_eq!(std::fs::read(&path).unwrap(), b"{}");
    assert_eq!(plain_file_state(&path), PlainFileState::Plain);
}

#[cfg(unix)]
#[test]
fn replace_project_file_replaces_a_link_without_writing_through() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    let path = temp.path().join("report.json");
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    replace_project_file(&path, b"{}").unwrap();
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), SENTINEL);
    assert_eq!(plain_file_state(&path), PlainFileState::Plain);
}

/// A compressed (WOF) file carries a reparse point that is not a name
/// surrogate, like a cloud-files placeholder kept by a sync client. It is the
/// file itself: admitted, and read with its real content.
#[cfg(windows)]
#[test]
#[ignore = "requires an NTFS volume and compact.exe with /EXE (Windows 10 or later)"]
fn windows_compressed_file_is_read_as_an_ordinary_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("report.json");
    let content = "{\"word\":\"compressible\"} ".repeat(8192).into_bytes();
    std::fs::write(&path, &content).unwrap();
    let status = std::process::Command::new("compact.exe")
        .args(["/C", "/EXE:XPRESS4K"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success(), "compact.exe /EXE failed: {status}");
    let entry = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .unwrap();
    // Windows may hide the compression reparse point from an ordinary open. When
    // it is visible it must be the compression tag, never a link.
    if let Some(tag) = crate::reparse::handle_reparse_tag(&entry).unwrap() {
        assert_eq!(tag, 0x8000_0017, "expected IO_REPARSE_TAG_WOF");
    }
    drop(entry);

    assert_eq!(plain_file_state(&path), PlainFileState::Plain);
    match read_bounded_cache(&path, 4 * 1024 * 1024) {
        BoundedCacheRead::Bytes(bytes) => assert_eq!(bytes, content),
        other => panic!("compressed file was not read: {other:?}"),
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
fn windows_symlink_report_is_not_a_plain_file() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    let link = temp.path().join("report.json");
    std::os::windows::fs::symlink_file(&outside, &link).unwrap();
    assert_eq!(plain_file_state(&link), PlainFileState::NotPlain);
    assert!(matches!(
        read_bounded_cache(&link, 1024),
        BoundedCacheRead::Unavailable
    ));
}
