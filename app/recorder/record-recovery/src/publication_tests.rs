//! Focused no-replace publication and native-staging contracts.

use std::fs;
use std::io::Write;

use tempfile::tempdir;

use crate::{
    create_staging_file, publish_new_synced, read_manifest, CaptureStart, CheckpointFacts,
    ManifestOwner, MediaFacts, PrivateStaging,
};

fn facts() -> CheckpointFacts {
    CheckpointFacts {
        start_ms: 0,
        end_ms: 100,
        event_offset_ms: 0,
        audio_offset_ms: None,
    }
}

fn media() -> MediaFacts {
    MediaFacts {
        duration_ms: 100,
        decoded_video_frames: 1,
        has_audio: false,
    }
}

fn staged_file(root: &std::path::Path) -> std::path::PathBuf {
    let (path, mut file) = create_staging_file(root, "wav-test").unwrap();
    file.write_all(b"new complete capture").unwrap();
    file.sync_all().unwrap();
    drop(file);
    path
}

#[test]
fn publication_collision_preserves_existing_final_and_keeps_candidate_private() {
    let root = tempdir().unwrap();
    let part = staged_file(root.path());
    let final_path = root.path().join("system.wav");
    fs::write(&final_path, b"old final capture").unwrap();

    assert!(publish_new_synced(&part, &final_path).is_err());
    assert_eq!(fs::read(&final_path).unwrap(), b"old final capture");
    assert!(part.is_file(), "a collision never destroys the candidate");
}

#[test]
fn publication_moves_fresh_staging_without_replacing() {
    let root = tempdir().unwrap();
    let part = staged_file(root.path());
    let final_path = root.path().join("system.wav");

    publish_new_synced(&part, &final_path).unwrap();
    assert_eq!(fs::read(&final_path).unwrap(), b"new complete capture");
    assert!(
        !part.exists(),
        "successful Windows publication consumes staging"
    );
}

#[cfg(windows)]
fn windows_root_with_utf16_units(temp: &std::path::Path, units: usize) -> std::path::PathBuf {
    use std::os::windows::ffi::OsStrExt;

    let prefix_units = temp.as_os_str().encode_wide().count();
    assert!(units > prefix_units + 1);
    let root = temp.join("p".repeat(units - prefix_units - 1));
    fs::create_dir(&root).unwrap();
    assert_eq!(root.as_os_str().encode_wide().count(), units);
    root
}

/// Exercises a long WGC staging root on a native Windows volume.
/// The WAV stage remains longer than legacy MAX_PATH, so raw `MoveFileExW`
/// must receive extended absolute paths while preserving no-replace publication.
#[cfg(windows)]
#[test]
fn windows_native_long_root_staging_publication() {
    use std::os::windows::ffi::OsStrExt;

    let temp = tempdir().unwrap();
    let root = windows_root_with_utf16_units(temp.path(), 225);
    let (part, mut file) = create_staging_file(&root, "system-wav").unwrap();
    assert!(
        part.as_os_str().encode_wide().count() + 1 > 260,
        "the stage must cross the raw legacy path boundary"
    );
    file.write_all(b"long-root complete capture").unwrap();
    file.sync_all().unwrap();
    drop(file);

    let final_path = root.join("system.wav");
    publish_new_synced(&part, &final_path).unwrap();
    assert_eq!(
        fs::read(&final_path).unwrap(),
        b"long-root complete capture"
    );
    assert!(!part.exists(), "successful publication consumes staging");
}

#[cfg(unix)]
#[test]
fn publication_rejects_staging_and_final_planted_links_without_following_them() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_stage = outside.path().join("outside-stage.wav");
    fs::write(&outside_stage, b"outside staging remains untouched").unwrap();
    let part = staged_file(root.path());
    fs::remove_file(&part).unwrap();
    symlink(&outside_stage, &part).unwrap();
    let final_path = root.path().join("system.wav");
    assert!(publish_new_synced(&part, &final_path).is_err());
    assert_eq!(
        fs::read(&outside_stage).unwrap(),
        b"outside staging remains untouched"
    );
    assert!(!final_path.exists());

    let part = staged_file(root.path());
    let outside_final = outside.path().join("outside-final.wav");
    fs::write(&outside_final, b"outside final remains untouched").unwrap();
    symlink(&outside_final, &final_path).unwrap();
    assert!(publish_new_synced(&part, &final_path).is_err());
    assert_eq!(
        fs::read(&outside_final).unwrap(),
        b"outside final remains untouched"
    );
    assert!(
        part.is_file(),
        "a final collision leaves the candidate recoverable"
    );
}

#[test]
fn checkpoint_native_stage_is_an_absent_private_leaf_with_a_safe_logical_journal_name() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("capture", 100)).unwrap();
    let leaf = owner.begin_segment(0, 0).unwrap();
    let stage_dir = leaf.parent().unwrap().to_path_buf();

    assert_eq!(leaf.file_name().unwrap(), "segment.mp4");
    assert!(
        !leaf.exists(),
        "the native encoder receives an absent output leaf"
    );
    assert!(stage_dir.is_dir());
    assert_ne!(stage_dir, root.path());
    let manifest = fs::read_to_string(root.path().join(crate::MANIFEST_FILE)).unwrap();
    assert!(manifest.contains(".checkpoint-000000.open.mp4"));
    assert!(!manifest.contains(&stage_dir.display().to_string()));

    fs::write(&leaf, b"native encoder output").unwrap();
    let final_path = root.path().join("checkpoints/segment-000000.mp4");
    fs::write(&final_path, b"previous immutable checkpoint").unwrap();
    assert!(owner.publish(0, &leaf, facts(), media()).is_err());
    assert_eq!(
        fs::read(&final_path).unwrap(),
        b"previous immutable checkpoint"
    );
    drop(owner);
    assert!(
        !stage_dir.exists(),
        "a failed known native stage is cleaned without touching the final"
    );
    let reopened = read_manifest(root.path()).unwrap();
    assert_eq!(
        reopened.openings.len(),
        1,
        "the logical open entry survives safely"
    );
}

#[test]
fn private_stage_cleanup_never_recursively_deletes_unexpected_residue() {
    let root = tempdir().unwrap();
    let stage = PrivateStaging::create(root.path(), "native", "segment.mp4").unwrap();
    let stage_dir = stage.path().parent().unwrap().to_path_buf();
    let residue = stage_dir.join("unexpected-native-log.txt");
    fs::write(&residue, b"retain for inspection").unwrap();

    drop(stage);
    assert!(stage_dir.is_dir());
    assert_eq!(fs::read(residue).unwrap(), b"retain for inspection");
}

#[test]
fn checkpoint_publication_is_idempotent_after_one_successful_no_replace_publish() {
    let root = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(root.path(), CaptureStart::new("capture", 100)).unwrap();
    let leaf = owner.begin_segment(0, 0).unwrap();
    let stage_dir = leaf.parent().unwrap().to_path_buf();
    fs::write(&leaf, b"native encoder output").unwrap();

    owner.publish(0, &leaf, facts(), media()).unwrap();
    let final_path = root.path().join("checkpoints/segment-000000.mp4");
    let published = fs::read(&final_path).unwrap();
    assert!(
        !stage_dir.exists(),
        "successful publication removes only the known empty stage"
    );
    assert!(owner.publish(0, &leaf, facts(), media()).is_err());
    assert_eq!(fs::read(&final_path).unwrap(), published);
}
