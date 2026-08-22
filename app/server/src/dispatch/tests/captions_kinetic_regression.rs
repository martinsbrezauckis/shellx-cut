use super::*;

#[tokio::test]
async fn captions_kinetic_per_word_uses_timeline_transcript_without_caption_track() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "one", "start_ms": 5100, "end_ms": 5300},
                {"idx": 1, "word": "word", "start_ms": 5350, "end_ms": 5550},
                {"idx": 2, "word": "cue", "start_ms": 5600, "end_ms": 5800}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |asset| {
        asset.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();

    let before = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(before.result.unwrap()).unwrap();
    assert!(
        !project
            .tracks
            .iter()
            .any(|track| track.kind == cut_core::TrackKind::Caption),
        "per-word kinetic starts without a caption track"
    );

    let result = dispatch(
        &state,
        "captions.kinetic",
        json!({"per_word": true, "rationale": "animate transcript words"}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
    assert_eq!(result.result.as_ref().unwrap()["cue_count"], 3);

    let after = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(after.result.unwrap()).unwrap();
    assert!(
        project
            .tracks
            .iter()
            .any(|track| track.id.starts_with("title") && !track.clips.is_empty()),
        "one title overlay carries the three word-timed cues"
    );
}
