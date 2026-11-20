//! A baked nest is reused from `cache/nest/` only when it is an ordinary file
//! of the project. A received project with a link (or a file shared with
//! another path) at the cached name must not put that file into renders.

use super::nest_media_io::create_nested_project;
use std::path::{Path, PathBuf};

const PRIVATE: &[u8] = b"private-local-video-bytes";

async fn baked_nest_files(state: &crate::state::AppState, dir: &Path) -> Vec<PathBuf> {
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let flat = crate::nest::bake_and_flatten(project, dir).unwrap();
    project
        .nests
        .iter()
        .map(|nest| PathBuf::from(&flat.assets[&nest.id].path))
        .collect()
}

async fn assert_planted_cache_is_not_reused(plant: impl Fn(&Path, &Path)) {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let (state, dir) = create_nested_project(root.path()).await;
    let dir = dir.canonicalize().unwrap();
    let first = baked_nest_files(&state, &dir).await;
    let private = root.path().join("private.mp4");
    std::fs::write(&private, PRIVATE).unwrap();
    for baked in &first {
        std::fs::remove_file(baked).unwrap();
        plant(&private, baked);
    }

    let second = baked_nest_files(&state, &dir).await;

    assert_eq!(std::fs::read(&private).unwrap(), PRIVATE);
    for baked in &second {
        let metadata = std::fs::symlink_metadata(baked).unwrap();
        assert!(
            metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
            "{baked:?} is still the planted link"
        );
        assert_ne!(std::fs::read(baked).unwrap(), PRIVATE, "{baked:?}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn linked_nest_cache_is_rebaked_not_rendered() {
    assert_planted_cache_is_not_reused(|private, baked| {
        std::os::unix::fs::symlink(private, baked).unwrap()
    })
    .await;
}

#[tokio::test]
async fn shared_nest_cache_is_rebaked_not_rendered() {
    assert_planted_cache_is_not_reused(|private, baked| {
        std::fs::hard_link(private, baked).unwrap()
    })
    .await;
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
async fn windows_symlink_nest_cache_is_rebaked_not_rendered() {
    assert_planted_cache_is_not_reused(|private, baked| {
        std::os::windows::fs::symlink_file(private, baked).unwrap()
    })
    .await;
}

/// Item 155: an ORDINARY file planted under the cached name (a received
/// project can carry any picture there; the name is a hash of project.json
/// data) is not reused either: only a bake this engine made is.
#[tokio::test]
async fn ordinary_planted_nest_cache_is_rebaked_not_rendered() {
    assert_planted_cache_is_not_reused(|private, baked| {
        std::fs::copy(private, baked).unwrap();
    })
    .await;
}
