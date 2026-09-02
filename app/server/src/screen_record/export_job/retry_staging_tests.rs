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
        &ExportFormat::Mp4,
    )
    .unwrap();

    let staged = stage_retry_inputs_async(
        project.clone(),
        "op_000001".into(),
        source.clone(),
        plan.clone(),
        audio,
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
