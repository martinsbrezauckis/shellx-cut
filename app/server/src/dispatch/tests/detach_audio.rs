use super::*;

fn audio_asset(path: &str, hash: &str) -> cut_core::Asset {
    cut_core::Asset {
        path: path.into(),
        hash: hash.into(),
        probe: Some(json!({"duration_ms": 10_000, "has_audio": true})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

fn audio_clip_intervals(
    project: &cut_core::Project,
    clip_ids: &[&str],
) -> std::collections::BTreeMap<String, (String, u64, u64)> {
    cut_core::edl_from_project(project)
        .segments
        .into_iter()
        .filter(|segment| segment.track_kind == cut_core::TrackKind::Audio)
        .filter_map(|segment| {
            let clip_id = segment.clip_id?;
            clip_ids.contains(&clip_id.as_str()).then_some((
                clip_id,
                (
                    segment.track,
                    segment.timeline_in_ms,
                    segment.timeline_out_ms,
                ),
            ))
        })
        .collect()
}

#[tokio::test]
async fn detach_audio_never_moves_preexisting_audio_clips() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "detach", "dir": root.path().join("detach.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .record_import(
                Some("a1".into()),
                audio_asset("/fixture/video.mp4", "sha256:video"),
                test_actor(),
                None,
            )
            .unwrap();
        store
            .record_import(
                Some("a2".into()),
                audio_asset("/fixture/existing.wav", "sha256:existing"),
                test_actor(),
                None,
            )
            .unwrap();
    }

    for (asset, track, at_ms, src_range_ms) in [
        ("a1", "v1", 0, [0, 1000]),
        ("a2", "a1t", 0, [0, 2000]),
        ("a2", "a1t", 2000, [2000, 3000]),
    ] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({
                "asset": asset,
                "track": track,
                "at_ms": at_ms,
                "src_range_ms": src_range_ms,
                "ripple": false
            }),
            test_actor(),
        )
        .await;
        assert!(inserted.ok, "insert failed: {:?}", inserted.error);
    }

    let before = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        audio_clip_intervals(&store.project, &["c2", "c3"])
    };
    assert_eq!(
        before,
        std::collections::BTreeMap::from([
            ("c2".into(), ("a1t".into(), 0, 2000)),
            ("c3".into(), ("a1t".into(), 2000, 3000)),
        ])
    );

    let detached = dispatch(
        &state,
        "edit.detach_audio",
        json!({"clip": "c1"}),
        test_actor(),
    )
    .await;
    assert!(detached.ok, "detach failed: {:?}", detached.error);
    let result = detached.result.unwrap();
    assert_eq!(result["detached"], true);
    assert_ne!(
        result["audio_track"], "a1t",
        "an occupied audio track must not be used as an insert target"
    );

    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let after = audio_clip_intervals(&store.project, &["c2", "c3"]);
    assert_eq!(
        after, before,
        "detach must not move, split, or retime any pre-existing audio clip"
    );
    let detached_id = result["audio_clip"].as_str().unwrap();
    let detached_interval = audio_clip_intervals(&store.project, &[detached_id]);
    assert_eq!(
        detached_interval[detached_id],
        (result["audio_track"].as_str().unwrap().into(), 0, 1000)
    );
}
