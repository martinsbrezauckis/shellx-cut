//! Request-level admission must happen before queued export/output allocation.
use super::*;
use serde_json::json;

#[tokio::test]
async fn imported_unregistered_webcam_is_refused_before_export_or_warm_polish() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("camera.cutproj");
    let outside = tempfile::tempdir().unwrap();
    let camera = outside.path().join("private.mp4");
    std::fs::write(&camera, b"unregistered private camera sentinel").unwrap();
    let state = AppState::new();
    let actor = cut_core::Actor::system();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"camera","dir":dir}),
        actor.clone(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let cache = crate::screen_record::screen_record_cache_dir(&dir).unwrap();
    let source = dir.join("source.mp4");
    std::fs::write(&source, b"main video bytes").unwrap();
    let plan_path = dir.join("plan.json");
    let plan = crate::screen_record::plan_inputs::tests::camera_plan(&camera);
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let output = dir.join("exports/refused.mp4");
    let result = screen_record_export(
        &state,
        json!({"source":source,"plan":plan_path,"path":output}),
    )
    .await;
    assert!(result.is_err());
    assert!(!output.exists());
    // A preexisting ordinary bake must not bypass media authorization.
    let key = format!(
        "{}_{}_v.mp4",
        cut_core::hash_file(&source).unwrap().replace(':', "_"),
        crate::screen_record::plan_cache_tag(&plan_path).unwrap()
    );
    let baked = cache.join(key);
    std::fs::write(&baked, b"warm bake bytes").unwrap();
    let result = crate::dispatch::dispatch(
        &state,
        "screen_record.polish",
        json!({"source":source,"plan":plan_path}),
        actor,
    )
    .await;
    assert!(!result.ok);
    assert_eq!(std::fs::read(&baked).unwrap(), b"warm bake bytes");
    assert_eq!(
        std::fs::read(&camera).unwrap(),
        b"unregistered private camera sentinel"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn imported_generated_plan_link_is_refused_before_autoedit_write() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("plan.cutproj");
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel.json");
    std::fs::write(&sentinel, b"outside plan sentinel").unwrap();
    let state = AppState::new();
    let actor = cut_core::Actor::system();
    assert!(
        crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name":"plan","dir":dir}),
            actor.clone()
        )
        .await
        .ok
    );
    let cache = crate::screen_record::screen_record_cache_dir(&dir).unwrap();
    let track = dir.join("events.json");
    std::fs::write(
        &track,
        serde_json::to_vec(&json!({"duration_ms":400,"screen_w":160,"screen_h":90,"clicks":[]}))
            .unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&sentinel, cache.join("events.plan.json")).unwrap();
    let result = crate::dispatch::dispatch(
        &state,
        "screen_record.autoedit",
        json!({"track":track}),
        actor,
    )
    .await;
    assert!(!result.ok);
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside plan sentinel");
}

#[cfg(unix)]
#[tokio::test]
async fn polish_rejects_imported_dangling_raw_and_normal_bakes_before_ffmpeg() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("bake.cutproj");
    let outside = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let actor = cut_core::Actor::system();
    assert!(
        crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name":"bake","dir":dir}),
            actor.clone()
        )
        .await
        .ok
    );
    let source = dir.join("source.mp4");
    let plan_path = dir.join("plan.json");
    std::fs::write(&source, b"source bytes").unwrap();
    std::fs::write(
        &plan_path,
        serde_json::to_vec(&record_core::EditPlan::empty(160, 90, 400, 25.0)).unwrap(),
    )
    .unwrap();
    let cache = crate::screen_record::screen_record_cache_dir(&dir).unwrap();
    for (raw, suffix) in [(true, "raw_v"), (false, "v")] {
        let key = format!(
            "{}_{}_{suffix}.mp4",
            cut_core::hash_file(&source).unwrap().replace(':', "_"),
            crate::screen_record::plan_cache_tag(&plan_path).unwrap()
        );
        let target = outside.path().join(format!("{suffix}.mp4"));
        std::os::unix::fs::symlink(&target, cache.join(key)).unwrap();
        let result = crate::dispatch::dispatch(
            &state,
            "screen_record.polish",
            json!({"source":source,"plan":plan_path,"raw":raw}),
            actor.clone(),
        )
        .await;
        assert!(!result.ok);
        assert!(result
            .error
            .unwrap()
            .message
            .contains("unsafe recorder cache"));
        assert!(!target.exists());
    }
}

#[tokio::test]
async fn raw_polish_ignores_unavailable_embedded_camera_and_publishes_owned_media() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("raw.cutproj");
    let state = AppState::new();
    let actor = cut_core::Actor::system();
    assert!(
        crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name":"raw","dir":dir}),
            actor.clone()
        )
        .await
        .ok
    );
    let source = dir.join("source.mp4");
    assert!(std::process::Command::new(cut_media::toolpath::ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=160x90:rate=25:duration=0.4",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p"
        ])
        .arg(&source)
        .status()
        .unwrap()
        .success());
    let plan_path = dir.join("plan.json");
    let plan =
        crate::screen_record::plan_inputs::tests::camera_plan(&dir.join("unavailable-camera.mp4"));
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let result = crate::dispatch::dispatch(
        &state,
        "screen_record.polish",
        json!({"source":source,"plan":plan_path,"raw":true}),
        actor,
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
    let cache = crate::screen_record::screen_record_cache_dir(&dir).unwrap();
    assert!(std::fs::read_dir(cache).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with("_raw_v.mp4")));
}
