use super::admit_public_scene_capture_owner;
use super::scene_projection_start::{
    apply_to_edit_plan, completed_receipt_for_stop, publish_projection, receipt_for_autoedit,
    RECORDING_SCENE_RECEIPT_FILE,
};
use record_core::{fixtures, EditPlan, RecordingProject, Settings};
use record_recovery::CaptureRoot;
use std::fs;
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    project_dir: PathBuf,
    capture_dir: PathBuf,
    project_path: PathBuf,
    project: RecordingProject,
    project_bytes: Vec<u8>,
    projection: record_capture::RecordingSceneProjection,
}

fn fixture(capture_id: &str, logical_duration_ms: u64) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project_dir = temp.path().join("recording-scenes.cutproj");
    fs::create_dir(&project_dir).unwrap();
    let root = CaptureRoot::for_project(&project_dir).unwrap();
    let capture_dir = root.create_capture_dir(capture_id).unwrap();
    fs::write(capture_dir.join("source.mp4"), b"sealed screen source").unwrap();
    let project = RecordingProject::new(
        "source.mp4",
        Settings::default(),
        fixtures::generate("click-walkthrough").unwrap(),
    );
    let project_bytes = serde_json::to_vec_pretty(&project).unwrap();
    let mut engine = record_capture::RecordingSceneEngine::create(
        &root,
        capture_id,
        record_capture::RecordingSceneConfig::default_screen()
            .accepted_snapshot()
            .unwrap(),
    )
    .unwrap();
    engine.start_at(0).unwrap();
    engine.finish_at(logical_duration_ms).unwrap();
    let projection = engine.completed_projection_at(logical_duration_ms).unwrap();
    Fixture {
        _temp: temp,
        project_dir,
        capture_dir: capture_dir.clone(),
        project_path: capture_dir.join("project.json"),
        project,
        project_bytes,
        projection,
    }
}

fn publish(fixture: &Fixture, capture_id: &str) {
    publish_projection(
        &fixture.projection,
        &fixture.project_dir,
        capture_id,
        &fixture.capture_dir,
        &fixture.project_path,
        &fixture.project,
        &fixture.project_bytes,
    )
    .unwrap();
}

#[test]
fn receipt_is_create_only_and_precedes_project_visibility() {
    let fixture = fixture("scene-receipt-order", 5_000);
    publish(&fixture, "scene-receipt-order");
    assert!(fixture
        .capture_dir
        .join(RECORDING_SCENE_RECEIPT_FILE)
        .is_file());
    assert!(
        !fixture.project_path.exists(),
        "the receipt must publish before the stop-visible project"
    );
    let wire: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.capture_dir.join(RECORDING_SCENE_RECEIPT_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(wire["capture_id"], "scene-receipt-order");

    fs::write(&fixture.project_path, &fixture.project_bytes).unwrap();
    let receipt = completed_receipt_for_stop(
        &fixture.project_dir,
        "scene-receipt-order",
        &fixture.capture_dir,
    )
    .unwrap()
    .unwrap();
    assert_eq!(receipt.timeline.logical_duration_ms, 5_000);
    assert_eq!(
        receipt.timeline.timer_label_at(1_000).unwrap().as_deref(),
        Some("00:01")
    );

    // Finalization retry accepts the exact immutable receipt but cannot replace it.
    publish(&fixture, "scene-receipt-order");
}

#[test]
fn reopen_and_autoedit_input_reject_source_project_and_projection_tampering() {
    let fixture = fixture("scene-receipt-tamper", 5_000);
    publish(&fixture, "scene-receipt-tamper");
    fs::write(&fixture.project_path, &fixture.project_bytes).unwrap();
    let track = fixture.capture_dir.join("events.json");
    fs::write(&track, b"{}\n").unwrap();
    let receipt_path = fixture.capture_dir.join(RECORDING_SCENE_RECEIPT_FILE);
    let reopened = receipt_for_autoedit(
        &fixture.project_dir,
        &track,
        &receipt_path.display().to_string(),
    )
    .unwrap();
    assert_eq!(reopened.timeline.logical_duration_ms, 5_000);

    fs::write(fixture.capture_dir.join("source.mp4"), b"tampered source").unwrap();
    assert!(completed_receipt_for_stop(
        &fixture.project_dir,
        "scene-receipt-tamper",
        &fixture.capture_dir,
    )
    .is_err());
    fs::write(
        fixture.capture_dir.join("source.mp4"),
        b"sealed screen source",
    )
    .unwrap();

    fs::write(&fixture.project_path, b"tampered project").unwrap();
    assert!(receipt_for_autoedit(
        &fixture.project_dir,
        &track,
        &receipt_path.display().to_string(),
    )
    .is_err());
    fs::write(&fixture.project_path, &fixture.project_bytes).unwrap();

    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    receipt["capture_id"] = serde_json::json!("other-capture");
    fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(receipt_for_autoedit(
        &fixture.project_dir,
        &track,
        &receipt_path.display().to_string(),
    )
    .is_err());

    receipt["capture_id"] = serde_json::json!("scene-receipt-tamper");
    receipt["projection"]["scene"]["active_scene_id"] = serde_json::json!("forged");
    fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(receipt_for_autoedit(
        &fixture.project_dir,
        &track,
        &receipt_path.display().to_string(),
    )
    .is_err());
}

#[test]
fn reopened_receipt_persists_a_timeline_consumed_by_normal_composition() {
    let fixture = fixture("scene-receipt-export", 5_000);
    publish(&fixture, "scene-receipt-export");
    fs::write(&fixture.project_path, &fixture.project_bytes).unwrap();
    let track = fixture.capture_dir.join("events.json");
    fs::write(&track, b"{}\n").unwrap();
    let receipt = receipt_for_autoedit(
        &fixture.project_dir,
        &track,
        &fixture
            .capture_dir
            .join(RECORDING_SCENE_RECEIPT_FILE)
            .display()
            .to_string(),
    )
    .unwrap();

    let mut plan = EditPlan::empty(
        fixture.project.events.screen_w,
        fixture.project.events.screen_h,
        5_000,
        30.0,
    );
    apply_to_edit_plan(&mut plan, &receipt, None).unwrap();
    let reopened_plan: EditPlan =
        serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    reopened_plan.validate().unwrap();
    assert!(reopened_plan.scene_timeline.is_some());

    let source = record_render::draw_desktop(&fixture.project.events);
    let at_zero = record_render::compose_frame(&source, &reopened_plan, 0).unwrap();
    let at_one_second = record_render::compose_frame(&source, &reopened_plan, 1_000).unwrap();
    assert_ne!(at_zero.data(), at_one_second.data());
}

#[test]
fn public_scenes_refuse_prepublished_project_owners_before_native_capture() {
    assert!(admit_public_scene_capture_owner(true, true).is_err());
    assert!(admit_public_scene_capture_owner(false, true).is_ok());
    assert!(admit_public_scene_capture_owner(true, false).is_ok());
}
