use super::*;

fn write_video(path: &std::path::Path) {
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=30:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(path)
        .status()
        .expect("ffmpeg present (cut-media dependency)");
    assert!(status.success(), "lavfi asset generation failed");
}

async fn create_nested_project(root: &std::path::Path) -> (AppState, std::path::PathBuf) {
    let project_dir = root.join("nested.cutproj");
    let source = root.join("source.mp4");
    write_video(&source);

    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({
            "name": "nested",
            "dir": project_dir,
            "settings": {"width": 160, "height": 90, "fps": 30.0}
        }),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().expect("project open");
        store
            .record_import(
                None,
                cut_core::Asset {
                    path: source.to_string_lossy().into_owned(),
                    hash: "sha256:nest-media-io-fixture".into(),
                    probe: Some(json!({
                        "duration_ms": 1000,
                        "width": 160,
                        "height": 90,
                        "fps": 30.0,
                        "has_audio": true
                    })),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                test_actor(),
                Some("nest media I/O regression fixture".into()),
            )
            .expect("register source asset");
    }

    for track in ["v1", "a1t"] {
        for (at_ms, src_range_ms) in [(0, [0, 400]), (400, [400, 800])] {
            let inserted = dispatch(
                &state,
                "edit.insert",
                json!({
                    "asset": "a1",
                    "track": track,
                    "at_ms": at_ms,
                    "src_range_ms": src_range_ms,
                    "ripple": false
                }),
                test_actor(),
            )
            .await;
            assert!(
                inserted.ok,
                "insert on {track} failed: {:?}",
                inserted.error
            );
        }
    }

    for (clips, name) in [(["c1", "c2"], "intro video"), (["c3", "c4"], "intro audio")] {
        let nested = dispatch(
            &state,
            "edit.nest",
            json!({"clips": clips, "name": name}),
            test_actor(),
        )
        .await;
        assert!(nested.ok, "nest failed: {:?}", nested.error);
    }
    assert!(
        dispatch(&state, "project.save", json!({}), test_actor())
            .await
            .ok
    );
    (state, project_dir)
}

