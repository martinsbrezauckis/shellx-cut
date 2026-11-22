// Capture-shaped fixtures: scene replay first, then the actual Studio log projection.
fn live_camera_plan(plan: &mut record_core::EditPlan, camera: &Path, kind: &str) {
    use record_core::{scene::ScenePreset, scene_projection::EditableSceneTimeline};
    use record_core::{
        AcceptedStartSnapshot, CatalogRevision, PipCorner, PipShape, PipSizePercent, PresenterPip,
        PresetRevision, SceneCatalog, SceneComposition, SceneEvent, SceneEventKind, SceneId,
        TimerConfig,
    };
    let id = |name: &str| SceneId::parse(name).unwrap();
    let revision = |n| PresetRevision::new(n).unwrap();
    let presenter = |corner, size, shape| {
        SceneComposition::PresenterPip(PresenterPip::new(
            corner,
            PipSizePercent::new(size).unwrap(),
            shape,
        ))
    };
    let presets = vec![
        ScenePreset::new(
            id("screen"),
            "Screen only",
            revision(1),
            SceneComposition::ScreenOnly,
            TimerConfig::Elapsed,
        )
        .unwrap(),
        ScenePreset::new(
            id("presenter-corner"),
            "Presenter corner",
            revision(2),
            presenter(PipCorner::BottomRight, 22, PipShape::Circle),
            TimerConfig::Elapsed,
        )
        .unwrap(),
        ScenePreset::new(
            id("demo-corner"),
            "Demo corner",
            revision(3),
            presenter(PipCorner::TopRight, 18, PipShape::RoundedRect),
            TimerConfig::Elapsed,
        )
        .unwrap(),
    ];
    let catalog = SceneCatalog::new(CatalogRevision::new(1).unwrap(), presets).unwrap();
    let snapshot = AcceptedStartSnapshot::accept(&catalog, &id("presenter-corner")).unwrap();
    let mut changes = vec![(0, SceneEventKind::TimerStart)];
    if kind == "scenes" {
        for (t, name, n) in [
            (500, "screen", 1),
            (1000, "presenter-corner", 2),
            (1750, "demo-corner", 3),
            (2500, "presenter-corner", 2),
        ] {
            changes.push((
                t,
                SceneEventKind::ActivateScene {
                    scene_id: id(name),
                    preset_revision: revision(n),
                },
            ));
        }
    }
    if kind == "timers" {
        changes.extend([
            (500, SceneEventKind::TimerPause),
            (1000, SceneEventKind::TimerResume),
            (1500, SceneEventKind::TimerReset),
            (1500, SceneEventKind::TimerStart),
            (2000, SceneEventKind::TimerReset),
            (2000, SceneEventKind::TimerStart),
            (2500, SceneEventKind::TimerEnd),
        ]);
    }
    let events: Vec<_> = changes
        .into_iter()
        .enumerate()
        .map(|(i, (t, kind))| SceneEvent {
            sequence: i as u64 + 1,
            logical_media_time_ms: t,
            snapshot_revision: snapshot.revision(),
            kind,
        })
        .collect();
    let scene =
        EditableSceneTimeline::from_replay(&snapshot, &events, 3000, "a".repeat(64)).unwrap();
    scene
        .apply_to_edit_plan(
            plan,
            Some(camera.display().to_string()),
            Some(record_core::CameraClockRange {
                first_frame_offset_ms: 0,
                end_frame_offset_ms: 3000,
            }),
        )
        .unwrap();
    let transform = |t, fields: serde_json::Value| {
        let mut value = json!({"t_ms":t,"source":"camera","kind":"transform"});
        for (k, v) in fields.as_object().unwrap() {
            value[k] = v.clone();
        }
        value
    };
    let edits = match kind {
        "position" => vec![
            transform(500, json!({"x":0.04,"y":0.04})),
            transform(1500, json!({"size":0.4})),
        ],
        "size" => vec![
            transform(500, json!({"size":0.4})),
            transform(1500, json!({"size":0.18})),
        ],
        "visibility" => vec![
            json!({"t_ms":500,"source":"camera","kind":"visibility","visible":false}),
            json!({"t_ms":1500,"source":"camera","kind":"visibility","visible":true}),
        ],
        "shape" => vec![transform(
            500,
            json!({"shape":"rounded_rect","radius":12.0}),
        )],
        "reset" => vec![
            transform(500, json!({"x":0.04,"y":0.04,"size":0.4})),
            json!({"t_ms":1500,"source":"camera","kind":"reset"}),
        ],
        "background" => {
            vec![json!({"t_ms":500,"source":"background","kind":"style","background":"none"})]
        }
        _ => vec![],
    };
    let path = camera
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(crate::screen_record_studio::STUDIO_EVENTS_FILENAME);
    std::fs::write(&path, b"").unwrap();
    let mut journal = crate::screen_record_studio_journal::StudioJournalOwner::default();
    for event in edits {
        journal
            .append(&path, serde_json::from_value(event).unwrap())
            .unwrap();
    }
    let log = crate::screen_record_studio::read_studio_events(&path).unwrap();
    crate::screen_record_studio::apply_studio_events_to_plan(plan, None, None, &log).unwrap();
    assert!(plan
        .scene_timeline
        .as_ref()
        .unwrap()
        .camera
        .iter()
        .any(|s| s.presenter.is_some()));
    plan.validate().unwrap();
}

