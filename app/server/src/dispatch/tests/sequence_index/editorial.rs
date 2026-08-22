use super::super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use cut_core::{edit::make_media_clip, edl_from_project, Clip};
use serde_json::json;

async fn import_stub_media(state: &AppState, path: &std::path::Path) -> String {
    let imported = dispatch(
        state,
        "media.import",
        json!({"path":path,"proxy":false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    imported.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn coalesces_each_speed_ramp_clip_into_one_laid_index_range() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_path = dir.path().join("sequence-index-ramp.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"sequence-index-ramp","dir":project_path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let target_path = dir.path().join("target.mp4");
    std::fs::write(&target_path, b"target").unwrap();
    let target = import_stub_media(&state, &target_path).await;
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset":target,"track":"v1","at_ms":0,"src_range_ms":[0,4000]}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);
    let clip_id = inserted.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let ramped = dispatch(
        &state,
        "edit.speed_ramp",
        json!({
            "clip":clip_id,
            "points":[
                {"at_ms":0,"factor":1.0},
                {"at_ms":2000,"factor":4.0},
                {"at_ms":4000,"factor":1.0}
            ],
            "segments":4
        }),
        test_actor(),
    )
    .await;
    assert!(ramped.ok, "{:?}", ramped.error);
    let laid_duration_ms = ramped.result.unwrap()["new_duration_ms"].as_u64().unwrap();

    let ramp_segments = {
        let guard = state.project.read().await;
        edl_from_project(&guard.as_ref().unwrap().project)
            .track_segments("v1")
            .filter(|segment| segment.clip_id.as_deref() == Some(clip_id.as_str()))
            .count()
    };
    assert!(
        ramp_segments >= 2,
        "the fixture must expand into multiple laid EDL segments"
    );

    let indexed = dispatch(
        &state,
        "project.sequence_index",
        json!({"asset":target,"kind":"clip"}),
        test_actor(),
    )
    .await;
    assert!(indexed.ok, "{:?}", indexed.error);
    let row = &indexed.result.unwrap()["results"][0];
    assert_eq!(row["at_ms"], 0);
    assert_eq!(row["end_ms"], laid_duration_ms);
}

#[tokio::test]
async fn indexes_cross_sequence_asset_uses_at_laid_crossfade_starts() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_path = dir.path().join("sequence-index-laid.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"sequence-index-laid","dir":project_path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let lead_path = dir.path().join("lead.mp4");
    let target_path = dir.path().join("target.mp4");
    std::fs::write(&lead_path, b"lead").unwrap();
    std::fs::write(&target_path, b"target").unwrap();
    let lead = import_stub_media(&state, &lead_path).await;
    let target = import_stub_media(&state, &target_path).await;

    for (asset, at_ms) in [(&lead, 0_u64), (&target, 1000_u64)] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({"asset":asset,"track":"v1","at_ms":at_ms,"src_range_ms":[0,1000]}),
            test_actor(),
        )
        .await;
        assert!(inserted.ok, "{:?}", inserted.error);
    }
    let crossfade = dispatch(
        &state,
        "edit.crossfade",
        json!({"track":"v1","at_ms":1000,"duration_ms":300}),
        test_actor(),
    )
    .await;
    assert!(crossfade.ok, "{:?}", crossfade.error);

    let alternate = dispatch(
        &state,
        "project.sequence_create",
        json!({"name":"Alternate use","from":"empty"}),
        test_actor(),
    )
    .await;
    assert!(alternate.ok, "{:?}", alternate.error);
    let alternate_id = alternate.result.unwrap()["active_sequence"]
        .as_str()
        .unwrap()
        .to_owned();
    let alternate_insert = dispatch(
        &state,
        "edit.insert",
        json!({"asset":target,"track":"v1","at_ms":400,"src_range_ms":[0,1000]}),
        test_actor(),
    )
    .await;
    assert!(alternate_insert.ok, "{:?}", alternate_insert.error);

    let indexed = dispatch(
        &state,
        "project.sequence_index",
        json!({"asset":target,"kind":"clip","limit":500}),
        test_actor(),
    )
    .await;
    assert!(indexed.ok, "{:?}", indexed.error);
    let indexed = indexed.result.unwrap();
    assert_eq!(indexed["asset"], target);
    assert_eq!(indexed["total"], 2);
    assert_eq!(indexed["truncated"], false);
    assert_eq!(indexed["results"][0]["sequence_id"], "seq1");
    assert_eq!(indexed["results"][0]["at_ms"], 700);
    assert_eq!(indexed["results"][0]["end_ms"], 1700);
    assert_eq!(indexed["results"][1]["sequence_id"], alternate_id);
    assert_eq!(indexed["results"][1]["at_ms"], 400);
}

#[tokio::test]
async fn exact_asset_filter_runs_before_the_500_row_limit() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_path = dir.path().join("sequence-index-asset-filter.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"sequence-index-asset-filter","dir":project_path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let decoy_path = dir.path().join("decoy.mp4");
    let target_path = dir.path().join("target.mp4");
    std::fs::write(&decoy_path, b"decoy").unwrap();
    std::fs::write(&target_path, b"target").unwrap();
    let decoy = import_stub_media(&state, &decoy_path).await;
    let target = import_stub_media(&state, &target_path).await;

    {
        let mut guard = state.project.write().await;
        let project = &mut guard.as_mut().unwrap().project;
        let track = project.track_mut("v1").unwrap();
        // These non-target rows all collide with the old text query because a
        // clip id is searchable. They deliberately precede the real use.
        for index in 0..501 {
            track.clips.push(Clip::Media(make_media_clip(
                &target,
                &decoy,
                index,
                index + 1,
            )));
        }
        track
            .clips
            .push(Clip::Media(make_media_clip("target-use", &target, 0, 1)));
    }

    let text_query = dispatch(
        &state,
        "project.sequence_index",
        json!({"query":target,"kind":"clip","limit":500}),
        test_actor(),
    )
    .await;
    assert!(text_query.ok, "{:?}", text_query.error);
    let text_query = text_query.result.unwrap();
    assert_eq!(text_query["total"], 502);
    assert_eq!(text_query["truncated"], true);
    assert!(text_query["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["asset"] == decoy));

    let exact = dispatch(
        &state,
        "project.sequence_index",
        json!({"asset":target,"kind":"clip","limit":500}),
        test_actor(),
    )
    .await;
    assert!(exact.ok, "{:?}", exact.error);
    let exact = exact.result.unwrap();
    assert_eq!(exact["total"], 1);
    assert_eq!(exact["truncated"], false);
    assert_eq!(exact["results"][0]["id"], "target-use");
    assert_eq!(exact["results"][0]["asset"], target);
}
