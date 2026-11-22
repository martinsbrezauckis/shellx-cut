use super::*;

#[tokio::test]
async fn zoom_auto_bounds_event_bytes_clicks_and_duration() {
    use std::io::Write;

    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("bounded-zoom.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"bounded-zoom","dir":dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let capture = crate::screen_record::screen_record_cache_dir(&dir)
        .unwrap()
        .join("take");
    std::fs::create_dir_all(&capture).unwrap();
    let events = record_core::fixtures::generate("click-walkthrough").unwrap();
    let plan = record_engine::autoedit(&events, &Default::default());
    std::fs::write(
        capture.join("plan.json"),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    let event_path = capture.join("events.json");
    std::fs::write(&event_path, serde_json::to_vec(&events).unwrap()).unwrap();
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let asset: cut_core::Asset = serde_json::from_value(json!({
            "path":capture.join("bake.mp4"),"hash":"sha256:bake",
            "probe":{"duration_ms":events.duration_ms}
        }))
        .unwrap();
        store
            .apply_atomic_media_insert_plan(
                &"f".repeat(64),
                vec![asset],
                vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,events.duration_ms]})],
                test_actor(),
                None,
            )
            .unwrap();
        store
            .apply(
                "screen_record.take_link",
                json!({"recording":{
                    "id":"take1","capture_folder":capture,"source":capture.join("source.mp4"),
                    "events":event_path,"pending_plan":capture.join("plan.json"),
                    "applied_plan":capture.join("plan.json"),"clip_ids":["c1"],
                    "asset":"a1","segments":[]
                }}),
                test_actor(),
                None,
            )
            .unwrap();
    }
    let mut file = std::fs::File::create(&event_path).unwrap();
    file.write_all(&serde_json::to_vec(&events).unwrap())
        .unwrap();
    // On the heap: a megabyte array inside this async test overflows the test thread's stack.
    let spaces = vec![b' '; 1024 * 1024];
    for _ in 0..65 {
        file.write_all(&spaces).unwrap();
    }
    drop(file);
    let oversized = dispatch(
        &state,
        "screen_record.zoom_auto",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(!oversized.ok);
    assert!(oversized.error.unwrap().message.contains("64 MiB limit"));

    let mut crowded = events.clone();
    crowded.clicks = vec![events.clicks[0]; 40_001];
    std::fs::write(&event_path, serde_json::to_vec(&crowded).unwrap()).unwrap();
    let too_many = dispatch(
        &state,
        "screen_record.zoom_auto",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(!too_many.ok);
    assert!(too_many.error.unwrap().message.contains("zoom_auto limits"));

    let mut too_long = events.clone();
    too_long.duration_ms = 7 * 24 * 60 * 60 * 1_000 + 1;
    std::fs::write(&event_path, serde_json::to_vec(&too_long).unwrap()).unwrap();
    let too_long = dispatch(
        &state,
        "screen_record.zoom_auto",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(!too_long.ok);
    assert!(too_long.error.unwrap().message.contains("plan duration"));
}

#[tokio::test]
async fn render_final_guards_the_snapshot_after_an_await() {
    use crate::dispatch::rendering::final_snapshot_gate::{self, Gate};
    use std::sync::Arc;

    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::new());
    let dir = root.path().join("race.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"race","dir":dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        guard.as_mut().unwrap().project.recordings.push(
            serde_json::from_value(json!({
                "id":"take","capture_folder":"capture","source":"source.mp4",
                "events":"events.json","pending_plan":"applied.json",
                "applied_plan":"applied.json","clip_ids":[],"asset":"unused",
                "segments":[]
            }))
            .unwrap(),
        );
    }
    let gate = Gate {
        path: "exports/race.mp4".into(),
        reached: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    };
    let jobs_before = state.jobs.list().len();
    final_snapshot_gate::install(Some(gate.clone()));
    let running = Arc::clone(&state);
    let render = tokio::spawn(async move {
        crate::dispatch::rendering::render_final(
            &running,
            json!({"path":"exports/race.mp4","hardware":"off"}),
            test_actor(),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), gate.reached.notified())
        .await
        .unwrap();
    {
        let mut guard = state.project.write().await;
        guard.as_mut().unwrap().project.recordings[0].pending_plan = "edited.json".into();
    }
    gate.release.notify_one();
    let result = render.await.unwrap();
    final_snapshot_gate::install(None);
    let error = result.unwrap_err();
    assert!(error.message.contains("Update video first"), "{error:?}");
    assert_eq!(state.jobs.list().len(), jobs_before);
    assert!(!dir.join("receipts/.render_001.reserved").exists());
    let allowed = crate::dispatch::rendering::render_final(
        &state,
        json!({"path":"exports/allowed.mp4","dry_run":true,"hardware":"off","allow_stale_recordings":true}),
        test_actor(),
    )
    .await;
    assert!(!allowed
        .as_ref()
        .err()
        .is_some_and(|error| error.message.contains("Update video first")));
}

#[tokio::test]
async fn recording_zoom_verbs_stale_and_old_project_guard() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("zooms.cutproj");
    let result = dispatch(
        &state,
        "project.create",
        json!({"name":"zooms","dir":dir}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
    let capture = crate::screen_record::screen_record_cache_dir(&dir)
        .unwrap()
        .join("take");
    std::fs::create_dir_all(&capture).unwrap();
    let events = record_core::fixtures::generate("click-walkthrough").unwrap();
    let plan = record_engine::autoedit(&events, &Default::default());
    let original_bytes = serde_json::to_vec_pretty(&plan).unwrap();
    std::fs::write(capture.join("plan.json"), &original_bytes).unwrap();
    std::fs::write(
        capture.join("events.json"),
        serde_json::to_vec(&events).unwrap(),
    )
    .unwrap();
    let segments = record_core::zoom_segments::keys_to_segments(&plan.zoom).unwrap();
    let core_segments: Vec<cut_core::recording::ZoomSegment> =
        serde_json::from_value(serde_json::to_value(&segments).unwrap()).unwrap();
    let back: Vec<record_core::zoom_segments::ZoomSegment> =
        serde_json::from_value(serde_json::to_value(core_segments).unwrap()).unwrap();
    assert_eq!(segments, back);
    let rebuilt =
        record_core::zoom_segments::segments_to_keys(&segments, events.duration_ms).unwrap();
    assert_eq!(
        serde_json::to_vec(&plan.zoom).unwrap(),
        serde_json::to_vec(&rebuilt).unwrap()
    );
    {
        let mut guard = state.project.write().await;
        let s = guard.as_mut().unwrap();
        let asset:cut_core::Asset = serde_json::from_value(json!({"path":capture.join("bake.mp4"),"hash":"sha256:bake","probe":{"duration_ms":events.duration_ms}})).unwrap();
        s.apply_atomic_media_insert_plan(
            &"f".repeat(64),
            vec![asset],
            vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,events.duration_ms]})],
            test_actor(),
            None,
        )
        .unwrap();
    }
    let old = dispatch(
        &state,
        "screen_record.zoom_list",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(!old.ok);
    let snapshot = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert!(snapshot["tracks"][0]["clips"][0].get("recording").is_none());
    {
        let mut guard = state.project.write().await;
        guard.as_mut().unwrap().apply("screen_record.take_link",json!({"recording":{"id":"take1","capture_folder":capture,"source":capture.join("source.mp4"),"events":capture.join("events.json"),"pending_plan":capture.join("plan.json"),"applied_plan":capture.join("plan.json"),"clip_ids":["c1"],"asset":"a1","segments":segments}}),test_actor(),None).unwrap();
    }
    let list = dispatch(
        &state,
        "screen_record.zoom_list",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(list.ok, "{:?}", list.error);
    assert!(!list.result.unwrap()["stale"].as_bool().unwrap());
    let mut edited = serde_json::to_value(&segments).unwrap();
    edited[0]["scale"] = json!(3.0);
    let pinned = dispatch(
        &state,
        "screen_record.zoom_set",
        json!({"clip":"c1","segments":edited}),
        test_actor(),
    )
    .await;
    assert!(pinned.ok, "{:?}", pinned.error);
    assert_eq!(pinned.result.unwrap()["segments"][0]["origin"], "manual");
    let set = dispatch(
        &state,
        "screen_record.zoom_set",
        json!({"clip":"c1","segments":[]}),
        test_actor(),
    )
    .await;
    assert!(set.ok, "{:?}", set.error);
    assert_eq!(set.result.unwrap()["stale"], true);
    let add = dispatch(
        &state,
        "screen_record.zoom_add",
        json!({"clip":"c1","at_ms":1000}),
        test_actor(),
    )
    .await;
    assert!(add.ok, "{:?}", add.error);
    let id = add.result.unwrap()["segment"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let update = dispatch(
        &state,
        "screen_record.zoom_update",
        json!({"clip":"c1","id":id,"scale":3.0}),
        test_actor(),
    )
    .await;
    assert!(update.ok, "{:?}", update.error);
    {
        let mut guard = state.project.write().await;
        let s = guard.as_mut().unwrap();
        s.apply(
            "edit.trim",
            json!({"clip":"c1","src_in_ms":1500,"src_out_ms":3500}),
            test_actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.speed",
            json!({"clip":"c1","factor":2}),
            test_actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.reverse",
            json!({"clip":"c1","on":true}),
            test_actor(),
            None,
        )
        .unwrap();
    }
    let mapped = dispatch(
        &state,
        "screen_record.zoom_list",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await
    .result
    .unwrap();
    assert_eq!(
        mapped["segments"][0]["timeline"],
        json!({"start_ms":250,"end_ms":1000})
    );
    let auto = dispatch(
        &state,
        "screen_record.zoom_auto",
        json!({"clip":"c1"}),
        test_actor(),
    )
    .await;
    assert!(auto.ok, "{:?}", auto.error);
    assert!(auto.result.unwrap()["segments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == id && s["origin"] == "manual"));
    let remove = dispatch(
        &state,
        "screen_record.zoom_remove",
        json!({"clip":"c1","id":id}),
        test_actor(),
    )
    .await;
    assert!(remove.ok, "{:?}", remove.error);
    let snapshot = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(
        snapshot["tracks"][0]["clips"][0]["recording"]["stale"],
        true
    );
    let refused = dispatch(&state, "edit.auto_zoom", json!({"clip":"c1"}), test_actor()).await;
    assert!(!refused.ok);
    assert!(refused
        .error
        .unwrap()
        .message
        .contains("this recording has its own zooms"));
    let refused = dispatch(
        &state,
        "render.final",
        json!({"dry_run":true}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    assert!(refused
        .error
        .unwrap()
        .message
        .contains("Update video first"));
    assert_eq!(
        std::fs::read(capture.join("plan.json")).unwrap(),
        original_bytes
    );
}

#[tokio::test]
async fn recording_refresh_bakes_and_preserves_split_trim_speed_or_refuses_duration() {
    for old_duration in [3000, 1000] {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let dir = root.path().join("refresh.cutproj");
        assert!(
            dispatch(
                &state,
                "project.create",
                json!({"name":"refresh","dir":dir}),
                test_actor()
            )
            .await
            .ok
        );
        let capture = crate::screen_record::screen_record_cache_dir(&dir)
            .unwrap()
            .join("refresh");
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let old_bake = capture.join("old.mp4");
        std::fs::copy(&source, &old_bake).unwrap();
        let plan = record_core::EditPlan::empty(320, 180, 3000, 30.0);
        let plan_path = capture.join("plan.json");
        std::fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
        std::fs::write(capture.join("events.json"), b"{}").unwrap();
        {
            let mut guard = state.project.write().await;
            let s = guard.as_mut().unwrap();
            let asset: cut_core::Asset = serde_json::from_value(
                json!({"path":old_bake,"hash":"sha256:old","probe":{"duration_ms":old_duration}}),
            )
            .unwrap();
            s.apply_atomic_media_insert_plan(
                &"e".repeat(64),
                vec![asset],
                vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,1000]})],
                test_actor(),
                None,
            )
            .unwrap();
            s.apply("screen_record.take_link",json!({"recording":{"id":"refresh","capture_folder":capture,"source":source,"events":capture.join("events.json"),"pending_plan":plan_path,"applied_plan":plan_path,"clip_ids":["c1"],"asset":"a1","segments":[]}}),test_actor(),None).unwrap();
            s.apply(
                "edit.split",
                json!({"track":"v1","at_ms":500}),
                test_actor(),
                None,
            )
            .unwrap();
            s.apply(
                "edit.trim",
                json!({"clip":"c1","src_in_ms":100}),
                test_actor(),
                None,
            )
            .unwrap();
            s.apply(
                "edit.speed",
                json!({"clip":"c1","factor":2}),
                test_actor(),
                None,
            )
            .unwrap();
        }
        let added = dispatch(
            &state,
            "screen_record.zoom_add",
            json!({"clip":"c1","at_ms":500,"duration_ms":2000}),
            test_actor(),
        )
        .await;
        assert!(added.ok, "{:?}", added.error);
        let before = state.project.read().await.as_ref().unwrap().project.clone();
        let refreshed = dispatch(
            &state,
            "screen_record.take_refresh",
            json!({"clip":"c1"}),
            test_actor(),
        )
        .await;
        assert!(refreshed.ok, "{:?}", refreshed.error);
        let job = refreshed.result.unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let terminal = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let status = dispatch(&state, "jobs.status", json!({"job_id":job}), test_actor())
                    .await
                    .result
                    .unwrap();
                if status["state"] == "done" || status["state"] == "failed" {
                    break status;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        let guard = state.project.read().await;
        let after = &guard.as_ref().unwrap().project;
        if old_duration == 1000 {
            assert_eq!(terminal["state"], "failed", "{terminal}");
            assert!(terminal.to_string().contains("more than one frame"));
            assert_eq!(before, *after);
        } else {
            assert_eq!(terminal["state"], "done", "{terminal}");
            assert!(!after.recordings[0].stale());
            let mut expected = before.tracks.clone();
            for track in &mut expected {
                for clip in &mut track.clips {
                    if let cut_core::Clip::Media(m) = clip {
                        m.asset = after.recordings[0].asset.clone();
                    }
                }
            }
            assert_eq!(expected, after.tracks);
        }
    }
}

#[test]
fn recording_refresh_duration_tolerance() {
    use crate::screen_record::take_refresh_job::check_duration;
    assert!(check_duration(1000, 1033, 30.0).is_ok());
    assert!(check_duration(1000, 1034, 30.0).is_err());
}
