use super::super::{dispatch, screen_record_polish_subverb_error};
use super::test_actor;
use crate::output_paths::resolve_existing_project_file;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::json;

#[tokio::test]
async fn screen_record_stop_rejects_path_like_capture_id() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("capture_id_guard.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"capture_id_guard","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": "../escape"}),
        test_actor(),
    )
    .await;

    assert!(
        !r.ok,
        "path-like capture ids must be rejected before filesystem lookup"
    );
    let err = r.error.as_ref().unwrap();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("capture_id"),
        "error should name the invalid field: {err:?}"
    );
}

#[tokio::test]
async fn screen_record_studio_event_appends_camera_transform() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("studio_event.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"studio_event","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-studio-event";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();

    let r = dispatch(
        &state,
        "screen_record.studio_event",
        json!({
            "capture_id": capture_id,
            "event": {
                "t_ms": 1200,
                "source": "camera",
                "kind": "transform",
                "x": 0.72,
                "y": 0.66,
                "size": 0.22,
                "shape": "circle"
            }
        }),
        test_actor(),
    )
    .await;

    assert!(r.ok, "{:?}", r.error);
    let result = r.result.as_ref().unwrap();
    assert_eq!(result["count"], 1);
    let events_path = cap_dir.join("studio-events.json");
    let body: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
    assert_eq!(body["version"], 1);
    assert_eq!(body["events"][0]["source"], "camera");
    assert_eq!(body["events"][0]["kind"], "transform");
    assert_eq!(body["events"][0]["x"], 0.72);
}

#[tokio::test]
async fn screen_record_studio_event_rejects_path_like_capture_id() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("studio_event_guard.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"studio_event_guard","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "screen_record.studio_event",
        json!({
            "capture_id": "../escape",
            "event": {"t_ms": 0, "source": "camera", "kind": "visibility", "visible": true}
        }),
        test_actor(),
    )
    .await;

    assert!(!r.ok, "path-like capture ids must be rejected");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::INVALID_ARGS);
}

#[tokio::test]
async fn screen_record_studio_event_rejects_invalid_camera_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("studio_event_bounds.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"studio_event_bounds","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-studio-bounds";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();

    let r = dispatch(
        &state,
        "screen_record.studio_event",
        json!({
            "capture_id": capture_id,
            "event": {
                "t_ms": 100,
                "source": "camera",
                "kind": "transform",
                "x": 1.5,
                "y": 0.5,
                "size": 0.20
            }
        }),
        test_actor(),
    )
    .await;

    assert!(!r.ok, "out-of-range Studio transform should fail");
    let err = r.error.as_ref().unwrap();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("x"),
        "error should name the invalid field: {err:?}"
    );
}

#[test]
fn screen_record_project_file_resolver_rejects_outside_paths() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("recorder_inputs.cutproj");
    let capture_dir = project_dir.join("cache/screen_record/cap-test");
    std::fs::create_dir_all(&capture_dir).unwrap();
    let inside = capture_dir.join("events.json");
    std::fs::write(&inside, "{}").unwrap();

    let resolved = resolve_existing_project_file(
        &project_dir,
        "cache/screen_record/cap-test/events.json",
        "EventTrack",
        "test resolver",
    )
    .unwrap();
    assert_eq!(resolved, inside.canonicalize().unwrap());

    let outside_dir = dir.path().join("outside");
    std::fs::create_dir_all(&outside_dir).unwrap();
    let outside = outside_dir.join("events.json");
    std::fs::write(&outside, "{}").unwrap();

    let absolute_err = resolve_existing_project_file(
        &project_dir,
        outside.to_str().unwrap(),
        "EventTrack",
        "test resolver",
    )
    .expect_err("absolute outside EventTrack must be rejected");
    assert_eq!(absolute_err.code, error_codes::INVALID_ARGS);

    let relative_err = resolve_existing_project_file(
        &project_dir,
        "../outside/events.json",
        "EventTrack",
        "test resolver",
    )
    .expect_err("relative traversal EventTrack must be rejected");
    assert_eq!(relative_err.code, error_codes::INVALID_ARGS);
}

#[tokio::test]
async fn screen_record_stop_rejects_oversized_project_json() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("oversized.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"oversized","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-oversized";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();
    let project_path = cap_dir.join("project.json");
    let source = cap_dir.join("source.mp4");
    std::fs::write(&source, b"not-real-video").unwrap();
    let oversized_note = "x".repeat((4 * 1024 * 1024) + 1);
    std::fs::write(
        &project_path,
        serde_json::to_vec(&json!({
            "source_video": source.display().to_string(),
            "events": {"note": oversized_note}
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id}),
        test_actor(),
    )
    .await;

    assert!(!r.ok, "oversized capture project should be rejected");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::INVALID_ARGS);
    assert!(
        r.error
            .as_ref()
            .unwrap()
            .message
            .contains("project.json is too large"),
        "unexpected error: {:?}",
        r.error
    );
}

