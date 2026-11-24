use super::*;

#[test]
fn recording_edits_replay_undo_and_preserve_clip_fields() {
    let d = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
    s.apply_atomic_media_insert_plan(
        &"a".repeat(64),
        vec![asset()],
        vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,1000]})],
        actor(),
        None,
    )
    .unwrap();
    let take = json!({"id":"take-1","capture_folder":"cache/take","source":"source.mp4","events":"events.json","pending_plan":"p1","applied_plan":"p1","clip_ids":["c1"],"asset":"a1","segments":[]});
    let before = serde_json::to_vec(&s.project).unwrap();
    assert!(!String::from_utf8(before.clone())
        .unwrap()
        .contains("recordings"));
    assert_eq!(
        before,
        serde_json::to_vec(&serde_json::from_slice::<Project>(&before).unwrap()).unwrap()
    );
    let checkpoint = s.checkpoint("before-take", actor(), None).unwrap();
    s.apply(
        "screen_record.take_link",
        json!({"recording":take}),
        actor(),
        None,
    )
    .unwrap();
    s.undo(actor()).unwrap();
    assert!(s.project.recordings.is_empty());
    s.redo(actor()).unwrap();
    s.apply(
        "screen_record.zoom_set",
        json!({"take":"take-1","segments":[],"pending_plan":"p2"}),
        actor(),
        None,
    )
    .unwrap();
    assert!(s.project.recordings[0].stale());
    s.undo(actor()).unwrap();
    assert!(!s.project.recordings[0].stale());
    s.redo(actor()).unwrap();
    s.apply(
        "edit.split",
        json!({"track":"v1","at_ms":500}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.speed",
        json!({"clip":"c1","factor":2.0}),
        actor(),
        None,
    )
    .unwrap();
    let old = s.project.tracks.clone();
    let refresh = json!({"take":"take-1","asset_id":"a2","asset":Asset { path:"/new.mp4".into(), hash:"sha256:new".into(), ..asset() },"applied_plan":"p2"});
    s.apply("screen_record.take_refresh", refresh, actor(), None)
        .unwrap();
    assert!(!s.project.recordings[0].stale());
    let mut expected = old.clone();
    for track in &mut expected {
        for clip in &mut track.clips {
            if let Clip::Media(m) = clip {
                m.asset = "a2".into();
            }
        }
    }
    assert_eq!(s.project.tracks, expected);
    let dir = s.dir.clone();
    drop(s);
    let mut s = ProjectStore::open(&dir).unwrap();
    assert_eq!(s.project.tracks, expected);
    s.undo(actor()).unwrap();
    assert_eq!(s.project.tracks, old);
    assert!(s.project.recordings[0].stale());
    s.redo(actor()).unwrap();
    assert_eq!(s.project.tracks, expected);
    s.revert(&checkpoint.0.id, actor()).unwrap();
    assert!(s.project.recordings.is_empty());
}

#[test]
fn make_editable_activates_legacy_camera_replays_and_undoes_in_one_op() {
    let d = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(d.path(), "legacy-cam", None).unwrap();
    let capture = d.path().join("cache/screen_record/cap_old");
    std::fs::create_dir_all(capture.join("camera")).unwrap();
    let cam_bytes = b"fake-cam-mp4";
    std::fs::write(capture.join("camera/camera_01.mp4"), cam_bytes).unwrap();
    let cam_hash = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(cam_bytes))
    };
    let cam_asset = Asset {
        path: capture
            .join("camera/camera_01.mp4")
            .to_string_lossy()
            .into_owned(),
        hash: format!("sha256:{cam_hash}"),
        probe: Some(json!({"kind":"video","duration_ms":4000,"width":640,"height":480})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let cam_track_id = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"shellx-record/camera-artifact/1");
        h.update([0]);
        h.update(b"cap_old");
        h.update([0]);
        h.update(b"camera_01");
        format!("v_camera_{}", &format!("{:x}", h.finalize())[..32])
    };
    s.apply_atomic_media_insert_plan(
        &"1".repeat(64),
        vec![asset()],
        vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,4000]})],
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.add_track",
        json!({"kind":"video","id":cam_track_id}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.track_visible",
        json!({"track":cam_track_id,"on":false}),
        actor(),
        None,
    )
    .unwrap();
    s.apply_atomic_media_insert_plan(
        &"2".repeat(64),
        vec![cam_asset],
        vec![json!({"track":cam_track_id,"at_ms":0,"src_range_ms":[0,4000]})],
        actor(),
        None,
    )
    .unwrap();
    let pre_tracks = s.project.tracks.clone();
    assert!(!s.project.track(&cam_track_id).unwrap().visible);
    let payload = json!({
        "clip": "c1",
        "recording": {
            "id": "take-old-1",
            "capture_folder": capture,
            "source": capture.join("source.mp4"),
            "events": capture.join("events.json"),
            "pending_plan": d.path().join("cache/screen_record/plan-screen.json"),
            "applied_plan": d.path().join("cache/screen_record/plan-baked.json"),
            "clip_ids": ["c1"],
            "asset": "a1",
            "camera_asset": "a2",
            "camera_clip": "c2",
            "camera_artifact": {
                "schema": "shellx-record/camera-artifact/1",
                "capture_id": "cap_old",
                "artifact_id": "camera_01",
                "video": "camera/camera_01.mp4",
                "clock": { "first_frame_offset_ms": 0, "end_frame_offset_ms": 4000 },
                "media": { "sha256": cam_hash, "width": 640, "height": 480, "fps_num": 30, "fps_den": 1, "duration_ms": 4000, "frame_count": 120 },
                "terminal_state": "complete"
            },
            "camera_look": {
                "transform": { "x": 0.75, "y": 0.72, "scale": 0.25, "opacity": 1.0, "shape": "circle" },
                "keyframes": []
            },
            "segments": [{
                "id": "z1",
                "start_ms": 500,
                "end_ms": 2000,
                "scale": 2.0,
                "cx": 0.5,
                "cy": 0.5,
                "ramp_in_ms": 550,
                "ramp_out_ms": 500,
                "origin": "auto"
            }]
        }
    });
    s.apply("screen_record.make_editable", payload, actor(), None)
        .unwrap();
    assert_eq!(s.project.recordings.len(), 1);
    assert!(s.project.track(&cam_track_id).unwrap().visible);
    let Clip::Media(cam_clip) = &s.project.track(&cam_track_id).unwrap().clips[0] else {
        panic!("camera clip must be media");
    };
    assert!(cam_clip.transform.is_some());
    let Clip::Media(screen_clip) = &s.project.track("v1").unwrap().clips[0] else {
        panic!("screen clip must be media");
    };
    assert_eq!(screen_clip.asset, "a1");

    // One Undo removes the take, hides the camera track again, and clears its overlay transform.
    s.undo(actor()).unwrap();
    assert!(s.project.recordings.is_empty());
    assert_eq!(s.project.tracks, pre_tracks);

    // Redo + clean open replay deterministically restore editable state.
    s.redo(actor()).unwrap();
    let dir = s.dir.clone();
    drop(s);
    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.project.recordings.len(), 1);
    assert!(reopened.project.track(&cam_track_id).unwrap().visible);
    let Clip::Media(re_cam) = &reopened.project.track(&cam_track_id).unwrap().clips[0] else {
        panic!("camera clip must be media");
    };
    assert!(re_cam.transform.is_some());
}
