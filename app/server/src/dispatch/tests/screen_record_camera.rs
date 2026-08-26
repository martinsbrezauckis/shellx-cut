#[tokio::test]
async fn screen_record_stop_validates_camera_artifact_integrity_and_containment() {
    use sha2::{Digest, Sha256};

    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("camera_artifact_stop.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"camera_artifact_stop","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let capture_id = "cap_camera_artifact";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    let camera_dir = cap_dir.join("camera");
    std::fs::create_dir_all(&camera_dir).unwrap();
    let source = cap_dir.join("source.mp4");
    let camera = camera_dir.join("camera.mp4");
    std::fs::write(&source, b"screen").unwrap();
    std::fs::write(&camera, b"camera bytes").unwrap();
    let camera_hash = format!("{:x}", Sha256::digest(b"camera bytes"));
    let project_json = json!({
        "source_video": source.display().to_string(),
        "camera_artifact": {
            "schema": "shellx-record/camera-artifact/1",
            "capture_id": capture_id,
            "artifact_id": "camera_01",
            "video": "camera/camera.mp4",
            "clock": {"first_frame_offset_ms": 500, "end_frame_offset_ms": 1500},
            "media": {
                "width": 640, "height": 480, "fps_num": 30, "fps_den": 1,
                "frame_count": 30, "duration_ms": 1000, "sha256": camera_hash,
            },
            "terminal_state": "complete",
        },
        "events": {"duration_ms": 1500, "screen_w": 640, "screen_h": 480,
            "cursor": [], "clicks": [], "scrolls": [], "keys": []}
    });
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&project_json).unwrap(),
    )
    .unwrap();

    let stopped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false}),
        test_actor(),
    )
    .await;
    assert!(stopped.ok, "{:?}", stopped.error);
    let result = stopped.result.unwrap();
    assert_eq!(result["camera_artifact"]["video"], "camera/camera.mp4");
    assert_eq!(
        result["camera_artifact"]["clock"]["first_frame_offset_ms"],
        500
    );
    assert_eq!(
        result["raw_streams"]["camera"],
        camera.canonicalize().unwrap().display().to_string()
    );

    std::fs::write(&camera, b"tampered").unwrap();
    let tampered = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false}),
        test_actor(),
    )
    .await;
    assert!(
        !tampered.ok,
        "tampered camera must fail integrity validation"
    );
    assert_eq!(tampered.error.unwrap().code, error_codes::INVALID_ARGS);

    let escaped_id = "cap_camera_escape";
    let escaped_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(escaped_id);
    std::fs::create_dir_all(&escaped_dir).unwrap();
    std::fs::write(escaped_dir.join("source.mp4"), b"screen").unwrap();
    let outside = project_dir.join("outside-camera.mp4");
    std::fs::write(&outside, b"outside").unwrap();
    let escaped = json!({
        "source_video": "source.mp4",
        "camera_artifact": {
            "schema": "shellx-record/camera-artifact/1",
            "capture_id": escaped_id,
            "artifact_id": "camera_02",
            "video": "../outside-camera.mp4",
            "clock": {"first_frame_offset_ms": 0, "end_frame_offset_ms": 1000},
            "media": {
                "width": 640, "height": 480, "fps_num": 30, "fps_den": 1,
                "frame_count": 30, "duration_ms": 1000,
                "sha256": format!("{:x}", Sha256::digest(b"outside")),
            },
            "terminal_state": "complete",
        },
        "events": {"duration_ms": 1000, "screen_w": 640, "screen_h": 480,
            "cursor": [], "clicks": [], "scrolls": [], "keys": []}
    });
    std::fs::write(
        escaped_dir.join("project.json"),
        serde_json::to_vec(&escaped).unwrap(),
    )
    .unwrap();
    let escaped = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": escaped_id, "autoedit": false}),
        test_actor(),
    )
    .await;
    assert!(
        !escaped.ok,
        "parent-traversing camera artifact must fail closed"
    );
    assert_eq!(escaped.error.unwrap().code, error_codes::INVALID_ARGS);
}

#[tokio::test]
async fn screen_record_polish_places_camera_artifact_on_its_own_offset_track() {
    use sha2::{Digest, Sha256};

    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("camera_artifact_polish.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"camera_artifact_polish","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap_camera_polish");
    let camera_dir = cap_dir.join("camera");
    std::fs::create_dir_all(&camera_dir).unwrap();
    let source = cap_dir.join("source.mp4");
    let source_status = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=30:duration=3",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(
        source_status.success(),
        "synthetic screen recording creation failed"
    );
    let camera = camera_dir.join("camera.mp4");
    let camera_status = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=1",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&camera)
        .status()
        .unwrap();
    assert!(
        camera_status.success(),
        "synthetic camera recording creation failed"
    );
    let camera_hash = format!("{:x}", Sha256::digest(std::fs::read(&camera).unwrap()));
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": source.display().to_string(),
            "camera_artifact": {
                "schema": "shellx-record/camera-artifact/1",
                "capture_id": "cap_camera_polish",
                "artifact_id": "camera_03",
                "video": "camera/camera.mp4",
                "clock": {"first_frame_offset_ms": 500, "end_frame_offset_ms": 1500},
                "media": {
                    "width": 320, "height": 240, "fps_num": 30, "fps_den": 1,
                    "frame_count": 30, "duration_ms": 1000, "sha256": camera_hash,
                },
                "terminal_state": "complete",
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let plan = record_core::EditPlan::empty(640, 360, 3_000, 30.0);
    let plan_path = cap_dir.join("plan.json");
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();

    let polished = dispatch(
        &state,
        "screen_record.polish",
        json!({"source": source.display().to_string(), "plan": plan_path.display().to_string(), "raw": true}),
        test_actor(),
    )
    .await;
    assert!(polished.ok, "{:?}", polished.error);
    let result = polished.result.unwrap();
    assert_eq!(result["camera_artifact_id"], "camera_03");
    assert_eq!(result["camera_track_id"], "v_camera");
    assert_eq!(result["camera_first_frame_offset_ms"], 500);
    let camera_clip_id = result["camera_clip_id"].as_str().unwrap();

    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let camera_track = project
        .tracks
        .iter()
        .find(|track| track.id == "v_camera")
        .expect("camera must be a separate editable video track");
    assert_eq!(camera_track.kind, cut_core::TrackKind::Video);
    assert!(
        !camera_track.visible,
        "foundation camera track stays hidden until the real visible-track workflow exists"
    );
    let camera_clip = camera_track
        .clips
        .iter()
        .find_map(|clip| match clip {
            cut_core::Clip::Media(media) if media.id == camera_clip_id => Some(media),
            _ => None,
        })
        .expect("camera track must contain the inserted artifact clip");
    assert_eq!(camera_clip.src_in_ms, 0);
    assert_eq!(camera_clip.src_out_ms, 1_000);
    let placement_ms = camera_track
        .clips
        .iter()
        .take_while(
            |clip| !matches!(clip, cut_core::Clip::Media(media) if media.id == camera_clip_id),
        )
        .map(cut_core::Clip::timeline_duration_ms)
        .sum::<u64>();
    assert_eq!(
        placement_ms, 500,
        "camera clip must retain CaptureClock offset"
    );
}
