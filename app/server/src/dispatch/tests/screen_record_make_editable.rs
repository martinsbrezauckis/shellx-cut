use super::*;
use sha2::{Digest, Sha256};

fn zoom_plan(
    duration_ms: u64,
    with_zoom: bool,
    with_webcam: bool,
    cam_path: &str,
) -> record_core::EditPlan {
    let mut plan = record_core::EditPlan::empty(1920, 1080, duration_ms, 30.0);
    if with_zoom {
        plan.zoom = record_core::zoom_segments::segments_to_keys(
            &[record_core::zoom_segments::ZoomSegment {
                id: "z1".into(),
                start_ms: 500,
                end_ms: 2500,
                scale: 2.0,
                cx: 0.4,
                cy: 0.5,
                ramp_in_ms: 550,
                ramp_out_ms: 500,
                origin: record_core::zoom_segments::ZoomOrigin::Auto,
            }],
            duration_ms,
        )
        .unwrap();
    }
    if with_webcam {
        plan.webcam = Some(record_core::WebcamOverlay {
            source: cam_path.to_string(),
            shape: record_core::WebcamShape::Circle,
            anchor: record_core::Anchor::BottomRight,
            margin: 0.05,
            size: 0.25,
            camera_clock: None,
            timeline: vec![],
        });
    }
    plan
}

fn events_track(duration_ms: u64) -> record_core::EventTrack {
    record_core::EventTrack {
        duration_ms,
        screen_w: 1920,
        screen_h: 1080,
        monitors: vec![],
        cursor: vec![record_core::CursorSample {
            t_ms: 500,
            x: 768.0,
            y: 540.0,
        }],
        clicks: vec![],
        scrolls: vec![],
        keys: vec![],
        cursor_correlation: record_core::CursorCorrelation::default(),
    }
}

struct OldRecordingFixture {
    _root: tempfile::TempDir,
    project_dir: std::path::PathBuf,
    cache_dir: std::path::PathBuf,
    capture_dir: std::path::PathBuf,
    plan_path: std::path::PathBuf,
    baked_path: std::path::PathBuf,
    camera_track_id: Option<String>,
    state: AppState,
}