async fn live_camera_case(kind: &'static str, build: fn(&mut record_core::EditPlan, &Path)) {
    let (root, state, reply) =
        camera_timeline_fixture_with_plan("separate", false, Some(build)).await;
    let (_broot, bstate, baked) =
        camera_timeline_fixture_with_plan("baked", false, Some(build)).await;
    camera_assert_baked_compatibility(&bstate, baked["camera_clip_id"].as_str().unwrap()).await;
    let dir = root.path().join("camera.cutproj");
    let original: record_core::EditPlan = serde_json::from_slice(
        &std::fs::read(dir.join("cache/screen_record/timeline/plan.json")).unwrap(),
    )
    .unwrap();
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let take = &store.project.recordings[0];
    let screen: record_core::EditPlan =
        serde_json::from_slice(&std::fs::read(&take.pending_plan).unwrap()).unwrap();
    screen.validate().unwrap();
    assert!(screen.webcam.is_none());
    let scenes = screen.scene_timeline.as_ref().unwrap();
    assert!(scenes.camera.iter().all(|s| s.presenter.is_none()));
    assert_eq!(
        scenes.screen,
        original.scene_timeline.as_ref().unwrap().screen
    );
    assert_eq!(
        scenes.timer,
        original.scene_timeline.as_ref().unwrap().timer
    );
    let webcam = original.webcam.as_ref().unwrap();
    for at in [250, 750, 1250, 1625, 2125, 2750] {
        let placement = webcam.placement_at(at, 640, 360);
        let d = (placement.size * 360.0).round().max(2.0);
        let x = (placement.x * 640.0).clamp(0.0, (640.0 - d).max(0.0));
        let y = (placement.y * 360.0).clamp(0.0, (360.0 - d).max(0.0));
        let frame = cut_media::render::extract_frame(
            &store.project,
            &cut_core::edl_from_project(&store.project),
            &store.dir,
            at,
            None,
        )
        .unwrap();
        let path = dir.join("live-frame.jpg");
        std::fs::write(&path, frame).unwrap();
        let rgb = camera_test_rgb(&path, 0.0);
        let baked_rgb = camera_test_rgb(
            Path::new(baked["baked"].as_str().unwrap()),
            at as f64 / 1000.0,
        );
        let screen_rgb = camera_test_rgb(
            Path::new(reply["baked"].as_str().unwrap()),
            at as f64 / 1000.0,
        );
        // Sample the centre and the inner/outer radius on both axes, avoiding
        // the known inset-vs-centred ring and first-shape-only corners.
        for (dx, dy) in [
            (0.0, 0.0),
            (-0.25, 0.0),
            (0.25, 0.0),
            (0.0, -0.25),
            (0.0, 0.25),
            (-0.65, 0.0),
            (0.65, 0.0),
            (0.0, -0.65),
            (0.0, 0.65),
        ] {
            let px = (x + d * (0.5 + dx)).round() as i64;
            let py = (y + d * (0.5 + dy)).round() as i64;
            if !(0..640).contains(&px) || !(0..360).contains(&py) {
                continue;
            }
            let i = (py as usize * 640 + px as usize) * 3;
            let red = |p: &[u8]| p[i] > 180 && p[i + 1] < 60;
            assert_eq!(red(&rgb), red(&baked_rgb), "{kind} at {at}, {px},{py}");
        }
        let i = ((y + d / 2.0) as usize * 640 + (x + d / 2.0) as usize) * 3;
        assert!(
            !(screen_rgb[i] > 180 && screen_rgb[i + 1] < 60),
            "screen bake has a camera at {at}"
        );
    }
    drop(guard);
    let clip = reply["clip_id"].as_str().unwrap();
    assert!(
        dispatch(
            &state,
            "screen_record.zoom_add",
            json!({"clip":clip,"at_ms":500,"duration_ms":2000}),
            test_actor()
        )
        .await
        .ok
    );
    let refresh = dispatch(
        &state,
        "screen_record.take_refresh",
        json!({"clip":clip}),
        test_actor(),
    )
    .await;
    assert!(refresh.ok, "{:?}", refresh.error);
    camera_wait_job(&state, refresh.result.unwrap()["job_id"].as_str().unwrap()).await;
    // Feed an ORIGINAL camera-bearing pending plan to copy, as well as the
    // applied bubble-free one, so the copy's rewrite path must normalize it too.
    let mut pending = dir.join("cache/screen_record/timeline/plan.json");
    if kind == "size" {
        // A screen plan written by the old separate-camera implementation.
        let mut legacy = original.clone();
        legacy.webcam = None;
        assert!(legacy.validate().is_err());
        pending = dir.join("cache/screen_record/timeline/legacy-screen-plan.json");
        std::fs::write(&pending, serde_json::to_vec(&legacy).unwrap()).unwrap();
    }
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let take = store.project.recordings[0].clone();
        store
            .apply(
                "screen_record.zoom_set",
                json!({"take":take.id,"segments":[],"pending_plan":pending}),
                test_actor(),
                None,
            )
            .unwrap();
    }
    let dest = root.path().join("copies");
    std::fs::create_dir(&dest).unwrap();
    let plan = dispatch(
        &state,
        "project.package_plan",
        json!({"destination":dest,"name":"copy"}),
        test_actor(),
    )
    .await;
    assert!(plan.ok, "{:?}", plan.error);
    let copy = dispatch(
        &state,
        "project.package_create",
        json!({"destination":dest,"name":"copy","plan_hash":plan.result.unwrap()["plan_hash"]}),
        test_actor(),
    )
    .await;
    assert!(copy.ok, "{:?}", copy.error);
    camera_wait_job(&state, copy.result.unwrap()["job_id"].as_str().unwrap()).await;
    let copied = cut_core::ProjectStore::open(&dest.join("copy.cutproj")).unwrap();
    let take = &copied.project.recordings[0];
    let pending: record_core::EditPlan =
        serde_json::from_slice(&std::fs::read(copied.dir.join(&take.pending_plan)).unwrap())
            .unwrap();
    pending.validate().unwrap();
    assert!(pending.webcam.is_none());
    assert!(pending
        .scene_timeline
        .unwrap()
        .camera
        .iter()
        .all(|s| s.presenter.is_none()));
}

