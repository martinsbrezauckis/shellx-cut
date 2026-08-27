use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use serde_json::json;

#[tokio::test]
async fn stop_returns_versioned_cadence_and_never_fabricates_it_for_legacy_projects() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("cadence_stop.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"cadence_stop","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    for (capture_id, cadence) in [
        (
            "cap-cadence-v1",
            Some(json!({
                "schema": "shellx-record/capture-cadence/1",
                "requested": {"num": 2997, "den": 100},
                "backend_requested": {"num": 30, "den": 1},
                "probed_media": {
                    "avg_frame_rate": {"num": 30000, "den": 1001},
                    "r_frame_rate": {"num": 30, "den": 1},
                    "decoded_video_frames": 300,
                    "duration_ms": 10010
                }
            })),
        ),
        ("cap-cadence-legacy", None),
    ] {
        let capture = crate::screen_record::screen_record_cache_dir(&project_dir)
            .unwrap()
            .join(capture_id);
        std::fs::create_dir_all(&capture).unwrap();
        let source = capture.join("source.mp4");
        std::fs::write(&source, b"fixture source").unwrap();
        let mut recording_project = json!({
            "source_video": source.display().to_string(),
            "events": {"duration_ms": 1, "cursor": [], "clicks": []},
        });
        if let Some(cadence) = cadence {
            recording_project["capture_cadence"] = cadence;
        }
        std::fs::write(
            capture.join("project.json"),
            serde_json::to_vec(&recording_project).unwrap(),
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
        if capture_id == "cap-cadence-v1" {
            assert_eq!(
                result["cadence"]["requested"],
                json!({"num": 2997, "den": 100})
            );
            assert_eq!(
                result["cadence"]["probed_media"]["avg_frame_rate"],
                json!({"num": 30000, "den": 1001})
            );
            assert_ne!(
                result["cadence"]["probed_media"]["avg_frame_rate"],
                result["cadence"]["probed_media"]["r_frame_rate"],
                "average and nominal cadence must remain independently measured"
            );
        } else {
            assert!(
                result["cadence"].is_null(),
                "legacy project must not gain cadence facts"
            );
        }
    }
}
