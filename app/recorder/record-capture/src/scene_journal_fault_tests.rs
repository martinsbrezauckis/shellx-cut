#![cfg(unix)]

use std::fs::{self, OpenOptions};
use std::io::Write;

use record_core::scene::ScenePreset;
use record_core::{
    AcceptedStartSnapshot, CatalogRevision, PresetRevision, SceneCatalog, SceneComposition,
    SceneEvent, SceneEventKind, SceneId, TimerConfig,
};
use record_recovery::CaptureRoot;

use crate::scene_journal::{SceneJournalOwner, SCENE_JOURNAL_FILE};
use crate::scene_journal_test_hooks::{inject_once, inject_sequence, SceneJournalIoHook};

fn snapshot() -> AcceptedStartSnapshot {
    let scene = ScenePreset::new(
        SceneId::parse("scene-a").unwrap(),
        "Scene A",
        PresetRevision::new(1).unwrap(),
        SceneComposition::ScreenOnly,
        TimerConfig::Elapsed,
    )
    .unwrap();
    let catalog = SceneCatalog::new(CatalogRevision::new(1).unwrap(), vec![scene]).unwrap();
    AcceptedStartSnapshot::accept(&catalog, &SceneId::parse("scene-a").unwrap()).unwrap()
}

fn event(snapshot: &AcceptedStartSnapshot) -> SceneEvent {
    SceneEvent {
        sequence: 1,
        logical_media_time_ms: 0,
        snapshot_revision: snapshot.revision(),
        kind: SceneEventKind::TimerStart,
    }
}

fn setup(capture_id: &str) -> (tempfile::TempDir, CaptureRoot) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir(capture_id).unwrap();
    (temp, root)
}

fn seeded(root: &CaptureRoot, capture_id: &str) -> std::path::PathBuf {
    let snapshot = snapshot();
    let mut owner = SceneJournalOwner::create_new(root, capture_id, snapshot.clone()).unwrap();
    owner.append_event(event(&snapshot)).unwrap();
    let path = owner.path().to_path_buf();
    drop(owner);
    path
}