macro_rules! live_camera_regression {
    ($name:ident,$kind:literal) => {
        #[test]
        fn $name() {
            fn build(plan: &mut record_core::EditPlan, camera: &Path) {
                live_camera_plan(plan, camera, $kind);
            }
            std::thread::Builder::new()
                .stack_size(16 * 1024 * 1024)
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(live_camera_case($kind, build))
                })
                .unwrap()
                .join()
                .unwrap();
        }
    };
}
live_camera_regression!(
    live_camera_position_and_size_polish_refresh_and_copy,
    "position"
);
live_camera_regression!(live_camera_size_polish_refresh_and_copy, "size");
live_camera_regression!(live_camera_visibility_polish_refresh_and_copy, "visibility");
live_camera_regression!(live_camera_shape_polish_refresh_and_copy, "shape");
live_camera_regression!(live_camera_reset_polish_refresh_and_copy, "reset");
live_camera_regression!(live_camera_background_polish_refresh_and_copy, "background");
live_camera_regression!(live_camera_scene_switches_polish_refresh_and_copy, "scenes");
live_camera_regression!(live_camera_timer_actions_polish_refresh_and_copy, "timers");

async fn camera_assert_baked_compatibility(state: &AppState, clip: &str) {
    let reply = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(reply.ok, "{:?}", reply.error);
    let reply = reply.result.unwrap();
    let camera_track = reply["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["clips"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == clip)
        })
        .unwrap();
    assert_eq!(camera_track["visible"], false);
    assert!(camera_track["id"]
        .as_str()
        .unwrap()
        .starts_with("v_camera_"));
    for c in reply["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
    {
        assert_ne!(
            c["recording"]["camera"], true,
            "baked clips must not be marked editable cameras"
        );
    }
    // Polish can leave import/proxy work running in this private AppState.
    // Like wait_for_owned_jobs_to_settle, wait for terminal jobs BEFORE taking
    // the snapshot, without holding the project lock. Rescan for child jobs.
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            if state.jobs.list().iter().all(|job| {
                matches!(
                    job.state,
                    crate::jobs::JobState::Done | crate::jobs::JobState::Failed
                )
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("camera fixture jobs did not settle before the refusal snapshot");
    let before = {
        let guard = state.project.read().await;
        let project = &guard.as_ref().unwrap().project;
        let take = &project.recordings[0];
        assert!(take.camera_look.is_none());
        assert_eq!(take.camera_clip.as_deref(), Some(clip));
        assert!(
            take.camera_asset.is_some(),
            "ownership/copy provenance retained"
        );
        serde_json::to_value(project).unwrap()
    };
    let reset = dispatch(
        state,
        "screen_record.camera_reset",
        json!({"clip":clip}),
        test_actor(),
    )
    .await;
    assert!(!reset.ok);
    assert_eq!(
        reset.error.unwrap().message,
        "this recording's camera is part of the video"
    );
    let guard = state.project.read().await;
    assert_eq!(
        serde_json::to_value(&guard.as_ref().unwrap().project).unwrap(),
        before
    );
}