#[tokio::test]
async fn nested_timeline_survives_reopen_across_media_io_without_persisting_bake_asset() {
    let root = tempfile::tempdir().unwrap();
    let (state, project_dir) = create_nested_project(root.path()).await;

    assert!(
        dispatch(&state, "project.close", json!({}), test_actor())
            .await
            .ok
    );
    let reopened = AppState::new();
    let opened = dispatch(
        &reopened,
        "project.open",
        json!({"path": project_dir}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "reopen failed: {:?}", opened.error);

    // The visible Preview requests a composed frame as soon as edit.nest lands,
    // while an agent or release gate can request render.preview at the same time.
    // Both must share the first content-addressed bake instead of colliding on a
    // process-wide temp filename.
    let (concurrent_frame, concurrent_preview) = tokio::join!(
        dispatch(
            &reopened,
            "render.frame",
            json!({"at_ms": 200, "h": 90, "compose": true}),
            test_actor(),
        ),
        dispatch(
            &reopened,
            "render.preview",
            json!({"at_ms": 0, "duration_ms": 300}),
            test_actor(),
        ),
    );
    assert!(
        concurrent_frame.ok,
        "concurrent nested render.frame failed: {:?}",
        concurrent_frame.error
    );
    assert!(
        concurrent_preview.ok,
        "concurrent nested render.preview failed: {:?}",
        concurrent_preview.error
    );

    let frame = dispatch(
        &reopened,
        "render.frame",
        json!({"at_ms": 200, "h": 90, "compose": true}),
        test_actor(),
    )
    .await;
    assert!(frame.ok, "nested render.frame failed: {:?}", frame.error);
    let frame_result = frame.result.expect("frame result");
    assert_eq!(frame_result["width"], 160);
    assert_eq!(frame_result["height"], 90);
    assert!(
        std::fs::metadata(frame_result["path"].as_str().unwrap())
            .map(|meta| meta.len() > 0)
            .unwrap_or(false),
        "nested frame should contain bytes"
    );

    for (verb, args) in [
        ("render.preview", json!({"at_ms": 0, "duration_ms": 300})),
        ("render.preview", json!({"draft": true})),
        (
            "render.storyboard",
            json!({"count": 2, "h": 90, "compose": true}),
        ),
        (
            "export.range",
            json!({"range_ms": [0, 400], "preset": "draft", "to_asset": false}),
        ),
        (
            "export.gif",
            json!({"range_ms": [0, 400], "fps": 6, "width": 160, "to_asset": false}),
        ),
        ("export.audio", json!({"format": "wav", "to_asset": false})),
        ("export.xml", json!({"format": "fcpxml"})),
        ("export.edl", json!({})),
    ] {
        let result = dispatch(&reopened, verb, args, test_actor()).await;
        assert!(result.ok, "nested {verb} failed: {:?}", result.error);
        let output = result.result.expect("media I/O result");
        let output_path = output["path"].as_str().expect("result path");
        assert!(
            std::fs::metadata(output_path)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false),
            "nested {verb} should write a non-empty output"
        );
    }

    let otio = dispatch(&reopened, "export.otio", json!({}), test_actor()).await;
    assert!(otio.ok, "nested export.otio failed: {:?}", otio.error);
    let otio_path = otio.result.unwrap()["path"].as_str().unwrap().to_string();
    let otio_text = std::fs::read_to_string(otio_path).expect("read nested OTIO");
    assert!(
        otio_text.contains("cache/nest") || otio_text.contains("cache\\\\nest"),
        "interchange export should reference the ephemeral flattened nest asset"
    );

    let final_render = dispatch(
        &reopened,
        "render.final",
        json!({
            "preset": "draft",
            "hardware": "off",
            "profile": "silent_screen_demo",
            "path": "exports/nested-final.mp4"
        }),
        test_actor(),
    )
    .await;
    assert!(
        final_render.ok,
        "nested render.final failed to start: {:?}",
        final_render.error
    );
    let render_job_id = final_render.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let render_job = wait_job(&reopened, &render_job_id, 60).await;
    assert_eq!(
        render_job.state,
        crate::jobs::JobState::Done,
        "nested render.final failed: {render_job:?}"
    );
    let render_path = render_job.result.unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        std::fs::metadata(render_path)
            .map(|meta| meta.len() > 0)
            .unwrap_or(false),
        "nested render.final should write a non-empty output"
    );

    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(project_dir.join("project.json")).expect("read persisted project"),
    )
    .expect("parse persisted project");
    assert!(
        persisted["assets"].get("nest1").is_none(),
        "the synthetic baked asset must never be persisted"
    );
    assert!(
        persisted["assets"].get("nest2").is_none(),
        "the synthetic baked audio asset must never be persisted"
    );
    assert_eq!(persisted["nests"][0]["id"], "nest1");
    assert_eq!(persisted["nests"][1]["id"], "nest2");
}

#[tokio::test]
async fn storyboard_common_heights_are_supported_and_scratch_is_clean() {
    let root = tempfile::tempdir().unwrap();
    let (state, project_dir) = create_nested_project(root.path()).await;
    let scratch = project_dir.join("frames/.sb_tmp");

    for height in [90, 120, 150, 180, 200, 240, 360] {
        let result = dispatch(
            &state,
            "render.storyboard",
            json!({"count": 1, "h": height, "compose": true}),
            test_actor(),
        )
        .await;
        assert!(
            !scratch.exists(),
            "storyboard h={height} left its scratch directory after {:?}",
            result.error
        );
        assert!(
            result.ok,
            "storyboard h={height} failed: {:?}",
            result.error
        );
        let output_path = result.result.unwrap()["path"].as_str().unwrap().to_string();
        let output_bytes = std::fs::read(&output_path).expect("read storyboard image");
        assert_eq!(
            jpeg_dimensions(&output_bytes),
            Some((storyboard_tile_width(height), height)),
            "storyboard h={height} should preserve the requested height and safe 16:9 width"
        );
        assert!(
            std::fs::metadata(&output_path)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false),
            "storyboard h={height} should write a non-empty image"
        );
    }
}