#[test]
fn complete_final_record_without_newline_stays_untouched() {
    let (_temp, root) = setup("complete-tail");
    let path = seeded(&root, "complete-tail");
    let mut bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'));
    fs::write(&path, &bytes).unwrap();

    assert!(SceneJournalOwner::open(&root, "complete-tail").is_err());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn arbitrary_malformed_final_bytes_stay_untouched() {
    for (capture_id, tail) in [
        ("bare-tail", b"not-json".as_slice()),
        ("syntax-tail", b"{bad".as_slice()),
    ] {
        let (_temp, root) = setup(capture_id);
        let path = seeded(&root, capture_id);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(tail)
            .unwrap();
        let before = fs::read(&path).unwrap();

        assert!(SceneJournalOwner::open(&root, capture_id).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn retry_after_post_write_or_parent_sync_error_confirms_durability() {
    for (capture_id, hook) in [
        (
            "post-write",
            SceneJournalIoHook::PostWriteBeforeFileSyncFailure,
        ),
        ("parent-sync", SceneJournalIoHook::ParentSyncFailure),
    ] {
        let (_temp, root) = setup(capture_id);
        let snapshot = snapshot();
        let entry = event(&snapshot);
        let mut owner = SceneJournalOwner::create_new(&root, capture_id, snapshot).unwrap();
        inject_once(hook);
        assert!(owner.append_event(entry.clone()).is_err());
        let path = owner.path().to_path_buf();
        drop(owner);

        let mut reopened = SceneJournalOwner::open(&root, capture_id).unwrap();
        let before_retry = fs::read(&path).unwrap();
        reopened.append_event(entry).unwrap();
        assert_eq!(reopened.replayed().events().len(), 1);
        assert_eq!(fs::read(path).unwrap(), before_retry);
    }
}

#[test]
fn direct_open_reacknowledges_post_write_file_and_parent_durability() {
    for (capture_id, hook) in [
        (
            "open-post-write",
            SceneJournalIoHook::PostWriteBeforeFileSyncFailure,
        ),
        ("open-file-sync", SceneJournalIoHook::FileSyncFailure),
        ("open-parent-sync", SceneJournalIoHook::ParentSyncFailure),
    ] {
        let (_temp, root) = setup(capture_id);
        let path = seeded(&root, capture_id);
        let before = fs::read(&path).unwrap();
        inject_once(hook);

        assert!(SceneJournalOwner::open(&root, capture_id).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            SceneJournalOwner::open(&root, capture_id)
                .unwrap()
                .replayed()
                .events()
                .len(),
            1
        );
    }
}

#[test]
fn swap_after_validation_write_or_sync_never_reports_success() {
    for (capture_id, hook) in [
        (
            "swap-validated",
            SceneJournalIoHook::SwapLeafAfterValidation,
        ),
        ("swap-written", SceneJournalIoHook::SwapLeafAfterWrite),
        ("swap-synced", SceneJournalIoHook::SwapLeafAfterSync),
        (
            "swap-capture",
            SceneJournalIoHook::SwapCaptureDirectoryAfterSync,
        ),
    ] {
        let (_temp, root) = setup(capture_id);
        let snapshot = snapshot();
        let mut owner = SceneJournalOwner::create_new(&root, capture_id, snapshot.clone()).unwrap();
        inject_once(hook);

        assert!(owner.append_event(event(&snapshot)).is_err());
        assert_eq!(
            fs::read(owner.path()).unwrap(),
            b"replacement remains authoritative"
        );
    }
}

#[test]
fn read_rejects_growth_and_leaf_replacement() {
    for (capture_id, hook) in [
        ("read-growth", SceneJournalIoHook::GrowDuringRead),
        ("read-swap", SceneJournalIoHook::SwapLeafDuringRead),
    ] {
        let (_temp, root) = setup(capture_id);
        let path = seeded(&root, capture_id);
        inject_once(hook);

        assert!(SceneJournalOwner::replay(&root, capture_id).is_err());
        if hook == SceneJournalIoHook::SwapLeafDuringRead {
            assert_eq!(
                fs::read(path).unwrap(),
                b"replacement remains authoritative"
            );
        }
    }
}

#[test]
fn creation_failures_retain_the_creator_leaf_and_fail_closed() {
    for (capture_id, hook) in [
        ("create-lock", SceneJournalIoHook::CreateLockFailure),
        (
            "create-write",
            SceneJournalIoHook::PostWriteBeforeFileSyncFailure,
        ),
        ("create-sync", SceneJournalIoHook::ParentSyncFailure),
    ] {
        let (_temp, root) = setup(capture_id);
        let path = root.capture_file(capture_id, SCENE_JOURNAL_FILE).unwrap();
        inject_once(hook);

        assert!(SceneJournalOwner::create_new(&root, capture_id, snapshot()).is_err());
        assert!(path.exists());
        assert!(SceneJournalOwner::create_new(&root, capture_id, snapshot()).is_err());
    }
}

#[test]
fn creation_failure_preserves_a_replacement_leaf() {
    let (_temp, root) = setup("create-replacement");
    let path = root
        .capture_file("create-replacement", SCENE_JOURNAL_FILE)
        .unwrap();
    inject_once(SceneJournalIoHook::SwapLeafAfterWrite);

    assert!(SceneJournalOwner::create_new(&root, "create-replacement", snapshot()).is_err());
    assert_eq!(
        fs::read(path).unwrap(),
        b"replacement remains authoritative"
    );
}

#[test]
fn failed_create_preserves_a_swap_after_cleanup_identity_proof() {
    let (_temp, root) = setup("cleanup-proof-swap");
    let path = root
        .capture_file("cleanup-proof-swap", SCENE_JOURNAL_FILE)
        .unwrap();
    inject_sequence(&[
        SceneJournalIoHook::CreateLockFailure,
        SceneJournalIoHook::SwapLeafAfterCleanupProof,
    ]);

    assert!(SceneJournalOwner::create_new(&root, "cleanup-proof-swap", snapshot()).is_err());
    assert_eq!(
        fs::read(path).unwrap(),
        b"replacement remains authoritative"
    );
}
