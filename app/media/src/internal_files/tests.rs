use super::*;

#[test]
fn an_internal_file_is_reused_only_with_one_name() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.mp4");
    assert!(!plain_cache_file_exists(&missing).unwrap());

    let plain = dir.path().join("seg.mp4");
    std::fs::write(&plain, b"segment").unwrap();
    assert!(plain_cache_file_exists(&plain).unwrap());

    // A second name of the same file: refused, with the way out.
    let second = dir.path().join("second.mp4");
    std::fs::hard_link(&plain, &second).unwrap();
    for path in [&plain, &second] {
        let error = plain_cache_file_exists(path).unwrap_err();
        assert_eq!(error.code, error_codes::IO);
        assert!(
            error.message.starts_with("unsafe media cache file"),
            "{error:?}"
        );
        assert!(error.cause.contains("second name"), "{error:?}");
        assert!(error.suggested_action.unwrap().contains("cache"));
    }

    #[cfg(unix)]
    {
        let link = dir.path().join("link.mp4");
        std::os::unix::fs::symlink(&missing, &link).unwrap();
        assert!(plain_cache_file_exists(&link).is_err());
    }
}

#[test]
fn a_private_list_is_a_fresh_file_outside_the_project_and_goes_away() {
    let path = private_list_file(".cut-test-", ".txt", "file 'a.mp4'\n").unwrap();
    let dir = crate::private_work::session_dir().unwrap();
    assert!(
        path.starts_with(&dir),
        "{} not in {}",
        path.display(),
        dir.display()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "file 'a.mp4'\n");
    let kept = path.to_path_buf();
    drop(path);
    assert!(!kept.exists());
}
