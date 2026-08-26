//! Dispatch output and receipt-path fencing regression tests.
//!
//! These tests cover the synchronous boundary checks around project-owned
//! exports and receipts, before renderer or judge processes are launched.

use super::*;

/// The output-fencing contract: output paths are fenced — traversal, foreign
/// dirs and non-media suffixes are refused.
#[test]
fn output_path_fencing() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("p.cutproj");
    std::fs::create_dir_all(&proj).unwrap();
    // Default path inside the project is fine.
    assert!(fence_output_path(&proj, None, "exports/out.mp4", OutputPathPolicy::MP4).is_ok());
    // Traversal refused.
    assert!(fence_output_path(&proj, Some("../evil.mp4"), "x.mp4", OutputPathPolicy::MP4).is_err());
    // Foreign absolute dir refused.
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("evil.mp4");
    assert!(
        fence_output_path(&proj, outside_file.to_str(), "x.mp4", OutputPathPolicy::MP4).is_err()
    );
    // Overwriting an EXISTING non-export-suffix file refused (the output-fencing contract
    // invariant: a render/export verb must not clobber project data files;
    // PathFence allows CREATING new files of any suffix inside the fence,
    // and refuses only unsafe overwrites).
    std::fs::create_dir_all(proj.join("exports")).unwrap();
    let inside = proj.join("exports/project.json");
    std::fs::write(&inside, b"existing project data file").unwrap();
    assert!(fence_output_path(
        &proj,
        Some(inside.to_str().unwrap()),
        "x.mp4",
        OutputPathPolicy::MP4
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn atomic_output_write_replaces_late_symlink_instead_of_following_it() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("p.cutproj");
    std::fs::create_dir_all(proj.join("exports")).unwrap();
    let final_path = fence_output_path(
        &proj,
        Some("exports/out.srt"),
        "exports/out.srt",
        OutputPathPolicy::SRT,
    )
    .expect("initial fence passes before attacker swap");

    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, b"keep me").unwrap();
    std::os::unix::fs::symlink(&outside, &final_path).unwrap();

    write_output_atomic(&final_path, b"caption export").expect("atomic output write");

    assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
    assert_eq!(std::fs::read(&final_path).unwrap(), b"caption export");
    assert!(
        !std::fs::symlink_metadata(&final_path)
            .unwrap()
            .file_type()
            .is_symlink(),
        "final path should be a regular export file, not the late symlink"
    );
}

#[test]
fn default_output_paths_avoid_existing_files() {
    let _output_dir_guard = crate::output_paths::SESSION_OUTPUT_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::output_paths::set_session_output_dir(None);
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("p.cutproj");
    std::fs::create_dir_all(proj.join("exports")).unwrap();
    let first =
        fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4).unwrap();
    assert!(first.ends_with("recording.mp4"));
    let first_path = first.to_path_buf();
    std::fs::write(&first_path, b"existing recording").unwrap();
    drop(first);
    let second =
        fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4).unwrap();
    assert!(second.ends_with("recording-2.mp4"));
    assert!(!second.exists());
    let explicit = fence_output_path(
        &proj,
        Some(first_path.to_str().unwrap()),
        "exports/recording.mp4",
        OutputPathPolicy::MP4,
    )
    .unwrap();
    assert_eq!(
        explicit.as_ref(),
        first_path,
        "explicit Save As paths stay exact"
    );
    crate::output_paths::set_session_output_dir(None);
}

#[test]
fn resolve_receipt_path_rejects_traversal_render_id() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let outside = dir.path().join("outside.json");
    std::fs::write(&outside, "{}").unwrap();

    let err = resolve_receipt_path(&receipts, Some("../outside"))
        .expect_err("render_id must not escape receipts dir");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
}

#[test]
fn resolve_receipt_path_with_explicit_id_never_falls_back_to_latest() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(receipts.join("render_001.json"), "{}").unwrap();

    let latest = resolve_receipt_path(&receipts, None).expect("latest receipt");
    assert!(latest.ends_with("render_001.json"));
    let err = resolve_receipt_path(&receipts, Some("render_002"))
        .expect_err("explicit missing receipt must stay missing");
    assert_eq!(err.code, error_codes::NOT_FOUND);
}

#[test]
fn reserve_receipt_id_prevents_reuse_before_receipt_exists() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    let (first, _first_marker) = reserve_receipt_id(&receipts, "render").unwrap();
    let (second, _second_marker) = reserve_receipt_id(&receipts, "render").unwrap();
    assert_eq!(first, "render_001");
    assert_eq!(second, "render_002");
    assert_eq!(next_receipt_id_preview(&receipts, "render"), "render_003");
}

#[test]
fn receipt_id_reservation_releases_marker_when_render_exits_without_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    let (_id, marker) = reserve_receipt_id(&receipts, "render").unwrap();
    assert!(marker.is_file(), "receipt reservation marker must exist");

    let guard = ReceiptIdReservation::new(marker.clone());
    drop(guard);

    assert!(
        !marker.exists(),
        "failed or cancelled render must release its receipt reservation"
    );
    assert_eq!(next_receipt_id_preview(&receipts, "render"), "render_001");
}

#[test]
fn attach_judge_to_receipt_rejects_receipt_outside_receipts_dir() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let outside = dir.path().join("outside.json");
    let receipt = cut_core::RenderReceipt {
        render_id: "render_001".into(),
        ts: OpRecord::now_ts(),
        output_path: dir.path().join("render.mp4").display().to_string(),
        output_hash: "sha256:fake".into(),
        duration_ms: 1000,
        preset: "standard".into(),
        at_op: "op_000001".into(),
        checks: vec![],
        pass: false,
        judge: None,
        fix_actions: vec![],
    };
    std::fs::write(&outside, serde_json::to_string_pretty(&receipt).unwrap()).unwrap();
    let state = AppState::new();

    let err = attach_judge_to_receipt(&state, &receipts, &outside, json!({"status":"test"}))
        .expect_err("judge attachment must not write receipts outside receipts dir");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
}