async fn setup_old_polished_project(with_camera: bool, duration_ms: u64) -> OldRecordingFixture {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("old118.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "old118", "dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let cache_dir = project_dir.join("cache/screen_record");
    let capture_dir = cache_dir.join("cap_1000_0");
    std::fs::create_dir_all(&capture_dir).unwrap();

    let source_path = capture_dir.join("source.mp4");
    std::fs::write(&source_path, b"synthetic-06118-source-mp4").unwrap();
    let src_hash = cut_core::hash_file(&source_path).unwrap();
    std::fs::write(
        capture_dir.join("events.json"),
        serde_json::to_vec_pretty(&events_track(duration_ms)).unwrap(),
    )
    .unwrap();

    let mut camera_track_id = None;
    let cam_path = capture_dir.join("camera/camera_01.mp4");
    if with_camera {
        std::fs::create_dir_all(cam_path.parent().unwrap()).unwrap();
        let cam_bytes = b"synthetic-06118-cam-mp4";
        std::fs::write(&cam_path, cam_bytes).unwrap();
        let cam_hash = format!("{:x}", Sha256::digest(cam_bytes));
        let artifact = record_core::CameraArtifact::new(
            "cap_1000_0",
            "camera_01",
            "camera/camera_01.mp4",
            record_core::CameraClockRange {
                first_frame_offset_ms: 0,
                end_frame_offset_ms: duration_ms,
            },
            record_core::CameraMediaFacts {
                width: 640,
                height: 480,
                fps_num: 30,
                fps_den: 1,
                frame_count: 150,
                duration_ms,
                sha256: cam_hash.clone(),
            },
            record_core::CameraTerminalState::Complete,
        )
        .unwrap();
        std::fs::write(
            capture_dir.join("project.json"),
            serde_json::to_vec_pretty(&json!({
                "schema": "shellx-record/1",
                "source_video": "source.mp4",
                "settings": {},
                "events": events_track(duration_ms),
                "camera_artifact": artifact,
            }))
            .unwrap(),
        )
        .unwrap();
        let mut digest = Sha256::new();
        digest.update(artifact.schema.as_bytes());
        digest.update([0]);
        digest.update(artifact.capture_id.as_bytes());
        digest.update([0]);
        digest.update(artifact.artifact_id.as_bytes());
        camera_track_id = Some(format!(
            "v_camera_{}",
            &format!("{:x}", digest.finalize())[..32]
        ));
    } else {
        std::fs::write(
            capture_dir.join("project.json"),
            serde_json::to_vec_pretty(&json!({
                "schema": "shellx-record/1",
                "source_video": "source.mp4",
                "settings": {},
                "events": events_track(duration_ms),
            }))
            .unwrap(),
        )
        .unwrap();
    }

    let plan = zoom_plan(duration_ms, true, with_camera, &cam_path.to_string_lossy());
    let plan_bytes = serde_json::to_vec_pretty(&plan).unwrap();
    let plan_file_sha = format!("{:x}", Sha256::digest(&plan_bytes));
    let plan_path = cache_dir.join(format!("plan-{plan_file_sha}.json"));
    std::fs::write(&plan_path, &plan_bytes).unwrap();
    let plan_tag = &plan_file_sha[..16];

    let baked_name = format!("{}_{plan_tag}_v.mp4", src_hash.replace(':', "_"));
    let baked_path = cache_dir.join(&baked_name);
    std::fs::write(&baked_path, b"synthetic-baked-06118-mp4").unwrap();
    let baked_hash = cut_core::hash_file(&baked_path).unwrap();

    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .apply_atomic_media_insert_plan(
                &"1".repeat(64),
                vec![cut_core::Asset {
                    path: baked_path.to_string_lossy().into_owned(),
                    hash: baked_hash,
                    probe: Some(json!({"kind":"video","duration_ms":duration_ms,"width":1920,"height":1080})),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                }],
                vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,duration_ms]})],
                test_actor(),
                None,
            )
            .unwrap();
        if let Some(ref cam_track) = camera_track_id {
            let cam_hash = cut_core::hash_file(&cam_path).unwrap();
            store
                .apply(
                    "edit.add_track",
                    json!({"kind":"video","id":cam_track}),
                    test_actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.track_visible",
                    json!({"track":cam_track,"on":false}),
                    test_actor(),
                    None,
                )
                .unwrap();
            store
                .apply_atomic_media_insert_plan(
                    &"2".repeat(64),
                    vec![cut_core::Asset {
                        path: cam_path.to_string_lossy().into_owned(),
                        hash: cam_hash,
                        probe: Some(json!({"kind":"video","duration_ms":duration_ms,"width":640,"height":480})),
                        transcript: None,
                        perception: None,
                        proxy: None,
                        filmstrip: None,
                    }],
                    vec![json!({"track":cam_track,"at_ms":0,"src_range_ms":[0,duration_ms]})],
                    test_actor(),
                    None,
                )
                .unwrap();
        }
    }

    OldRecordingFixture {
        _root: root,
        project_dir,
        cache_dir,
        capture_dir,
        plan_path,
        baked_path,
        camera_track_id,
        state,
    }
}

#[tokio::test]
async fn project_state_annotates_old_recording_without_auto_adopting() {
    let f = setup_old_polished_project(false, 5000).await;
    let st = dispatch(&f.state, "project.state", json!({}), test_actor()).await;
    assert!(st.ok, "{:?}", st.error);
    let val = st.result.unwrap();
    assert!(val["recordings"].as_array().is_none_or(|a| a.is_empty()));
    let c1 = &val["tracks"][0]["clips"][0];
    assert_eq!(c1["old_recording"], json!(true));
    assert!(c1.get("recording").is_none());
}

