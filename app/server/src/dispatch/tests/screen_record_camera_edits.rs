fn edit_camera_rgb(project: &cut_core::Project, dir: &Path, at: u64) -> Vec<u8> {
    let bytes = cut_media::render::extract_frame(
        project,
        &cut_core::edl_from_project(project),
        dir,
        at,
        None,
    )
    .unwrap();
    let path = dir.join("edit-camera-frame.jpg");
    std::fs::write(&path, bytes).unwrap();
    camera_test_rgb(&path, 0.0)
}
fn edit_camera_bounds(rgb: &[u8]) -> [usize; 4] {
    let mut b = [640, 360, 0, 0];
    for y in 0..360 {
        for x in 0..640 {
            let i = (y * 640 + x) * 3;
            if rgb[i] > 180 && rgb[i + 1] < 60 {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
    }
    b
}
async fn camera_manual_placement_case() {
    fn build(p: &mut record_core::EditPlan, c: &Path) {
        live_camera_plan(p, c, "position");
    }
    let (_root, state, reply) =
        camera_timeline_fixture_with_plan("separate", false, Some(build)).await;
    let clip = reply["camera_clip_id"].as_str().unwrap();
    assert!(dispatch(&state,"edit.keyframe",json!({"clip":clip,"param":"opacity","interp":"hold","points":[{"t_ms":0,"value":1.0},{"t_ms":2500,"value":0.0}]}),test_actor()).await.ok);
    let original = {
        let g = state.project.read().await;
        g.as_ref().unwrap().project.clone()
    };
    let (track, index) = original.find_clip(clip).unwrap();
    let cut_core::Clip::Media(media) = &original.track(track).unwrap().clips[index] else {
        panic!()
    };
    let mut transform = media.transform.clone().unwrap();
    let keys = media.keyframes.clone();
    assert!(keys.iter().any(|k| k.param == cut_core::KfParam::PosX));
    let before_widths = {
        let g = state.project.read().await;
        let s = g.as_ref().unwrap();
        [750, 1750].map(|at| {
            let b = edit_camera_bounds(&edit_camera_rgb(&s.project, &s.dir, at));
            b[2] - b[0] + 1
        })
    };
    transform.scale *= 0.75;
    let mut args = serde_json::to_value(&transform).unwrap();
    args["clip"] = json!(clip);
    assert!(
        dispatch(&state, "edit.transform", args, test_actor())
            .await
            .ok
    );
    let before_move = {
        let g = state.project.read().await;
        let s = g.as_ref().unwrap();
        for (i, at) in [750, 1750].into_iter().enumerate() {
            let b = edit_camera_bounds(&edit_camera_rgb(&s.project, &s.dir, at));
            assert!(
                ((b[2] - b[0] + 1) as f64 - before_widths[i] as f64 * 0.75).abs() <= 8.0,
                "size at {at}: bounds {b:?}, original widths {before_widths:?}"
            );
        }
        edit_camera_rgb(&s.project, &s.dir, 1750)
    };
    transform.x = 0.5;
    transform.y = 0.4;
    let mut args = serde_json::to_value(&transform).unwrap();
    args["clip"] = json!(clip);
    assert!(
        dispatch(&state, "edit.transform", args, test_actor())
            .await
            .ok
    );
    {
        let g = state.project.read().await;
        let s = g.as_ref().unwrap();
        let (t, i) = s.project.find_clip(clip).unwrap();
        let cut_core::Clip::Media(m) = &s.project.track(t).unwrap().clips[i] else {
            panic!()
        };
        assert_eq!(
            m.keyframes,
            keys.iter()
                .filter(|k| !matches!(k.param, cut_core::KfParam::PosX | cut_core::KfParam::PosY))
                .cloned()
                .collect::<Vec<_>>()
        );
        let b = edit_camera_bounds(&edit_camera_rgb(&s.project, &s.dir, 1750));
        assert!(
            b[0] >= 315 && b[1] >= 140,
            "manual position must replace recorded movement: {b:?}"
        );
        assert_eq!(
            edit_camera_bounds(&edit_camera_rgb(&s.project, &s.dir, 2750))[0],
            640,
            "visibility key survives hand placement"
        );
    }
    assert!(
        dispatch(&state, "project.undo", json!({}), test_actor())
            .await
            .ok
    );
    {
        let g = state.project.read().await;
        let s = g.as_ref().unwrap();
        let (t, i) = s.project.find_clip(clip).unwrap();
        let cut_core::Clip::Media(m) = &s.project.track(t).unwrap().clips[i] else {
            panic!()
        };
        assert_eq!(m.keyframes, keys);
        let restored = edit_camera_rgb(&s.project, &s.dir, 1750);
        assert_eq!(
            edit_camera_bounds(&restored),
            edit_camera_bounds(&before_move)
        );
        // Repeated JPEG/source decoding can round a few channels differently.
        // Require identical placement and tight pixel agreement, not byte identity.
        assert!(restored
            .iter()
            .zip(&before_move)
            .all(|(a, b)| a.abs_diff(*b) <= 4));
        let mean = restored
            .iter()
            .zip(&before_move)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / restored.len() as f64;
        assert!(mean < 0.01, "undo mean pixel difference {mean}");
    }
    assert!(
        dispatch(&state, "project.redo", json!({}), test_actor())
            .await
            .ok
    );
    {
        let g = state.project.read().await;
        let s = g.as_ref().unwrap();
        let b = edit_camera_bounds(&edit_camera_rgb(&s.project, &s.dir, 1750));
        assert!(b[0] >= 315 && b[1] >= 140);
    }
    assert!(
        dispatch(
            &state,
            "screen_record.camera_reset",
            json!({"clip":clip}),
            test_actor()
        )
        .await
        .ok
    );
    {
        let g = state.project.read().await;
        let s = g.as_ref().unwrap();
        let b = edit_camera_bounds(&edit_camera_rgb(&s.project, &s.dir, 1750));
        assert!(b[0] < 40 && b[1] < 40);
    }
    let mut ordinary = original.clone();
    ordinary.recordings.clear();
    cut_core::edit::transform(&mut ordinary, clip, transform).unwrap();
    let (t, i) = ordinary.find_clip(clip).unwrap();
    let cut_core::Clip::Media(m) = &ordinary.track(t).unwrap().clips[i] else {
        panic!()
    };
    assert_eq!(
        m.keyframes, keys,
        "ordinary overlays retain their position keys"
    );
}
async fn camera_split_picture_case() {
    fn build(p: &mut record_core::EditPlan, c: &Path) {
        let log=serde_json::from_value(json!({"version":1,"events":[{"t_ms":10000,"source":"camera","kind":"transform","x":0.04,"y":0.04,"size":0.4}]})).unwrap();
        crate::screen_record_studio::apply_studio_events_to_plan(
            p,
            Some(c.display().to_string()),
            None,
            &log,
        )
        .unwrap();
    }
    let (_root, state, reply) =
        camera_timeline_fixture_duration("separate", false, Some(build), 20000).await;
    let clip = reply["camera_clip_id"].as_str().unwrap();
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    for ordinary in [false, true] {
        for interp in ["hold", "linear", "ease_in_out_quad"] {
            let mut original = store.project.clone();
            if ordinary {
                original.recordings.clear();
            }
            let (track, index) = original.find_clip(clip).unwrap();
            let track = track.to_owned();
            let cut_core::Clip::Media(m) = &mut original.track_mut(&track).unwrap().clips[index]
            else {
                panic!()
            };
            // Include a late control point so linear/eased cuts cross a segment.
            m.keyframes
                .retain(|k| !matches!(k.param, cut_core::KfParam::PosX | cut_core::KfParam::PosY));
            m.keyframes.push(serde_json::from_value(json!({"param":"pos_x","interp":interp,"points":[{"t_ms":0,"value":0.75},{"t_ms":5000,"value":0.04},{"t_ms":18000,"value":0.5}]})).unwrap());
            m.keyframes.push(serde_json::from_value(json!({"param":"pos_y","interp":"hold","points":[{"t_ms":0,"value":0.6},{"t_ms":5000,"value":0.04}]})).unwrap());
            for at in [12500, 15000] {
                let expected = edit_camera_rgb(&original, &store.dir, at);
                let mut split = original.clone();
                cut_core::edit::split(&mut split, &track, 10000).unwrap();
                let actual = edit_camera_rgb(&split, &store.dir, at);
                let a = edit_camera_bounds(&actual);
                let b = edit_camera_bounds(&expected);
                for i in 0..4 {
                    assert!(
                        a[i].abs_diff(b[i]) <= 2,
                        "ordinary={ordinary}, {interp}: split {a:?}, original {b:?}"
                    );
                }
                let error: f64 = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| a.abs_diff(*b) as f64)
                    .sum::<f64>()
                    / actual.len() as f64;
                assert!(
                    error < 1.0,
                    "ordinary={ordinary}, {interp}, mean pixel error {error}"
                );
                let mut trimmed = original.clone();
                cut_core::edit::trim(&mut trimmed, clip, Some(10000), None).unwrap();
                let actual = edit_camera_rgb(&trimmed, &store.dir, at - 10000);
                let a = edit_camera_bounds(&actual);
                for i in 0..4 {
                    assert!(
                        a[i].abs_diff(b[i]) <= 2,
                        "trim ordinary={ordinary}, {interp}: {a:?} vs {b:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn camera_split_and_trim_preserve_hold_linear_eased_pixels() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(camera_split_picture_case())
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn camera_hand_placement_replaces_movement_and_undo_restores_it() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(camera_manual_placement_case())
        })
        .unwrap()
        .join()
        .unwrap();
}
