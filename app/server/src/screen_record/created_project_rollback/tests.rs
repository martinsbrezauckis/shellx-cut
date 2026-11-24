use super::reply::path_free_error;
use super::*;
use cut_core::{Actor, Asset, Project};

fn create_tx(root: &Path) -> FallbackProjectTransaction {
    FallbackProjectTransaction::create_at_root(root, Actor::system())
        .expect("create fallback project")
}

#[tokio::test]
async fn clean_fallback_project_with_start_capture_dir_is_removed_and_not_indexed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let state = AppState::new();
    let mut tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    let out_dir = crate::screen_record::create_capture_dir(&proj_dir, "cap-clean-1").unwrap();
    let cadence = crate::screen_record::cadence::from_server_fps(30.0).unwrap();
    crate::screen_record::recovery::begin(&out_dir, "cap-clean-1", &cadence).unwrap();
    crate::screen_record::publish_marker(&proj_dir, "cap-clean-1", b"{}").unwrap();
    assert!(out_dir.join(".capture.json").is_file());
    assert!(out_dir.join("checkpoints").is_dir());
    tx.note_capture_dir_created("cap-clean-1".into());

    let outcome = tx.rollback(&state).await;
    assert_eq!(outcome, RollbackOutcome::Discarded);
    assert!(
        !proj_dir.exists(),
        "fallback project folder must be removed"
    );
    assert!(state.project.read().await.is_none());
}

#[tokio::test]
async fn added_file_in_project_or_capture_cache_leaves_folder_untouched_and_reports_kept() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let state = AppState::new();
    let tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    std::fs::write(proj_dir.join("notes.txt"), "user added").unwrap();

    let outcome = tx.rollback(&state).await;
    assert_eq!(outcome, RollbackOutcome::Kept);
    assert!(proj_dir.join("notes.txt").is_file());
    assert!(proj_dir.join("ops.jsonl").is_file());
    assert!(state.project.read().await.is_some());
}

#[tokio::test]
async fn unexpected_file_in_capture_dir_stops_rollback_before_deleting_anything() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let state = AppState::new();
    let mut tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    let out_dir = crate::screen_record::create_capture_dir(&proj_dir, "cap-extra-1").unwrap();
    std::fs::write(out_dir.join("raw.mp4"), b"media").unwrap();
    tx.note_capture_dir_created("cap-extra-1".into());

    let outcome = tx.rollback(&state).await;
    assert_eq!(outcome, RollbackOutcome::Kept);
    assert!(out_dir.join("raw.mp4").is_file());
    assert!(proj_dir.join("receipts").is_dir());
    assert!(proj_dir.join("ops.jsonl").is_file());
}

