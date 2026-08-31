use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use serde_json::json;

#[tokio::test]
async fn stop_returns_quality_only_when_final_source_facts_match() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("quality_stop.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"quality_stop","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    for (capture_id, facts, quality_height, encoder) in [
        (
            "cap-quality-verified",
            json!({
                "schema": "shellx-record/capture-source-facts/1",
                "width": 1280, "height": 720, "codec_name": "h264"
            }),
            720,
            "libx264",
        ),
        (
            "cap-quality-codec-forged",
            json!({
                "schema": "shellx-record/capture-source-facts/1",
                "width": 1280, "height": 720, "codec_name": "hevc"
            }),
            720,
            "libx264",
        ),
        (
            "cap-quality-size-forged",
            json!({
                "schema": "shellx-record/capture-source-facts/1",
                "width": 1280, "height": 1080, "codec_name": "h264"
            }),
            1080,
            "libx264",
        ),
        (
            "cap-quality-encoder-forged",
            json!({
                "schema": "shellx-record/capture-source-facts/1",
                "width": 1280, "height": 720, "codec_name": "h264"
            }),
            720,
            "anything",
        ),
    ] {
        let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
            .unwrap()
            .join(capture_id);
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        std::fs::write(&source, b"fixture source").unwrap();
        std::fs::write(
            capture.join("project.json"),
            serde_json::to_vec(&json!({
                "source_video": source.display().to_string(),
                "events": {"duration_ms": 1, "cursor": [], "clicks": []},
                "capture_source_facts": facts,
                "capture_quality": {
                    "schema": "shellx-record/capture-quality/1",
                    "requested": {"output_size": "720p", "profile": "high"},
                    "width": 1280, "height": quality_height, "encoder": encoder
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let stopped = dispatch(
            &state,
            "screen_record.stop",
            json!({"capture_id": capture_id, "autoedit": false}),
            test_actor(),
        )
        .await;
        assert!(stopped.ok, "{:?}", stopped.error);
        let result = stopped.result.unwrap();
        if capture_id == "cap-quality-verified" {
            assert_eq!(result["quality"]["width"], 1280);
            assert_eq!(result["quality"]["height"], 720);
            assert_eq!(result["quality"]["encoder"], "libx264");
        } else {
            assert!(result["quality"].is_null(), "forged facts must stay hidden");
        }
    }
}