#[tokio::test]
async fn make_editable_converts_screen_recording_preserves_asset_and_undoes() {
    let f = setup_old_polished_project(false, 5000).await;
    let res = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(res.ok, "make_editable failed: {:?}", res.error);
    let body = res.result.unwrap();
    assert_eq!(body["stale"], json!(false));
    assert_eq!(body["segments"].as_array().unwrap().len(), 1);

    // Zooms are now editable and original asset a1 remains on the timeline clip.
    let zoom = dispatch(
        &f.state,
        "screen_record.zoom_list",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(zoom.ok, "{:?}", zoom.error);
    assert_eq!(
        zoom.result.unwrap()["segments"].as_array().unwrap().len(),
        1
    );
    let upd = dispatch(
        &f.state,
        "screen_record.zoom_update",
        json!({"clip":"c1","id":"z1","scale":2.5}),
        test_actor(),
    )
    .await;
    assert!(upd.ok, "{:?}", upd.error);
    assert_eq!(upd.result.unwrap()["stale"], json!(true));

    // Revert back past zoom_update and make_editable in one Undo each.
    assert!(
        dispatch(&f.state, "project.undo", json!({}), test_actor())
            .await
            .ok
    );
    assert!(
        dispatch(&f.state, "project.undo", json!({}), test_actor())
            .await
            .ok
    );
    let st = dispatch(&f.state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert!(st["recordings"].as_array().is_none_or(|a| a.is_empty()));
    assert_eq!(
        st["assets"]["a1"]["path"],
        json!(f.baked_path.to_string_lossy())
    );
}

#[tokio::test]
async fn make_editable_activates_camera_and_reverts_atomically_on_one_undo() {
    let f = setup_old_polished_project(true, 5000).await;
    let cam_track = f.camera_track_id.clone().unwrap();

    let res = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(res.ok, "make_editable camera failed: {:?}", res.error);
    let body = res.result.unwrap();
    assert_eq!(body["camera_clip"], json!("c2"));
    assert_eq!(body["stale"], json!(true));

    let st = dispatch(&f.state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    let vcam = st["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == cam_track)
        .unwrap();
    assert!(vcam.get("visible").and_then(Value::as_bool).unwrap_or(true));
    assert!(vcam["clips"][0]["transform"].is_object());
    assert_eq!(vcam["clips"][0]["recording"]["camera"], json!(true));

    // One Undo restores pre-make_editable state atomically.
    let undo = dispatch(&f.state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "{:?}", undo.error);
    let st_after = dispatch(&f.state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert!(st_after["recordings"]
        .as_array()
        .is_none_or(|a| a.is_empty()));
    let vcam_after = st_after["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == cam_track)
        .unwrap();
    assert_eq!(vcam_after["visible"], json!(false));
    assert!(vcam_after["clips"][0].get("transform").is_none());

    // Redo + clean open replay preserves activated camera track and editable take.
    assert!(
        dispatch(&f.state, "project.redo", json!({}), test_actor())
            .await
            .ok
    );
    let reopened = cut_core::ProjectStore::open(&f.project_dir).unwrap();
    assert_eq!(reopened.project.recordings.len(), 1);
    assert!(reopened.project.track(&cam_track).unwrap().visible);
}

#[tokio::test]
async fn make_editable_says_so_when_the_camera_clip_cannot_be_found() {
    let f = setup_old_polished_project(true, 5000).await;
    let cam_track = f.camera_track_id.clone().unwrap();
    {
        let mut g = f.state.project.write().await;
        let s = g.as_mut().unwrap();
        s.apply(
            "edit.remove_track",
            json!({"track": cam_track, "force": true}),
            test_actor(),
            None,
        )
        .unwrap();
    }
    let res = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(res.ok, "{:?}", res.error);
    let body = res.result.unwrap();
    assert_eq!(body["camera_clip"], Value::Null);
    let note = body["camera_note"].as_str().expect("camera_note in reply");
    assert!(
        note.starts_with("The camera could not be made editable:"),
        "{note}"
    );
}

#[tokio::test]
async fn make_editable_refuses_second_attempt_after_conversion_and_non_recording_clips() {
    let f = setup_old_polished_project(false, 5000).await;
    assert!(
        dispatch(
            &f.state,
            "screen_record.make_editable",
            json!({"clip":"c1"}),
            test_actor()
        )
        .await
        .ok
    );
    let second = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    let err = second.error.unwrap();
    assert!(err.message.contains("already editable"), "{err}");
    assert!(err.suggested_action.is_some());

    // Add a raw asset clip and ordinary media clip and verify clean refusals.
    {
        let mut g = f.state.project.write().await;
        let s = g.as_mut().unwrap();
        let raw_path = f.cache_dir.join("sha256_deadbeef_raw_v.mp4");
        std::fs::write(&raw_path, b"raw").unwrap();
        let ord_path = f.project_dir.join("ordinary.mp4");
        std::fs::write(&ord_path, b"ord").unwrap();
        s.apply_atomic_media_insert_plan(
            &"3".repeat(64),
            vec![cut_core::Asset {
                path: raw_path.to_string_lossy().into_owned(),
                hash: "sha256:raw".into(),
                probe: Some(json!({"kind":"video","duration_ms":2000})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }],
            vec![json!({"track":"v1","at_ms":5000,"src_range_ms":[0,2000]})],
            test_actor(),
            None,
        )
        .unwrap();
        s.apply_atomic_media_insert_plan(
            &"4".repeat(64),
            vec![cut_core::Asset {
                path: ord_path.to_string_lossy().into_owned(),
                hash: "sha256:ord".into(),
                probe: Some(json!({"kind":"video","duration_ms":2000})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            }],
            vec![json!({"track":"v1","at_ms":7000,"src_range_ms":[0,2000]})],
            test_actor(),
            None,
        )
        .unwrap();
    }
    let raw_err = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c2"}),
        test_actor(),
    )
    .await
    .error
    .unwrap();
    assert!(raw_err.message.contains("Raw recordings"), "{raw_err}");
    assert!(raw_err.suggested_action.is_some());

    let ord_err = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c3"}),
        test_actor(),
    )
    .await
    .error
    .unwrap();
    assert!(ord_err.suggested_action.is_some(), "{ord_err}");
}

#[tokio::test]
async fn make_editable_handles_zero_zooms_concurrent_clients_symlinks_and_restart() {
    // 1. Concurrent second client on the same clip: exactly one succeeds, one refuses cleanly.
    let f = setup_old_polished_project(false, 5000).await;
    let a1 = Actor {
        kind: cut_core::ActorKind::Human,
        name: "client-a".into(),
        via: "ui".into(),
        request: None,
    };
    let a2 = Actor {
        kind: cut_core::ActorKind::Agent,
        name: "client-b".into(),
        via: "mcp".into(),
        request: None,
    };
    let (r1, r2) = tokio::join!(
        dispatch(
            &f.state,
            "screen_record.make_editable",
            json!({"clip":"c1"}),
            a1
        ),
        dispatch(
            &f.state,
            "screen_record.make_editable",
            json!({"clip":"c1"}),
            a2
        ),
    );
    let ok_count = usize::from(r1.ok) + usize::from(r2.ok);
    assert_eq!(
        ok_count, 1,
        "one client must win and one refuse: r1={:?}, r2={:?}",
        r1.error, r2.error
    );
    let err = if r1.ok {
        r2.error.unwrap()
    } else {
        r1.error.unwrap()
    };
    assert!(err.message.contains("already editable"), "{err}");

    // 2. Restart server (new AppState) and reopen project cleanly.
    assert!(
        dispatch(
            &f.state,
            "screen_record.zoom_update",
            json!({"clip":"c1","id":"z1","scale":2.5}),
            test_actor()
        )
        .await
        .ok
    );
    let reopened_state = AppState::new();
    let opened = dispatch(
        &reopened_state,
        "project.open",
        json!({"path": f.project_dir}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);
    let listed = dispatch(
        &reopened_state,
        "screen_record.zoom_list",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(listed.ok, "{:?}", listed.error);
    let listed_body = listed.result.unwrap();
    assert_eq!(listed_body["stale"], json!(true));
    assert_eq!(listed_body["segments"][0]["scale"], json!(2.5));

    // 3. Symlinked events.json is refused safely.
    #[cfg(unix)]
    {
        let f_sym = setup_old_polished_project(false, 5000).await;
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            outside.path(),
            serde_json::to_vec(&events_track(5000)).unwrap(),
        )
        .unwrap();
        let ev = f_sym.capture_dir.join("events.json");
        std::fs::remove_file(&ev).unwrap();
        std::os::unix::fs::symlink(outside.path(), &ev).unwrap();
        let sym_err = dispatch(
            &f_sym.state,
            "screen_record.make_editable",
            json!({"clip":"c1"}),
            test_actor(),
        )
        .await
        .error
        .unwrap();
        assert!(sym_err.suggested_action.is_some(), "{sym_err}");
    }
}

#[tokio::test]
async fn make_editable_refuses_missing_old_or_tampered_inputs_with_plain_reasons() {
    // 1. Missing events.json
    let f = setup_old_polished_project(false, 5000).await;
    std::fs::remove_file(f.capture_dir.join("events.json")).unwrap();
    let err = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await
    .error
    .unwrap();
    assert!(err.suggested_action.is_some(), "must guide recovery: {err}");

    // 2. Older (pre-0.6.116) layout with events.plan.json and no plan-<sha256>.json
    let f_old = setup_old_polished_project(false, 5000).await;
    std::fs::remove_file(&f_old.plan_path).unwrap();
    std::fs::write(f_old.cache_dir.join("events.plan.json"), b"{}").unwrap();
    let err_old = dispatch(
        &f_old.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await
    .error
    .unwrap();
    let old_text = format!("{} {}", err_old.message, err_old.cause).to_lowercase();
    assert!(
        old_text.contains("older") || old_text.contains("0.6.116") || old_text.contains("cannot"),
        "expected older-version refusal, got: {err_old}"
    );
    assert!(err_old.suggested_action.is_some());

    // 3. Mismatched events duration vs plan duration (tampered events.json)
    let f_bad_ev = setup_old_polished_project(false, 5000).await;
    std::fs::write(
        f_bad_ev.capture_dir.join("events.json"),
        serde_json::to_vec_pretty(&events_track(9000)).unwrap(),
    )
    .unwrap();
    let err_ev = dispatch(
        &f_bad_ev.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await
    .error
    .unwrap();
    assert!(err_ev.suggested_action.is_some(), "{err_ev}");

    // 4. Planted captions.words_path pointing outside the project
    let f_cap = setup_old_polished_project(false, 5000).await;
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), r#"{"words":[]}"#).unwrap();
    let mut bad_plan = zoom_plan(5000, false, false, "");
    bad_plan.captions = Some(record_core::CaptionStyle {
        words_path: outside.path().to_string_lossy().into_owned(),
        font_px: 36.0,
        color: record_core::Rgba::new(255, 255, 255, 255),
        box_color: record_core::Rgba::new(0, 0, 0, 180),
    });
    let bad_bytes = serde_json::to_vec_pretty(&bad_plan).unwrap();
    let bad_sha = format!("{:x}", Sha256::digest(&bad_bytes));
    let bad_tag = &bad_sha[..16];
    let bad_plan_path = f_cap.cache_dir.join(format!("plan-{bad_sha}.json"));
    std::fs::write(&bad_plan_path, &bad_bytes).unwrap();
    let src_hash = cut_core::hash_file(&f_cap.capture_dir.join("source.mp4")).unwrap();
    let renamed_baked = f_cap
        .cache_dir
        .join(format!("{}_{bad_tag}_v.mp4", src_hash.replace(':', "_")));
    std::fs::rename(&f_cap.baked_path, &renamed_baked).unwrap();
    let baked_hash = cut_core::hash_file(&renamed_baked).unwrap();
    {
        let mut g = f_cap.state.project.write().await;
        let a = g.as_mut().unwrap().project.assets.get_mut("a1").unwrap();
        a.path = renamed_baked.to_string_lossy().into_owned();
        a.hash = baked_hash;
    }
    let err_cap = dispatch(
        &f_cap.state,
        "screen_record.make_editable",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await
    .error
    .unwrap();
    assert!(
        err_cap.suggested_action.is_some(),
        "must reject external captions path: {err_cap}"
    );
}

fn assert_reply_has_no_absolute_paths_and_path_free_cause(
    res: &VerbResult,
    project_dir: &std::path::Path,
) {
    assert!(!res.ok, "expected failure reply");
    let err = res.error.as_ref().expect("error payload");
    let serialized = serde_json::to_string(res).unwrap();
    let project_str = project_dir.to_string_lossy();
    assert!(
        !serialized.contains(project_str.as_ref()),
        "reply leaked project_dir '{}': {serialized}",
        project_str
    );
    for field in [
        Some(err.message.as_str()),
        Some(err.cause.as_str()),
        err.suggested_action.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        for token in field.split_whitespace() {
            let t =
                token.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '`' | ',' | ';'));
            let bytes = t.as_bytes();
            let looks_abs = t.starts_with('/')
                || t.starts_with("~/")
                || t.starts_with('\\')
                || t.contains('\\')
                || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic());
            assert!(
                !looks_abs,
                "error field contained absolute path token '{t}': {err:?}"
            );
        }
    }
    assert!(
        !err.cause.contains('/') && !err.cause.contains('\\'),
        "error cause must be path-free: {:?}",
        err.cause
    );
}

#[tokio::test]
async fn make_editable_errors_omit_absolute_paths_and_keep_path_free_cause() {
    // 1. Missing baked video file
    let f = setup_old_polished_project(false, 5000).await;
    std::fs::remove_file(&f.baked_path).unwrap();
    let missing_res = dispatch(
        &f.state,
        "screen_record.make_editable",
        json!({"clip": "c1"}),
        test_actor(),
    )
    .await;
    assert_reply_has_no_absolute_paths_and_path_free_cause(&missing_res, &f.project_dir);
    assert!(
        missing_res
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("cache/screen_record/"),
        "expected project-relative name in message: {:?}",
        missing_res.error
    );

    // 2. Non-polished asset path with private absolute directory (Unix and Windows spellings)
    for bad_path in [
        "/Users/private_person/Movies/secret_clip.mp4",
        r"C:\Users\private_person\Videos\secret_clip.mp4",
    ] {
        {
            let mut g = f.state.project.write().await;
            let a = g.as_mut().unwrap().project.assets.get_mut("a1").unwrap();
            a.path = bad_path.into();
        }
        let non_polished_res = dispatch(
            &f.state,
            "screen_record.make_editable",
            json!({"clip": "c1"}),
            test_actor(),
        )
        .await;
        assert_reply_has_no_absolute_paths_and_path_free_cause(&non_polished_res, &f.project_dir);
        assert!(
            !serde_json::to_string(&non_polished_res)
                .unwrap()
                .contains("private_person"),
            "leaked private path component: {:?}",
            non_polished_res.error
        );
    }

    // 3. Corrupt plan JSON (polish::load_plan / plan_cache_tag error path)
    let f_bad_plan = setup_old_polished_project(false, 5000).await;
    std::fs::write(&f_bad_plan.plan_path, b"{not valid json").unwrap();
    let bad_plan_res = dispatch(
        &f_bad_plan.state,
        "screen_record.make_editable",
        json!({"clip": "c1"}),
        test_actor(),
    )
    .await;
    assert_reply_has_no_absolute_paths_and_path_free_cause(&bad_plan_res, &f_bad_plan.project_dir);

    // 4. Symlinked events.json pointing outside project
    #[cfg(unix)]
    {
        let f_sym = setup_old_polished_project(false, 5000).await;
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            outside.path(),
            serde_json::to_vec(&events_track(5000)).unwrap(),
        )
        .unwrap();
        let ev = f_sym.capture_dir.join("events.json");
        std::fs::remove_file(&ev).unwrap();
        std::os::unix::fs::symlink(outside.path(), &ev).unwrap();
        let sym_res = dispatch(
            &f_sym.state,
            "screen_record.make_editable",
            json!({"clip": "c1"}),
            test_actor(),
        )
        .await;
        assert_reply_has_no_absolute_paths_and_path_free_cause(&sym_res, &f_sym.project_dir);
    }
}
