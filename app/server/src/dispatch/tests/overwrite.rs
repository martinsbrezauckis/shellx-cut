use super::*;

fn media_asset(path: &str, hash: &str) -> cut_core::Asset {
    cut_core::Asset {
        path: path.into(),
        hash: hash.into(),
        probe: Some(json!({"duration_ms": 6_000, "kind": "video", "has_audio": true})),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

async fn overwrite_state() -> AppState {
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.keep().join("overwrite.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"overwrite", "dir":project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "project create failed: {:?}", created.error);
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .record_import(
                Some("a1".into()),
                media_asset("/fixture/old.mp4", "sha256:old"),
                test_actor(),
                None,
            )
            .unwrap();
        store
            .record_import(
                Some("a2".into()),
                media_asset("/fixture/new.mp4", "sha256:new"),
                test_actor(),
                None,
            )
            .unwrap();
    }
    for track in ["v1", "a1t"] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1", "track":track, "at_ms":0, "src_range_ms":[0,4_000], "ripple":false}),
            test_actor(),
        )
        .await;
        assert!(
            inserted.ok,
            "initial {track} insert failed: {:?}",
            inserted.error
        );
    }
    state
}

#[tokio::test]
async fn source_marks_commit_one_atomic_linked_overwrite_with_stable_receipt() {
    let state = overwrite_state().await;
    let overwrite = dispatch(
        &state,
        "edit.overwrite",
        json!({
            "asset":"a2",
            "at_ms":1_000,
            "video_track":"v1",
            "audio_track":"a1t",
            "source_in_ms":500,
            "source_out_ms":2_000,
        }),
        test_actor(),
    )
    .await;
    assert!(overwrite.ok, "overwrite failed: {:?}", overwrite.error);
    assert_eq!(overwrite.op_ids.as_ref().map(Vec::len), Some(1));
    let receipt = overwrite.result.expect("overwrite receipt");
    assert_eq!(receipt["src_range_ms"], json!([500, 2_000]));
    assert_eq!(receipt["duration_ms"], 1_500);
    assert_eq!(receipt["linked_av"], true);
    assert_eq!(receipt["tracks"].as_array().map(Vec::len), Some(2));
    assert!(receipt["tracks"].as_array().unwrap().iter().all(|track| {
        track["added_ms"] == json!([1_000, 2_500]) && track["tail_extended_ms"] == 0
    }));

    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.project.track("v1").unwrap().duration_ms(), 4_000);
    assert_eq!(store.project.track("a1t").unwrap().duration_ms(), 4_000);
    let op = store.log.read_all().unwrap().pop().expect("overwrite op");
    assert_eq!(op.verb, "edit.overwrite");
    assert_eq!(op.effects.len(), 2, "one op holds both selected targets");
    assert_eq!(op.args["src_range_ms"], json!([500, 2_000]));
    assert!(op.args.get("source_in_ms").is_none());
    assert!(op.args.get("source_out_ms").is_none());
}

/// Source Monitor turns the rendered 4500ms playhead into editorial 5500ms
/// before making this one linked request. Both tracks carry the same upstream
/// crossfade, so the atomic V+A operation resolves to the same editorial
/// coordinate and the EDL proves the new source starts at the visible point.
#[tokio::test]
async fn linked_overwrite_after_matching_crossfades_lands_at_visible_playhead() {
    let state = overwrite_state().await;
    for track in ["v1", "a1t"] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1", "track":track, "at_ms":4_000, "src_range_ms":[0,2_000], "ripple":false}),
            test_actor(),
        )
        .await;
        assert!(
            inserted.ok,
            "second {track} insert failed: {:?}",
            inserted.error
        );
        let faded = dispatch(
            &state,
            "edit.crossfade",
            json!({"track":track, "at_ms":4_000, "duration_ms":1_000}),
            test_actor(),
        )
        .await;
        assert!(faded.ok, "{track} crossfade failed: {:?}", faded.error);
    }
    let rendered_extent_before = {
        let guard = state.project.read().await;
        cut_core::edl::edl_from_project(&guard.as_ref().unwrap().project).duration_ms
    };

    let overwrite = dispatch(
        &state,
        "edit.overwrite",
        json!({
            "asset":"a2",
            "at_ms":5_500,
            "video_track":"v1",
            "audio_track":"a1t",
            "src_range_ms":[0,500],
        }),
        test_actor(),
    )
    .await;
    assert!(overwrite.ok, "overwrite failed: {:?}", overwrite.error);
    let receipt = overwrite.result.expect("overwrite receipt");
    assert_eq!(
        receipt["at_ms"], 5_500,
        "the durable op keeps editorial time"
    );
    assert!(receipt["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|track| { track["added_ms"] == json!([5_500, 6_000]) }));

    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let edl = cut_core::edl::edl_from_project(project);
    for track in ["v1", "a1t"] {
        let added = edl
            .track_segments(track)
            .find(|segment| segment.asset.as_deref() == Some("a2"))
            .expect("the overwrite source appears in the rendered EDL");
        assert_eq!(
            (added.timeline_in_ms, added.timeline_out_ms),
            (4_500, 5_000),
            "{track}: editorial 5500ms is the shared visible 4500ms playhead after its crossfade"
        );
    }
    assert_eq!(
        edl.duration_ms, rendered_extent_before,
        "overwrite did not ripple rendered extent"
    );
}

#[tokio::test]
async fn overwrite_rejects_ambiguous_source_forms_and_missing_destinations() {
    let state = overwrite_state().await;
    for args in [
        json!({"asset":"a2", "at_ms":0, "src_range_ms":[0,1_000], "source_in_ms":0, "source_out_ms":1_000, "video_track":"v1"}),
        json!({"asset":"a2", "at_ms":0, "src_range_ms":[0,1_000]}),
    ] {
        let result = dispatch(&state, "edit.overwrite", args, test_actor()).await;
        assert!(!result.ok, "invalid overwrite must not commit");
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some(error_codes::INVALID_ARGS)
        );
    }
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(
        store.log.read_all().unwrap().len(),
        5,
        "validation produced no new operation"
    );
}