#[tokio::test]
async fn screen_record_stop_requires_project_json_object() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("non_object.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"non_object","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-non-object";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();
    std::fs::write(cap_dir.join("project.json"), b"[]").unwrap();

    let r = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id}),
        test_actor(),
    )
    .await;

    assert!(!r.ok, "non-object capture project should be rejected");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::INVALID_ARGS);
    assert!(
        r.error
            .as_ref()
            .unwrap()
            .message
            .contains("project.json must be an object"),
        "unexpected error: {:?}",
        r.error
    );
}

#[tokio::test]
async fn screen_record_stop_requires_source_video_for_all_modes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("missing_source_field.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"missing_source_field","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-missing-source-field";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&json!({"events": {}})).unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false}),
        test_actor(),
    )
    .await;

    assert!(
        !r.ok,
        "screen_record.stop should reject missing source_video"
    );
    let err = r.error.as_ref().unwrap();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("source_video"),
        "error should name source_video: {err:?}"
    );
}

#[tokio::test]
async fn screen_record_stop_returns_webcam_studio_events_and_raw_streams() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("stop_studio_metadata.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"stop_studio_metadata","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-stop-studio";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();
    let source = cap_dir.join("source.mp4");
    let webcam = cap_dir.join("webcam.mp4");
    let mic = cap_dir.join("mic.wav");
    let system = cap_dir.join("system.wav");
    std::fs::write(&source, b"screen").unwrap();
    std::fs::write(&webcam, b"camera").unwrap();
    std::fs::write(&mic, b"mic").unwrap();
    std::fs::write(&system, b"system").unwrap();
    let studio_events = cap_dir.join("studio-events.json");
    std::fs::write(
        &studio_events,
        serde_json::to_vec_pretty(&json!({
            "version": 1,
            "events": [
                {"t_ms": 0, "source": "camera", "kind": "visibility", "visible": true}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": source.display().to_string(),
            "webcam_video": webcam.display().to_string(),
            "audio": mic.display().to_string(),
            "events": {"duration_ms": 1000, "screen_w": 320, "screen_h": 180, "cursor": [], "clicks": [], "scrolls": [], "keys": []}
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false}),
        test_actor(),
    )
    .await;

    assert!(r.ok, "{:?}", r.error);
    let result = r.result.as_ref().unwrap();
    assert_eq!(
        result["webcam"],
        webcam.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        result["studio_events"],
        studio_events.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        result["raw_streams"]["screen"],
        source.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        result["raw_streams"]["camera"],
        webcam.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        result["raw_streams"]["mic"],
        mic.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        result["raw_streams"]["system"],
        system.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        result["raw_streams"]["studio_events"],
        studio_events.canonicalize().unwrap().display().to_string()
    );
}

#[tokio::test]
async fn screen_record_autoedit_patches_webcam_timeline_from_studio_events() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("autoedit_studio.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"autoedit_studio","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join("cap-autoedit-studio");
    std::fs::create_dir_all(&cap_dir).unwrap();
    let events = cap_dir.join("events.json");
    let webcam = cap_dir.join("webcam.mp4");
    let studio_events = cap_dir.join("studio-events.json");
    std::fs::write(
        &events,
        serde_json::to_vec_pretty(&json!({
            "duration_ms": 3000,
            "screen_w": 320,
            "screen_h": 180,
            "monitors": [],
            "cursor": [],
            "clicks": [],
            "scrolls": [],
            "keys": []
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(&webcam, b"camera").unwrap();
    std::fs::write(
        &studio_events,
        serde_json::to_vec_pretty(&json!({
            "version": 1,
            "events": [
                {"t_ms": 500, "source": "camera", "kind": "transform", "x": 0.10, "y": 0.20, "size": 0.30, "shape": "circle"},
                {"t_ms": 1500, "source": "camera", "kind": "visibility", "visible": false}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &state,
        "screen_record.autoedit",
        json!({
            "track": events.display().to_string(),
            "webcam": webcam.display().to_string(),
            "studio_events": studio_events.display().to_string()
        }),
        test_actor(),
    )
    .await;

    assert!(r.ok, "{:?}", r.error);
    let plan_path = r.result.as_ref().unwrap()["plan"].as_str().unwrap();
    let plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(plan_path).unwrap()).unwrap();
    assert_eq!(
        plan["webcam"]["source"],
        webcam.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(plan["webcam"]["timeline"][0]["t_ms"], 500);
    assert_eq!(plan["webcam"]["timeline"][0]["x"], 0.10);
    assert_eq!(plan["webcam"]["timeline"][1]["visible"], false);
}

#[tokio::test]
async fn screen_record_stop_mux_raw_requires_existing_source_video() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("mux_missing_source.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"mux_missing_source","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let capture_id = "cap-missing-source";
    let cap_dir = crate::screen_record::screen_record_cache_dir(&project_dir)
        .unwrap()
        .join(capture_id);
    std::fs::create_dir_all(&cap_dir).unwrap();
    let missing = cap_dir.join("missing-source.mp4");
    std::fs::write(
        cap_dir.join("project.json"),
        serde_json::to_vec(&json!({
            "source_video": missing.display().to_string(),
            "events": {}
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &state,
        "screen_record.stop",
        json!({"capture_id": capture_id, "autoedit": false, "mux_raw": true}),
        test_actor(),
    )
    .await;

    assert!(
        !r.ok,
        "mux_raw should not silently succeed without a source video"
    );
    let err = r.error.as_ref().unwrap();
    assert_eq!(err.code, error_codes::NOT_FOUND);
    assert!(
        err.message.contains("source_video"),
        "error should name the missing source_video: {err:?}"
    );
}

/// End-to-end placement: a recording WITH a sibling `system.wav` is polished
/// -> the desktop/system audio lands on its OWN `a_system` audio track (the video's
/// muxed mic already splits to its own track via linked-A/V import). The synthetic
/// capture lives under the project cache, matching the containment contract used
/// by `screen_record.stop` output.
#[tokio::test]
async fn f16_polish_places_system_audio() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("f16.cutproj");
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"f16","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let cap = project_dir
        .join("cache")
        .join("screen_record")
        .join("fixture");
    std::fs::create_dir_all(&cap).unwrap();
    let source = cap.join("rec.mp4");
    let ff1 = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=30:duration=3",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "3",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
            "-y",
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(ff1.success(), "synthetic recording mux failed");
    let syswav = cap.join("system.wav");
    let ff2 = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=5",
            "-ac",
            "2",
            "-ar",
            "48000",
            "-y",
        ])
        .arg(&syswav)
        .status()
        .unwrap();
    assert!(ff2.success(), "system.wav synth failed");

    let plan = record_core::EditPlan::empty(640, 360, 3000, 30.0);
    let plan_path = cap.join("plan.json");
    std::fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let pr = dispatch(
        &state,
        "screen_record.polish",
        json!({"source": source.display().to_string(), "plan": plan_path.display().to_string()}),
        test_actor(),
    )
    .await;
    assert!(pr.ok, "polish failed: {:?}", pr.error);
    assert!(
        pr.result
            .as_ref()
            .and_then(|r| r["system_clip_id"].as_str())
            .is_some(),
        "no system_clip_id in polish result: {:?}",
        pr.result
    );
    let guard = state.project.read().await;
    let proj = &guard.as_ref().unwrap().project;
    let sys_track = proj.tracks.iter().find(|t| t.id == "a_system");
    assert!(
        sys_track.is_some(),
        "no a_system track; tracks: {:?}",
        proj.tracks.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    let sys_media = sys_track.unwrap().clips.iter().find_map(|c| match c {
        cut_core::Clip::Media(m) => Some(m),
        _ => None,
    });
    assert!(sys_media.is_some(), "a_system track has no media clip");
    let m = sys_media.unwrap();
    let sys_dur = m.src_out_ms.saturating_sub(m.src_in_ms);
    assert!(
        sys_dur <= 3200,
        "a_system clip not trimmed to the video length: {sys_dur}ms (video is 3000ms; system.wav was 5000ms)"
    );
}

#[test]
fn screen_record_polish_subverb_failure_names_system_audio_step() {
    let inner = CutError::new(
        error_codes::INVALID_ARGS,
        "track kind mismatch",
        "audio cannot be inserted on a video track",
    );
    let res = VerbResult::err(inner);
    let err = screen_record_polish_subverb_error("system audio insert", &res)
        .expect("failed sub-verb should become a polish error");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(err.message.contains("system audio insert failed"));
    assert!(err.cause.contains("audio cannot be inserted"));
}
