//! Native owned-file and adverse read-only transition checks.

use super::*;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_ARCHIVE;

#[test]
fn owned_leaf_read_only_transition_updates_expected_identity() {
    let temp = tempfile::tempdir().unwrap();
    let camera_path = temp.path().join("camera");
    std::fs::create_dir(&camera_path).unwrap();
    let root = open_directory(temp.path(), "open test capture root").unwrap();
    let camera = open_directory(&camera_path, "open test camera directory").unwrap();
    let leaf_path = camera_path.join("owned.mp4");
    let raw = open_new_leaf(&leaf_path).unwrap();
    // SAFETY: the successful CREATE_NEW handle is transferred once to File.
    let file = unsafe { File::from_raw_handle(raw.0 as *mut _) };
    let leaf = identity_for_file(&file, "inspect test leaf").unwrap();
    let mut stage = WindowsNoReplaceCameraStage {
        root,
        camera,
        file,
        leaf,
        artifact_id: String::new(),
        video: String::new(),
        finalized: true,
    };
    stage.verify_anchored_handles().unwrap();
    stage.make_read_only().unwrap();
    stage.verify_anchored_handles().unwrap();
    let retained = identity_for_file(&stage.file, "inspect protected test leaf").unwrap();
    assert_eq!(stage.leaf, retained);
    assert_ne!(retained.attributes & FILE_ATTRIBUTE_READONLY.0, 0);

    // This test owns the leaf. Restore its permission and close the
    // directory handles before checking cleanup of the owned tempdir.
    let mut permissions = stage.file.metadata().unwrap().permissions();
    permissions.set_readonly(false);
    stage.file.set_permissions(permissions).unwrap();
    drop(stage);
    temp.close().unwrap();
}

#[test]
fn transition_rejects_foreign_identity_and_unexpected_attributes() {
    let before = FileIdentity {
        volume: 11,
        index: 27,
        attributes: FILE_ATTRIBUTE_NORMAL.0,
    };
    let after = FileIdentity {
        attributes: FILE_ATTRIBUTE_READONLY.0,
        ..before
    };
    assert!(is_owned_read_only_transition(before, after));
    assert!(!is_owned_read_only_transition(
        before,
        FileIdentity {
            volume: 12,
            ..after
        }
    ));
    assert!(!is_owned_read_only_transition(
        before,
        FileIdentity { index: 28, ..after }
    ));
    assert!(!is_owned_read_only_transition(
        before,
        FileIdentity {
            attributes: after.attributes | FILE_ATTRIBUTE_REPARSE_POINT.0,
            ..after
        }
    ));
    assert!(!is_owned_read_only_transition(
        before,
        FileIdentity {
            attributes: after.attributes | FILE_ATTRIBUTE_DIRECTORY.0,
            ..after
        }
    ));
    assert!(!is_owned_read_only_transition(
        before,
        FileIdentity {
            attributes: after.attributes | FILE_ATTRIBUTE_ARCHIVE.0,
            ..after
        }
    ));
    assert!(!is_owned_read_only_transition(
        before,
        FileIdentity {
            attributes: FILE_ATTRIBUTE_NORMAL.0,
            ..after
        }
    ));
}
