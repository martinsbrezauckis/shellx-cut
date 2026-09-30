use super::retry::retry_descriptor;
use super::retry_staging::stage_retry_inputs_async;
use super::ExportFormat;

#[tokio::test]
async fn retry_staging_binds_renderer_inputs_and_cleans_owned_copies() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("retry-staging.cutproj");
    let media = project.join("media");
    let plans = project.join("plans");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::create_dir_all(&plans).unwrap();
    let source = media.join("recording.mp4");
    let plan = plans.join("edit.json");
    std::fs::write(&source, b"accepted recording bytes").unwrap();
    std::fs::write(&plan, b"accepted plan bytes").unwrap();
    let audio = crate::screen_record::export_audio_for_source(&project, &source).unwrap();
    let expected = retry_descriptor(
        &project,
        "op_000001".into(),
        &source,
        &plan,
        &audio,
        &record_core::EditPlan::empty(160, 90, 1000, 25.0),
        &ExportFormat::Mp4,
    )
    .unwrap();

    let staged = stage_retry_inputs_async(
        project.clone(),
        "op_000001".into(),
        source.clone(),
        plan.clone(),
        audio,
        record_core::EditPlan::empty(160, 90, 1000, 25.0),
        ExportFormat::Mp4,
    )
    .await
    .unwrap();
    let stage_paths = staged.paths_for_test();
    assert!(staged.matches_descriptor(&expected));
    assert!(stage_paths.iter().all(|path| path.is_file()));

    // This models a replacement after retry admission but before the queued
    // renderer opens its input. The renderer path stays bound to copied bytes.
    std::fs::write(&source, b"replacement bytes after retry admission").unwrap();
    assert_eq!(
        std::fs::read(&staged.source).unwrap(),
        b"accepted recording bytes"
    );
    assert_eq!(std::fs::read(&staged.plan).unwrap(), b"accepted plan bytes");

    drop(staged);
    assert!(
        stage_paths.iter().all(|path| !path.exists()),
        "retry-owned copies must be removed after queued work releases them"
    );
}

#[tokio::test]
async fn retry_stages_registered_camera_and_preserves_original_plan_identity() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let plan_path = dir.path().join("plan.json");
    let camera = outside.path().join("registered.mp4");
    std::fs::write(&source, b"main source bytes").unwrap();
    std::fs::write(&camera, b"admitted external camera bytes").unwrap();
    let plan = crate::screen_record::plan_inputs::tests::camera_plan(&camera);
    let original = serde_json::to_vec(&plan).unwrap();
    std::fs::write(&plan_path, &original).unwrap();
    let audio = crate::screen_record::export_audio_for_source(dir.path(), &source).unwrap();
    let expected = retry_descriptor(
        dir.path(),
        "op_1".into(),
        &source,
        &plan_path,
        &audio,
        &plan,
        &ExportFormat::Mp4,
    )
    .unwrap();
    let staged = stage_retry_inputs_async(
        dir.path().to_owned(),
        "op_1".into(),
        source.clone(),
        plan_path.clone(),
        audio.clone(),
        plan.clone(),
        ExportFormat::Mp4,
    )
    .await
    .unwrap();
    assert!(staged.matches_descriptor(&expected));
    assert_eq!(std::fs::read(&staged.plan).unwrap(), original);
    let camera_copy = std::path::PathBuf::from(&staged.render_plan.webcam.as_ref().unwrap().source);
    assert_ne!(camera_copy, camera);
    std::fs::write(&camera, b"replacement camera after queue admission").unwrap();
    assert_eq!(
        std::fs::read(&camera_copy).unwrap(),
        b"admitted external camera bytes"
    );
    let changed = retry_descriptor(
        dir.path(),
        "op_1".into(),
        &source,
        &plan_path,
        &audio,
        &plan,
        &ExportFormat::Mp4,
    )
    .unwrap();
    assert_ne!(changed, expected);
    let paths = staged.paths_for_test();
    drop(staged);
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(std::fs::read(&plan_path).unwrap(), original);
}
