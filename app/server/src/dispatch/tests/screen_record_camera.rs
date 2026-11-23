use std::path::Path;
include!("screen_record_camera_timeline.rs");
include!("screen_record_camera_live.rs");
include!("screen_record_camera_edits.rs");

fn camera_test_rgb(path: &std::path::Path, at: f64) -> Vec<u8> {
    let mut command = std::process::Command::new("ffmpeg");
    command.args(["-v", "error", "-threads", "1"]);
    if at > 0.0 {
        command.args(["-ss", &at.to_string()]);
    }
    let output = command
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 640 * 360 * 3);
    output.stdout
}

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

async fn screen_record_polish_places_camera_artifact_on_its_own_offset_track_case(
    separate: bool,
    raw: bool,
) {
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
            if !raw {
                "color=c=green:size=640x360:rate=30:duration=3"
            } else {
                "testsrc=size=640x360:rate=30:duration=3"
            },
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
            if !raw {
                "color=c=red:size=320x240:rate=30:duration=1"
            } else {
                "testsrc2=size=320x240:rate=30:duration=1"
            },
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
    let mut plan = record_core::EditPlan::empty(640, 360, 3_000, 30.0);
    if !raw {
        plan.webcam = Some(record_core::WebcamOverlay {
            source: camera.display().to_string(),
            shape: record_core::WebcamShape::Circle,
            anchor: record_core::Anchor::BottomRight,
            margin: 0.05,
            size: 0.3,
            camera_clock: None,
            timeline: vec![record_core::WebcamKeyframe {
                t_ms: 1000,
                visible: None,
                x: Some(0.1),
                y: Some(0.2),
                size: None,
                shape: None,
            }],
        });
        std::fs::write(cap_dir.join("events.json"), b"[]").unwrap();
    }
    let plan_path = cap_dir.join("plan.json");
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();

    // Simulate a process loss immediately after the durable media.import op:
    // there is one legitimate camera asset but no camera track or clip yet.
    // Recovery must adopt it by its sealed source identity, not import it again.
    let imported_before_restart = dispatch(
        &state,
        "media.import",
        json!({"path": camera.display().to_string(), "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(
        imported_before_restart.ok,
        "pre-restart camera import failed: {:?}",
        imported_before_restart.error
    );
    let pre_restart_camera_asset_id = imported_before_restart.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();
    close_camera_test_project_after_jobs_drain(&state, "after durable camera import").await;
    let state = AppState::new();
    let opened = dispatch(
        &state,
        "project.open",
        json!({"path": project_dir}),
        test_actor(),
    )
    .await;
    assert!(
        opened.ok,
        "camera project reopen after import failed: {:?}",
        opened.error
    );

    let mut polish_args = json!({"source": source.display().to_string(), "plan": plan_path.display().to_string(), "raw": raw});
    if !separate {
        polish_args["camera"] = json!("baked");
    }
    let polished = dispatch(&state, "screen_record.polish", polish_args, test_actor()).await;
    assert!(polished.ok, "{:?}", polished.error);
    let result = polished.result.unwrap();
    assert_eq!(result["camera_artifact_id"], "camera_03");
    let camera_track_id = result["camera_track_id"]
        .as_str()
        .expect("camera placement must return its durable identity")
        .to_string();
    assert!(
        camera_track_id.starts_with(if separate { "Camera" } else { "v_camera_" }),
        "camera track identity is private, deterministic server ownership"
    );
    assert_eq!(result["camera_asset_id"], pre_restart_camera_asset_id);
    assert_eq!(result["camera_first_frame_offset_ms"], 500);
    let camera_clip_id = result["camera_clip_id"].as_str().unwrap();

    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let camera_track = project
        .tracks
        .iter()
        .find(|track| track.id == camera_track_id)
        .expect("camera must be a separate editable video track");
    assert_eq!(camera_track.kind, cut_core::TrackKind::Video);
    assert!(
        camera_track.visible == separate,
        "separate camera must be visible; raw/baked compatibility stays hidden"
    );
    let camera_clip = camera_track
        .clips
        .iter()
        .find_map(|clip| match clip {
            cut_core::Clip::Media(media) if media.id == camera_clip_id => Some(media),
            _ => None,
        })
        .expect("camera track must contain the inserted artifact clip");
    if separate {
        assert_eq!(
            serde_json::to_value(&project.recordings[0]).unwrap()["camera_asset"],
            result["camera_asset_id"]
        );
        let transform = camera_clip.transform.as_ref().unwrap();
        assert_eq!(transform.shape, cut_core::types::OverlayShape::Circle);
        assert!((transform.scale - 0.16875).abs() < 1e-6);
        assert!((transform.x - 0.803125).abs() < 1e-6);
        assert!((transform.y - 0.65).abs() < 1e-6);
        assert_eq!(transform.ring.as_ref().unwrap().colour, "#ffffffe6");
        assert_eq!(camera_clip.keyframes.len(), 2);
        assert_eq!(
            camera_clip.keyframes[0].interp,
            cut_core::types::KfInterp::Hold
        );
        assert_eq!(camera_clip.keyframes[0].points[1].t_ms, 500);
        assert_eq!(camera_clip.keyframes[0].points[1].value, 0.1);
        assert_eq!(
            project.recordings[0].camera_clip.as_deref(),
            Some(camera_clip_id)
        );
    }
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
    if separate {
        let baked = result["baked"].as_str().unwrap();
        let rgb = camera_test_rgb(Path::new(baked), 0.75);
        let center = (288 * 640 + 568) * 3;
        assert!(
            rgb[center + 1] > 80 && rgb[center] < 30,
            "screen bake must show green screen, not red camera"
        );
        let jpeg = cut_media::render::extract_frame(
            project,
            &cut_core::edl_from_project(project),
            &project_dir,
            750,
            None,
        )
        .unwrap();
        let frame_path = project_dir.join("separate-exact.jpg");
        std::fs::write(&frame_path, &jpeg).unwrap();
        let rgb = camera_test_rgb(&frame_path, 0.0);
        assert!(
            rgb[center] > 180 && rgb[center + 1] < 40,
            "exact-frame camera bubble must be red"
        );
        let corner = (236 * 640 + 516) * 3;
        assert!(
            rgb[corner + 1] > 80 && rgb[corner] < 40,
            "circle corner must show screen"
        );
        if let Ok(path) = std::env::var("CAMERA_PROOF_IMAGE") {
            std::fs::write(path, jpeg).unwrap();
        }
        let moved = cut_media::render::extract_frame(
            project,
            &cut_core::edl_from_project(project),
            &project_dir,
            1250,
            None,
        )
        .unwrap();
        let moved_path = project_dir.join("separate-moved.jpg");
        std::fs::write(&moved_path, moved).unwrap();
        let rgb = camera_test_rgb(&moved_path, 0.0);
        let moved_center = (126 * 640 + 118) * 3;
        assert!(
            rgb[moved_center] > 180 && rgb[moved_center + 1] < 40,
            "hold position keys move the composed bubble"
        );
        assert!(
            rgb[center + 1] > 80 && rgb[center] < 40,
            "hold keys leave the old position clear"
        );
    }
    if !separate && !raw {
        let rgb = camera_test_rgb(Path::new(result["baked"].as_str().unwrap()), 0.75);
        let center = (288 * 640 + 568) * 3;
        assert!(
            rgb[center] > 180 && rgb[center + 1] < 40,
            "baked mode keeps the camera bubble in the screen bake"
        );
    }
    drop(guard);
    if !separate && !raw {
        camera_assert_baked_compatibility(&state, camera_clip_id).await;
    }
    if separate {
        let edited = dispatch(
            &state,
            "edit.transform",
            json!({"clip":camera_clip_id,"x":0.2,"y":0.1,"scale":0.2,"opacity":1}),
            test_actor(),
        )
        .await;
        assert!(edited.ok, "{:?}", edited.error);
        let reset = dispatch(
            &state,
            "screen_record.camera_reset",
            json!({"clip":camera_clip_id}),
            test_actor(),
        )
        .await;
        assert!(reset.ok, "{:?}", reset.error);
        assert_eq!(reset.result.unwrap()["transform"]["shape"], "circle");
        let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
        assert!(undo.ok, "{:?}", undo.error);
        {
            let guard = state.project.read().await;
            let p = &guard.as_ref().unwrap().project;
            let (t, i) = p.find_clip(camera_clip_id).unwrap();
            let cut_core::Clip::Media(m) = &p.track(t).unwrap().clips[i] else {
                panic!()
            };
            assert_eq!(m.transform.as_ref().unwrap().x, 0.2);
        }
        let redo = dispatch(&state, "project.redo", json!({}), test_actor()).await;
        assert!(redo.ok, "{:?}", redo.error);
        let split = dispatch(
            &state,
            "edit.split",
            json!({"track":camera_track_id,"at_ms":1000}),
            test_actor(),
        )
        .await;
        assert!(split.ok, "{:?}", split.error);
        let second = {
            let guard = state.project.read().await;
            guard
                .as_ref()
                .unwrap()
                .project
                .track(&camera_track_id)
                .unwrap()
                .clips
                .iter()
                .find_map(|c| match c {
                    cut_core::Clip::Media(m) if m.src_in_ms == 500 => Some(m.id.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let state_reply = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap();
        let camera_annotation = state_reply["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap())
            .find(|c| c["id"] == second)
            .unwrap()["recording"]
            .clone();
        assert_eq!(camera_annotation["camera"], true);
        let edited = dispatch(
            &state,
            "edit.transform",
            json!({"clip":second,"x":0.4,"y":0.4,"scale":0.2}),
            test_actor(),
        )
        .await;
        assert!(edited.ok, "{:?}", edited.error);
        let reset = dispatch(
            &state,
            "screen_record.camera_reset",
            json!({"clip":second}),
            test_actor(),
        )
        .await;
        assert!(reset.ok, "{:?}", reset.error);
        let reset = reset.result.unwrap();
        assert_eq!(reset["transform"]["x"], 0.1);
        assert_eq!(reset["transform"]["y"], 0.2);
        let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
        assert!(undo.ok);
        {
            let guard = state.project.read().await;
            let p = &guard.as_ref().unwrap().project;
            let (t, i) = p.find_clip(&second).unwrap();
            let cut_core::Clip::Media(m) = &p.track(t).unwrap().clips[i] else {
                panic!()
            };
            assert_eq!(m.transform.as_ref().unwrap().x, 0.4);
        }
        let screen = result["clip_id"].as_str().unwrap();
        let changed = dispatch(
            &state,
            "screen_record.zoom_add",
            json!({"clip":screen,"at_ms":500,"duration_ms":2000}),
            test_actor(),
        )
        .await;
        assert!(changed.ok, "{:?}", changed.error);
        let refresh = dispatch(
            &state,
            "screen_record.take_refresh",
            json!({"clip":screen}),
            test_actor(),
        )
        .await;
        assert!(refresh.ok, "{:?}", refresh.error);
        let id = refresh.result.unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let mut done = None;
        // 5 minutes: a slow shared machine needs more than 30 s for this bake.
        for _ in 0..6_000 {
            let job = state.jobs.get(&id).unwrap();
            if matches!(
                job.state,
                crate::jobs::JobState::Done | crate::jobs::JobState::Failed
            ) {
                done = Some(job);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let job = done.expect("refresh completes");
        assert_eq!(job.state, crate::jobs::JobState::Done, "{:?}", job.error);
        let guard = state.project.read().await;
        let p = &guard.as_ref().unwrap().project;
        let rgb = camera_test_rgb(Path::new(&p.assets[&p.recordings[0].asset].path), 0.75);
        let center = (288 * 640 + 568) * 3;
        assert!(
            rgb[center + 1] > 80 && rgb[center] < 30,
            "refresh must not bake the camera back into the screen"
        );
    }

    // A restart can happen after every individual lowered op (import, track
    // creation, visibility, or insert). Once all have committed, retrying the
    // same sealed capture must adopt that durable identity rather than add a
    // second camera asset or clip.
    let first_camera_asset_id = result["camera_asset_id"].as_str().unwrap().to_string();
    let first_camera_clip_id = camera_clip_id.to_string();
    close_camera_test_project_after_jobs_drain(&state, "after camera polish").await;
    let reopened = AppState::new();
    let opened = dispatch(
        &reopened,
        "project.open",
        json!({"path": project_dir}),
        test_actor(),
    )
    .await;
    assert!(
        opened.ok,
        "camera project reopen failed: {:?}",
        opened.error
    );

    let retried = dispatch(
        &reopened,
        "screen_record.polish",
        json!({"source": source.display().to_string(), "plan": plan_path.display().to_string(), "raw": raw, "camera": if separate { "separate" } else { "baked" }}),
        test_actor(),
    )
    .await;
    assert!(
        retried.ok,
        "camera polish retry failed: {:?}",
        retried.error
    );
    let retried = retried.result.unwrap();
    assert_eq!(
        retried["baked"], result["baked"],
        "same inputs keep the bake name"
    );
    assert_eq!(retried["cached"], true, "unchanged bake is reused");
    assert_eq!(retried["camera_track_id"], camera_track_id);
    assert_eq!(retried["camera_asset_id"], first_camera_asset_id);
    assert_eq!(retried["camera_clip_id"], first_camera_clip_id);

    let reopened_guard = reopened.project.read().await;
    let reopened_camera_track = reopened_guard
        .as_ref()
        .unwrap()
        .project
        .tracks
        .iter()
        .find(|track| track.id == camera_track_id)
        .expect("camera track must survive reopen");
    assert_eq!(
        reopened_camera_track
            .clips
            .iter()
            .filter(|clip| matches!(clip, cut_core::Clip::Media(media) if media.id == first_camera_clip_id))
            .count(),
        1,
        "retry must not duplicate the durable camera clip"
    );
}

async fn screen_record_polish_camera_tail_cannot_extend_final_render_case() {
    use sha2::{Digest, Sha256};

    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("camera_tail.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"camera_tail","dir":project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap_camera_tail");
    let camera_dir = cap_dir.join("camera");
    std::fs::create_dir_all(&camera_dir).unwrap();
    let source = cap_dir.join("source.mp4");
    let camera = camera_dir.join("camera.mp4");
    for (path, input) in [
        (&source, "testsrc=size=640x360:rate=30:duration=3"),
        (&camera, "testsrc2=size=320x240:rate=30:duration=3"),
    ] {
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-nostats",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                input,
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success(), "fixture video encoding failed");
    }
    let camera_hash = format!("{:x}", Sha256::digest(std::fs::read(&camera).unwrap()));
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": source.display().to_string(),
            "camera_artifact": {
                "schema": "shellx-record/camera-artifact/1",
                "capture_id": "cap_camera_tail",
                "artifact_id": "camera_tail",
                "video": "camera/camera.mp4",
                "clock": {"first_frame_offset_ms":1291,"end_frame_offset_ms":4291},
                "media": {"width":320,"height":240,"fps_num":30,"fps_den":1,
                          "frame_count":90,"duration_ms":3000,"sha256":camera_hash},
                "terminal_state": "complete"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let plan_path = cap_dir.join("plan.json");
    std::fs::write(
        &plan_path,
        serde_json::to_vec(&record_core::EditPlan::empty(640, 360, 3_000, 30.0)).unwrap(),
    )
    .unwrap();

    let polished = dispatch(
        &state,
        "screen_record.polish",
        json!({"source":source.display().to_string(),"plan":plan_path.display().to_string(),"raw":true}),
        test_actor(),
    )
    .await;
    assert!(polished.ok, "{:?}", polished.error);
    let result = polished.result.unwrap();
    let camera_asset_id = result["camera_asset_id"].as_str().unwrap();
    let camera_clip_id = result["camera_clip_id"].as_str().unwrap();
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let imported_camera = &project.assets[camera_asset_id];
    assert_eq!(
        std::fs::canonicalize(&imported_camera.path).unwrap(),
        camera.canonicalize().unwrap(),
        "the imported camera asset must resolve to the measured camera file"
    );
    assert_eq!(imported_camera.hash, format!("sha256:{camera_hash}"));
    assert_eq!(
        std::fs::read(&imported_camera.path).unwrap(),
        std::fs::read(&camera).unwrap(),
        "the complete measured camera file must remain available as an independent asset"
    );
    let camera_track = project
        .track(result["camera_track_id"].as_str().unwrap())
        .unwrap();
    assert!(!camera_track.visible);
    let camera_clip = camera_track
        .clips
        .iter()
        .find_map(|clip| match clip {
            cut_core::Clip::Media(media) if media.id == camera_clip_id => Some(media),
            _ => None,
        })
        .unwrap();
    assert_eq!((camera_clip.src_in_ms, camera_clip.src_out_ms), (0, 1_709));
    assert_eq!(cut_core::edl_from_project(project).duration_ms, 3_000);
    drop(guard);
    let render_plan = dispatch(
        &state,
        "render.final",
        json!({"dry_run":true}),
        test_actor(),
    )
    .await;
    assert!(render_plan.ok, "{:?}", render_plan.error);
    assert_eq!(render_plan.result.unwrap()["output"]["duration_ms"], 3_000);
}

#[test]
fn screen_record_polish_camera_tail_cannot_extend_final_render() {
    let test = std::thread::Builder::new()
        .name("screen-record-camera-tail-test".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(screen_record_polish_camera_tail_cannot_extend_final_render_case());
        })
        .expect("camera tail test thread must start");
    test.join()
        .expect("camera tail test thread must complete without panicking");
}

async fn close_camera_test_project_after_jobs_drain(state: &AppState, phase: &str) {
    let mut last_pending = String::new();
    for _ in 0..10 {
        let closed = dispatch(state, "project.close", json!({}), test_actor()).await;
        if closed.ok {
            return;
        }
        let code = closed.error.as_ref().map(|error| error.code.as_str());
        let detail = format!("{:?}", closed.error);
        assert_eq!(
            code,
            Some("job_cancel_pending"),
            "camera test project close failed {phase}: {detail}"
        );
        last_pending = detail;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("camera test project close kept draining {phase}: {last_pending}");
}

#[test]
fn screen_record_baked_camera_preserves_legacy_bubble() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(
                    screen_record_polish_places_camera_artifact_on_its_own_offset_track_case(
                        false, false,
                    ),
                );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn screen_record_separate_camera_visible_look_and_offset() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(
                    screen_record_polish_places_camera_artifact_on_its_own_offset_track_case(
                        true, false,
                    ),
                );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn screen_record_polish_places_camera_artifact_on_its_own_offset_track() {
    // Windows test-harness threads have a smaller stack than the large async
    // camera-polish fixture needs. Keep the larger stack scoped to this test;
    // a global RUST_MIN_STACK would also alter compiler and unrelated workers.
    let test = std::thread::Builder::new()
        .name("screen-record-camera-polish-test".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(
                    screen_record_polish_places_camera_artifact_on_its_own_offset_track_case(
                        false, true,
                    ),
                );
        })
        .expect("camera polish test thread must start");
    test.join()
        .expect("camera polish test thread must complete without panicking");
}
