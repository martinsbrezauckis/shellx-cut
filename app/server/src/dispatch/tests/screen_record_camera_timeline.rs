async fn camera_timeline_fixture(
    mode: &str,
    fallback: bool,
) -> (tempfile::TempDir, AppState, serde_json::Value) {
    camera_timeline_fixture_with_plan(mode, fallback, None).await
}

async fn camera_timeline_fixture_with_plan(
    mode: &str,
    fallback: bool,
    build: Option<fn(&mut record_core::EditPlan, &Path)>,
) -> (tempfile::TempDir, AppState, serde_json::Value) {
    camera_timeline_fixture_duration(mode, fallback, build, 3000).await
}

async fn camera_timeline_fixture_duration(
    mode: &str,
    fallback: bool,
    build: Option<fn(&mut record_core::EditPlan, &Path)>,
    duration_ms: u64,
) -> (tempfile::TempDir, AppState, serde_json::Value) {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("camera.cutproj");
    assert!(
        dispatch(
            &state,
            "project.create",
            json!({"name":"camera","dir":dir}),
            test_actor()
        )
        .await
        .ok
    );
    let capture = dir.join("cache/screen_record/timeline");
    std::fs::create_dir_all(capture.join("camera")).unwrap();
    for (name, colour) in [("source.mp4", "green"), ("camera/camera.mp4", "red")] {
        assert!(std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "color=c={colour}:size=640x360:rate=30:duration={}",
                    duration_ms as f64 / 1000.0
                ),
                "-pix_fmt",
                "yuv420p",
                "-y"
            ])
            .arg(capture.join(name))
            .status()
            .unwrap()
            .success());
    }
    let camera = capture.join("camera/camera.mp4");
    let artifact = json!({"schema":"shellx-record/camera-artifact/1","capture_id":"timeline","artifact_id":"camera","video":"camera/camera.mp4","clock":{"first_frame_offset_ms":0,"end_frame_offset_ms":duration_ms},"media":{"width":640,"height":360,"fps_num":30,"fps_den":1,"frame_count":duration_ms*30/1000,"duration_ms":duration_ms,"sha256":format!("{:x}",Sha256::digest(std::fs::read(&camera).unwrap()))},"terminal_state":"complete"});
    std::fs::write(
        capture.join("project.json"),
        serde_json::to_vec(
            &json!({"source_video":capture.join("source.mp4"),"camera_artifact":artifact}),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(capture.join("events.json"), b"{}").unwrap();
    let mut plan = record_core::EditPlan::empty(640, 360, duration_ms, 30.0);
    if let Some(build) = build {
        build(&mut plan, &camera);
    } else if !fallback {
        plan.webcam = Some(serde_json::from_value(json!({"source":camera,"shape":{"shape":"circle"},"anchor":"bottom_right","margin":0.05,"size":0.3,"timeline":[{"t_ms":500,"visible":false},{"t_ms":1000,"visible":true},{"t_ms":1500,"size":0.5},{"t_ms":2000,"size":0.2},{"t_ms":2250,"shape":{"shape":"rounded_rect","radius":12.0}}]})).unwrap());
    }
    let path = capture.join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let reply = dispatch(
        &state,
        "screen_record.polish",
        json!({"source":capture.join("source.mp4"),"plan":path,"camera":mode}),
        test_actor(),
    )
    .await;
    assert!(reply.ok, "{:?}", reply.error);
    (root, state, reply.result.unwrap())
}

async fn camera_copy_refresh_case() {
    let (root, state, reply) = camera_timeline_fixture("separate", false).await;
    let original = root.path().join("camera.cutproj");
    let destination = root.path().join("copies");
    std::fs::create_dir(&destination).unwrap();
    let plan = dispatch(
        &state,
        "project.package_plan",
        json!({"destination":destination,"name":"copy"}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let create = dispatch(&state,"project.package_create",json!({"destination":destination,"name":"copy","plan_hash":plan.result.unwrap()["plan_hash"]}),test_actor()).await;
    assert!(create.ok, "{:?}", create.error);
    camera_wait_job(&state, create.result.unwrap()["job_id"].as_str().unwrap()).await;
    close_camera_test_project_after_jobs_drain(&state, "before source rename").await;
    std::fs::rename(&original, root.path().join("original-unavailable")).unwrap();
    assert!(!original.exists());
    let copy = destination.join("copy.cutproj");
    let state = AppState::new();
    let opened = dispatch(&state, "project.open", json!({"path":copy}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);
    let clip = reply["clip_id"].as_str().unwrap();
    let add = dispatch(
        &state,
        "screen_record.zoom_add",
        json!({"clip":clip,"at_ms":500,"duration_ms":2000}),
        test_actor(),
    )
    .await;
    assert!(add.ok, "{:?}", add.error);
    let refresh = dispatch(
        &state,
        "screen_record.take_refresh",
        json!({"clip":clip}),
        test_actor(),
    )
    .await;
    assert!(refresh.ok, "{:?}", refresh.error);
    camera_wait_job(&state, refresh.result.unwrap()["job_id"].as_str().unwrap()).await;
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let take = &store.project.recordings[0];
    assert!(!take.stale());
    // Compare resolved paths: the temporary folder is a link on macOS and the
    // project root is resolved differently on Windows.
    let refreshed = Path::new(&store.project.assets[&take.asset].path)
        .canonicalize()
        .unwrap();
    assert!(refreshed.starts_with(copy.canonicalize().unwrap()));
    assert!(!original.exists());
}

async fn camera_wait_job(state: &AppState, id: &str) {
    for _ in 0..600 {
        let job = state.jobs.get(id).unwrap();
        if job.state == crate::jobs::JobState::Done {
            return;
        }
        assert_ne!(job.state, crate::jobs::JobState::Failed, "{:?}", job.error);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("camera job did not finish");
}

#[test]
fn project_copy_keeps_real_zoom_refresh_without_original_folder() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(camera_copy_refresh_case());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn camera_timeline_pixels_case() {
    let (_root, separate, reply) = camera_timeline_fixture("separate", false).await;
    let (_broot, _baked, baked_reply) = camera_timeline_fixture("baked", false).await;

    assert!(reply["camera_note"]
        .as_str()
        .unwrap()
        .contains("first shape"));
    let guard = separate.project.read().await;
    let store = guard.as_ref().unwrap();
    for at in [250, 750, 1250, 1750, 2125] {
        let jpeg = cut_media::render::extract_frame(
            &store.project,
            &cut_core::edl_from_project(&store.project),
            &store.dir,
            at,
            None,
        )
        .unwrap();
        let frame = store.dir.join(format!("frame-{at}.jpg"));
        if let Ok(prefix) = std::env::var("CAMERA_R2_IMAGE_PREFIX") {
            std::fs::write(format!("{prefix}-{at}.jpg"), &jpeg).unwrap();
        }
        std::fs::write(&frame, jpeg).unwrap();
        let actual = camera_test_rgb(&frame, 0.0);
        let expected = camera_test_rgb(
            Path::new(baked_reply["baked"].as_str().unwrap()),
            at as f64 / 1000.0,
        );
        let bounds = |rgb: &[u8]| {
            let mut bounds: Option<(usize, usize, usize, usize)> = None;
            for y in 0..360 {
                for x in 0..640 {
                    let i = (y * 640 + x) * 3;
                    if (rgb[i] > 180 && rgb[i + 1] < 40)
                        || (rgb[i] > 170 && rgb[i + 1] > 170 && rgb[i + 2] > 170)
                    {
                        bounds = Some(bounds.map_or((x, y, x, y), |(a, b, c, d)| {
                            (a.min(x), b.min(y), c.max(x), d.max(y))
                        }));
                    }
                }
            }
            bounds
        };
        match (bounds(&actual), bounds(&expected)) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                for (a, b) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
                    assert!(
                        a.abs_diff(b) <= 4,
                        "outer bubble bounds differ at {at}: {a} vs {b}"
                    );
                }
            }
            _ => panic!("camera presence differs at {at}"),
        }
        // The overlay ring is inset, unlike the recorder's centred stroke.
        // Compare colours away from that anti-aliased border, plus outer bounds.
        let (cx, cy, r) = if at < 1500 {
            (568.0, 288.0, 54.0)
        } else if at < 2000 {
            (550.0, 270.0, 90.0)
        } else {
            (550.0, 270.0, 36.0)
        };
        for y in (4..356).step_by(4) {
            for x in (4..636).step_by(4) {
                if (((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() - r).abs() < 10.0 {
                    continue;
                }
                let i = (y * 640 + x) * 3;
                let bubble = |rgb: &[u8]| rgb[i] > 180; // red camera OR white ring
                assert_eq!(
                    bubble(&actual),
                    bubble(&expected),
                    "bubble presence/diameter at {at}ms pixel {x},{y}"
                );
            }
        }
    }
    let jpeg = cut_media::render::extract_frame(
        &store.project,
        &cut_core::edl_from_project(&store.project),
        &store.dir,
        2500,
        None,
    )
    .unwrap();
    let frame = store.dir.join("first-shape-kept.jpg");
    std::fs::write(&frame, jpeg).unwrap();
    let separate_shape = camera_test_rgb(&frame, 0.0);
    let baked_shape = camera_test_rgb(Path::new(baked_reply["baked"].as_str().unwrap()), 2.5);
    let corner = (240 * 640 + 520) * 3;
    assert!(
        separate_shape[corner] < 40 && separate_shape[corner + 1] > 80,
        "separate keeps the first circle after the recorded shape key: {:?}",
        &separate_shape[corner..corner + 3]
    );
    assert!(
        baked_shape[corner] > 180 && baked_shape[corner + 1] < 40,
        "baked mode still applies the rounded shape key: {:?}",
        &baked_shape[corner..corner + 3]
    );
    let track = reply["camera_track_id"].as_str().unwrap();
    drop(guard);
    let split = dispatch(
        &separate,
        "edit.split",
        json!({"track":track,"at_ms":1750}),
        test_actor(),
    )
    .await;
    assert!(split.ok, "{:?}", split.error);
    let second = {
        let guard = separate.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .project
            .track(track)
            .unwrap()
            .clips
            .iter()
            .find_map(|c| match c {
                cut_core::Clip::Media(m) if m.src_in_ms == 1750 => Some(m.id.clone()),
                _ => None,
            })
            .unwrap()
    };
    let reset = dispatch(
        &separate,
        "screen_record.camera_reset",
        json!({"clip":second}),
        test_actor(),
    )
    .await;
    assert!(reset.ok, "{:?}", reset.error);
    let reset = reset.result.unwrap();
    assert!((reset["transform"]["scale"].as_f64().unwrap() - 180.0 / 640.0).abs() < 1e-9);
    assert_eq!(reset["transform"]["opacity"], 1.0);
    assert_eq!(reset["transform"]["x"], 460.0 / 640.0);
    let keys = reset["keyframes"].as_array().unwrap();
    let scale = keys.iter().find(|k| k["param"] == "scale").unwrap();
    assert_eq!(scale["points"][1]["t_ms"], 250);
    assert!((scale["points"][1]["value"].as_f64().unwrap() - 0.4).abs() < 1e-9);
}

#[test]
fn camera_timeline_visibility_and_size_match_recorded_pixels() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(camera_timeline_pixels_case());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn camera_artifact_without_webcam_plan_uses_recorder_default_look() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let (_root, state, reply) = camera_timeline_fixture("separate", true).await;
                    assert_eq!(reply["camera_look"]["transform"]["shape"], "circle");
                    assert!(
                        (reply["camera_look"]["transform"]["scale"].as_f64().unwrap()
                            - 79.0 / 640.0)
                            .abs()
                            < 1e-9
                    );
                    let guard = state.project.read().await;
                    let store = guard.as_ref().unwrap();
                    let jpeg = cut_media::render::extract_frame(
                        &store.project,
                        &cut_core::edl_from_project(&store.project),
                        &store.dir,
                        250,
                        None,
                    )
                    .unwrap();
                    let frame = store.dir.join("fallback.jpg");
                    std::fs::write(&frame, jpeg).unwrap();
                    let rgb = camera_test_rgb(&frame, 0.0);
                    let i = (306 * 640 + 585) * 3;
                    assert!(rgb[i] > 180 && rgb[i + 1] < 40);
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
