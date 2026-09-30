use super::*;

#[test]
fn normal_plain_cache_files_and_static_hardlink_refusal() {
    let root = tempfile::tempdir().unwrap();
    let dir = cache_dir(root.path(), true).unwrap();
    let plain = dir.join("alpha.mkv");
    std::fs::write(&plain, b"alpha").unwrap();
    assert!(plain_file_exists(&plain).unwrap());
    let outside = root.path().join("outside");
    std::fs::write(&outside, b"sentinel").unwrap();
    let linked = dir.join("linked.mkv");
    std::fs::hard_link(&outside, &linked).unwrap();
    assert!(plain_file_exists(&linked).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"sentinel");
}

#[cfg(windows)]
#[test]
fn windows_reparse_cache_parents_and_dangling_outputs_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    if std::os::windows::fs::symlink_dir(outside.path(), root.path().join("cache")).is_err() {
        // Windows Developer Mode/admin symlink authority is environment-owned.
        return;
    }
    assert!(cache_dir(root.path(), true).is_err());
    std::fs::remove_dir(root.path().join("cache")).unwrap();
    let dir = cache_dir(root.path(), true).unwrap();
    let dangling = dir.join("dangling.mkv");
    std::os::windows::fs::symlink_file(root.path().join("absent"), &dangling).unwrap();
    assert!(plain_file_exists(&dangling).is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}
