use super::*;

#[cfg(unix)]
#[test]
fn opened_projection_artifact_rejects_a_leaf_swapped_to_a_link_after_preflight() {
    use std::os::unix::fs::symlink;

    let capture = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let source = capture.path().join("run.mp4");
    let target = outside.path().join("replacement.mp4");
    std::fs::write(&source, b"sealed projection source").unwrap();
    std::fs::write(&target, b"outside replacement").unwrap();

    assert!(is_plain_regular_file(&source).unwrap());
    std::fs::remove_file(&source).unwrap();
    symlink(&target, &source).unwrap();

    // This is the exact post-preflight branch: Unix refuses the no-follow
    // open, while Windows obtains a reparse-point handle and rejects its
    // metadata before any projection reader can consume bytes.
    assert!(open_checked_nofollow(&source).is_err());
    assert!(sha256(&source).is_err());
    assert!(read_nofollow(&source).is_err());
}
