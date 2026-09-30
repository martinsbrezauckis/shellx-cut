use super::*;

#[test]
fn missing_index_lookup_does_not_create_parent_and_plain_publication_overwrites() {
    let (_root, snapshot) = fixture();
    let parent = snapshot.dir.join("indexes");
    assert!(load_index(&snapshot.dir).is_none());
    assert!(!parent.exists());
    let first = build_fixture_index(&snapshot);
    publish_index(&snapshot, &first).unwrap();
    let mut second = first.clone();
    second.entries.clear();
    let second = finalize_index(second).unwrap();
    publish_index(&snapshot, &second).unwrap();
    assert_eq!(load_index(&snapshot.dir).unwrap().index_id, second.index_id);
    assert!(load_index(&snapshot.dir).unwrap().entries.is_empty());
}

#[cfg(unix)]
#[test]
fn linked_indexes_parent_never_reads_or_publishes_outside() {
    for dangling in [false, true] {
        let (_root, snapshot) = fixture();
        let outside_root = tempfile::tempdir().unwrap();
        let outside = outside_root.path().join("outside");
        let index = build_fixture_index(&snapshot);
        if !dangling {
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(
                outside.join("media-evidence-v1.json"),
                serde_json::to_vec(&index).unwrap(),
            )
            .unwrap();
        }
        let before =
            (!dangling).then(|| std::fs::read(outside.join("media-evidence-v1.json")).unwrap());
        std::os::unix::fs::symlink(&outside, snapshot.dir.join("indexes")).unwrap();
        assert!(
            load_index(&snapshot.dir).is_none(),
            "linked cache is not authority"
        );
        assert!(publish_index(&snapshot, &index).is_err());
        if let Some(before) = before {
            assert_eq!(
                std::fs::read(outside.join("media-evidence-v1.json")).unwrap(),
                before
            );
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        } else {
            assert!(!outside.exists());
        }
    }
}

#[cfg(windows)]
#[test]
fn indexes_directory_reparse_is_not_read_or_published() {
    let (_root, snapshot) = fixture();
    let outside_root = tempfile::tempdir().unwrap();
    let outside = outside_root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let index = build_fixture_index(&snapshot);
    let sentinel = outside.join("media-evidence-v1.json");
    std::fs::write(&sentinel, serde_json::to_vec(&index).unwrap()).unwrap();
    let before = std::fs::read(&sentinel).unwrap();
    match std::os::windows::fs::symlink_dir(&outside, snapshot.dir.join("indexes")) {
        Ok(()) => (),
        Err(error) if error.raw_os_error() == Some(1314) => {
            eprintln!("skipping directory reparse fixture: Windows symlink privilege unavailable");
            return;
        }
        Err(error) => panic!("directory reparse fixture: {error}"),
    }
    assert!(load_index(&snapshot.dir).is_none());
    assert!(publish_index(&snapshot, &index).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), before);
}