#[tokio::test]
async fn modified_assets_or_extra_journal_entry_stops_rollback_and_keeps_project() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let state = AppState::new();
    let tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    let mut pj: Project =
        serde_json::from_slice(&std::fs::read(proj_dir.join("project.json")).unwrap()).unwrap();
    pj.assets.insert(
        "a1".into(),
        Asset {
            path: "/media/clip.mp4".into(),
            hash: "sha256:00".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    std::fs::write(
        proj_dir.join("project.json"),
        serde_json::to_vec(&pj).unwrap(),
    )
    .unwrap();

    // CUT-122LR-03: a folder that no longer matches what Start made is not
    // confirmed, so the outcome is incomplete (unknown), never "kept".
    let outcome = tx.rollback(&state).await;
    assert_eq!(outcome, RollbackOutcome::Incomplete);
    assert!(proj_dir.join("project.json").is_file());
    assert!(state.project.read().await.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn unix_parent_folder_swapped_for_symlink_between_admit_and_remove_leaves_victim_intact() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let victim = temp.path().join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep.txt"), "must not delete").unwrap();
    let moved_out = temp.path().join("moved-away");

    let state = AppState::new();
    let tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    let ops_bytes = std::fs::read(proj_dir.join("ops.jsonl")).unwrap();
    let pj_bytes = std::fs::read(proj_dir.join("project.json")).unwrap();
    // Mirror the victim with identical baseline names so only pinned-handle / no-follow identity catches it.
    std::fs::write(victim.join("ops.jsonl"), ops_bytes).unwrap();
    std::fs::write(victim.join("project.json"), pj_bytes).unwrap();
    std::fs::create_dir_all(victim.join("receipts")).unwrap();
    std::fs::create_dir_all(victim.join("proxies")).unwrap();
    std::fs::create_dir_all(victim.join("filmstrip")).unwrap();

    let swapped_path = proj_dir.clone();
    let victim_clone = victim.clone();
    let outcome = tx
        .rollback_with_hooks(
            &state,
            || {
                std::fs::rename(&swapped_path, &moved_out).unwrap();
                symlink(&victim_clone, &swapped_path).unwrap();
            },
            || {},
        )
        .await;

    assert_eq!(outcome, RollbackOutcome::Incomplete);
    assert!(victim.join("keep.txt").is_file());
    assert!(victim.join("ops.jsonl").is_file());
    assert!(victim.join("receipts").is_dir());
    assert!(
        moved_out.join("ops.jsonl").is_file(),
        "must not unlink moved-away directory outside root either"
    );
    assert!(
        state.project.read().await.is_none(),
        "symlinked folder must never be mounted as the open project"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unix_hardlinked_ops_or_child_symlink_prevents_rollback() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let state = AppState::new();
    let tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    let external_link = temp.path().join("ops-hardlink.jsonl");
    std::fs::hard_link(proj_dir.join("ops.jsonl"), &external_link).unwrap();

    let outcome = tx.rollback(&state).await;
    assert_eq!(outcome, RollbackOutcome::Incomplete);
    assert!(proj_dir.join("ops.jsonl").is_file());

    let tx2 = create_tx(&root);
    let proj2 = tx2.project_dir_path().to_path_buf();
    std::fs::remove_dir(proj2.join("receipts")).unwrap();
    symlink(temp.path(), proj2.join("receipts")).unwrap();
    let outcome2 = tx2.rollback(&state).await;
    assert_eq!(outcome2, RollbackOutcome::Kept);
    assert!(proj2.join("ops.jsonl").is_file());
}

#[cfg(unix)]
#[tokio::test]
async fn unix_mid_baseline_swap_returns_incomplete_without_claiming_kept() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let victim = temp.path().join("victim.json");
    std::fs::write(&victim, "keep-victim").unwrap();
    let state = AppState::new();
    let tx = create_tx(&root);
    let proj_dir = tx.project_dir_path().to_path_buf();
    let pj_path = proj_dir.join("project.json");
    let victim_ref = victim.clone();

    let outcome = tx
        .rollback_with_hooks(
            &state,
            || {},
            || {
                let _ = std::fs::remove_file(&pj_path);
                let _ = symlink(&victim_ref, &pj_path);
            },
        )
        .await;

    assert_eq!(outcome, RollbackOutcome::Incomplete);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep-victim");
    assert!(state.project.read().await.is_none());
}

#[cfg(windows)]
#[tokio::test]
async fn windows_hardlink_and_junction_swap_leave_victim_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Projects");
    let state = AppState::new();

    // Hardlinked ops.jsonl (nNumberOfLinks > 1) must stop removal cleanly.
    let tx1 = create_tx(&root);
    let proj1 = tx1.project_dir_path().to_path_buf();
    std::fs::hard_link(proj1.join("ops.jsonl"), temp.path().join("ops-link.jsonl")).unwrap();
    assert_eq!(tx1.rollback(&state).await, RollbackOutcome::Incomplete);
    assert!(proj1.join("ops.jsonl").is_file());

    // Swapping project dir out and planting an NTFS directory junction between
    // verification and deletion must fail no-follow and leave victim untouched.
    let victim = temp.path().join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep.txt"), "must not delete").unwrap();
    let moved_out = temp.path().join("moved-away");

    let tx2 = create_tx(&root);
    let proj2 = tx2.project_dir_path().to_path_buf();
    let victim_ops = std::fs::read(proj2.join("ops.jsonl")).unwrap();
    let victim_pj = std::fs::read(proj2.join("project.json")).unwrap();
    std::fs::write(victim.join("ops.jsonl"), victim_ops).unwrap();
    std::fs::write(victim.join("project.json"), victim_pj).unwrap();
    std::fs::create_dir_all(victim.join("receipts")).unwrap();
    std::fs::create_dir_all(victim.join("proxies")).unwrap();
    std::fs::create_dir_all(victim.join("filmstrip")).unwrap();

    let proj2_swap = proj2.clone();
    let victim_clone = victim.clone();
    let outcome = tx2
        .rollback_with_hooks(
            &state,
            || {
                std::fs::rename(&proj2_swap, &moved_out).unwrap();
                let status = std::process::Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        &proj2_swap.to_string_lossy(),
                        &victim_clone.to_string_lossy(),
                    ])
                    .status()
                    .expect("create NTFS junction");
                assert!(status.success(), "mklink /J failed");
            },
            || {},
        )
        .await;

    assert_eq!(outcome, RollbackOutcome::Incomplete);
    assert!(victim.join("keep.txt").is_file());
    assert!(victim.join("ops.jsonl").is_file());
    assert!(moved_out.join("ops.jsonl").is_file());
}

#[test]
fn folder_left_after_creation_never_replies_not_created() {
    let error = CutError::new(error_codes::IO, "baseline failed", "x");
    let left = CreateFailure::FolderLeft(error.clone()).into_reply();
    let payload = left.result.unwrap();
    assert_eq!(payload["created_project_outcome"], "incomplete");
    assert_eq!(payload["created_project_kept"], false);
    assert_eq!(payload["created_project_discarded"], false);
    let none = CreateFailure::NoFolder(error).into_reply();
    assert_eq!(
        none.result.unwrap()["created_project_outcome"],
        "not_created"
    );
}

#[test]
fn running_take_reply_carries_capture_id_and_is_path_free() {
    let error = CutError::new(
        error_codes::CONFLICT,
        "changed at /home/u/Projects/a.cutproj",
        "C:\\Users\\u\\Projects\\a.cutproj moved",
    )
    .with_suggested_action("look in /home/u/Projects");
    let reply = running_take_failure_reply(error, "cap-1", None);
    assert!(!reply.ok);
    let payload = reply.result.unwrap();
    assert_eq!(payload["capture_id"], "cap-1");
    assert_eq!(payload["take_running"], true);
    assert_eq!(payload["created_project_outcome"], "incomplete");
    let text = serde_json::to_string(&reply.error).unwrap();
    assert!(!text.contains("/home"), "{text}");
    assert!(!text.contains("Users"), "{text}");
    assert_eq!(reply.error.unwrap().code, error_codes::CONFLICT);
}

#[test]
fn path_free_error_keeps_code_and_clean_text_and_always_has_a_next_step() {
    let clean = CutError::new(error_codes::CONFLICT, "portal cancelled", "user said no")
        .with_suggested_action("try again");
    assert_eq!(path_free_error(&clean), clean);
    let dirty = CutError::new(error_codes::IO, "cannot open /var/x", "~/y failed");
    let out = path_free_error(&dirty);
    assert_eq!(out.code, error_codes::IO);
    assert!(!out.message.contains('/') && !out.cause.contains('/'));
    assert!(out.suggested_action.is_some());
}
