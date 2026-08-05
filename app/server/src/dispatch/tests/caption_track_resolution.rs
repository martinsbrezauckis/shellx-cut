use super::*;

async fn op_for(state: &AppState, op_id: &str) -> cut_core::OpRecord {
    let ops = dispatch(state, "project.ops", json!({}), test_actor()).await;
    assert!(ops.ok, "project.ops failed: {:?}", ops.error);
    serde_json::from_value::<Vec<cut_core::OpRecord>>(ops.result.unwrap()["ops"].clone())
        .unwrap()
        .into_iter()
        .find(|op| op.op_id == op_id)
        .unwrap_or_else(|| panic!("missing operation {op_id}"))
}

#[tokio::test]
async fn shift_and_reflow_operate_on_the_timed_text_track_when_cap1_is_absent() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "timed-text", "dir": root.path().join("timed-text.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    let added = dispatch(
        &state,
        "captions.add_text",
        json!({
            "text": "A deliberately long timed text card for reflow",
            "range_ms": [1000, 5000],
            "position": "center"
        }),
        test_actor(),
    )
    .await;
    assert!(added.ok, "add_text failed: {:?}", added.error);
    assert_eq!(added.result.unwrap()["track_id"], "txt1");

    let shifted = dispatch(
        &state,
        "captions.shift",
        json!({"offset_ms": 500}),
        test_actor(),
    )
    .await;
    assert!(shifted.ok, "shift failed: {:?}", shifted.error);
    let shift_result = shifted.result.unwrap();
    assert_eq!(shift_result["track_id"], "txt1");
    assert_eq!(shift_result["shifted"], 1);
    let shift_op = op_for(&state, &shifted.op_ids.unwrap()[0]).await;
    assert_eq!(shift_op.effects[0].track.as_deref(), Some("txt1"));

    {
        let guard = state.project.read().await;
        let project = &guard.as_ref().unwrap().project;
        let cut_core::Clip::Caption(cue) = &project.track("txt1").unwrap().clips[0] else {
            panic!("txt1 must contain a caption clip");
        };
        assert_eq!(cue.range_ms, [1500, 5500]);
    }

    let reflowed = dispatch(
        &state,
        "captions.reflow",
        json!({"max_chars": 12}),
        test_actor(),
    )
    .await;
    assert!(reflowed.ok, "reflow failed: {:?}", reflowed.error);
    let reflow_result = reflowed.result.unwrap();
    assert_eq!(reflow_result["track_id"], "txt1");
    assert!(
        reflow_result["cues_after"].as_u64().unwrap() > 1,
        "the long timed-text card should be split"
    );
    let reflow_op = op_for(&state, &reflowed.op_ids.unwrap()[0]).await;
    assert_eq!(reflow_op.effects[0].track.as_deref(), Some("txt1"));

    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    assert!(project.track("cap1").is_none());
    assert!(project.track("txt1").unwrap().clips.len() > 1);
}

#[tokio::test]
async fn canonical_caption_track_is_preferred_over_timed_text() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "canonical", "dir": root.path().join("canonical.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    let added = dispatch(
        &state,
        "captions.add_text",
        json!({"text": "Title", "range_ms": [100, 900], "position": "center"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "add_text failed: {:?}", added.error);

    let subtitles = root.path().join("captions.srt");
    std::fs::write(
        &subtitles,
        "1\n00:00:01,000 --> 00:00:02,000\nCanonical caption\n",
    )
    .unwrap();
    let imported = dispatch(
        &state,
        "captions.import",
        json!({"path": subtitles}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "import failed: {:?}", imported.error);
    assert_eq!(imported.result.unwrap()["track_id"], "cap1");

    let shifted = dispatch(
        &state,
        "captions.shift",
        json!({"offset_ms": 250}),
        test_actor(),
    )
    .await;
    assert!(shifted.ok, "shift failed: {:?}", shifted.error);
    assert_eq!(shifted.result.unwrap()["track_id"], "cap1");

    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let cut_core::Clip::Caption(title) = &project.track("txt1").unwrap().clips[0] else {
        panic!("txt1 must contain a caption clip");
    };
    assert_eq!(title.range_ms, [100, 900], "txt1 must remain untouched");
    let cut_core::Clip::Caption(caption) = &project.track("cap1").unwrap().clips[0] else {
        panic!("cap1 must contain a caption clip");
    };
    assert_eq!(caption.range_ms, [1250, 2250]);
}
