//! Dispatch-level output path contract tests.
//!
//! These stop at the filename guard, before a renderer or serializer can do
//! work. The lower-level output_paths tests cover the canonical fence and the
//! concurrent default-name reservation itself.

use super::super::dispatch;
use super::test_actor;
use crate::state::AppState;
use cut_core::error_codes;
use serde_json::json;

async fn create_project(state: &AppState, dir: &tempfile::TempDir) {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": "p", "dir": dir.path().join("p.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
}

#[tokio::test]
async fn render_final_path_must_match_the_selected_container() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    create_project(&state, &dir).await;

    for args in [
        json!({"dry_run": true, "hardware": "off", "path": "exports/final.webm"}),
        json!({"dry_run": true, "hardware": "off", "format": "vp9", "path": "exports/final.mp4"}),
        json!({"dry_run": true, "hardware": "off", "format": "prores", "path": "exports/final.mp4"}),
    ] {
        let result = dispatch(&state, "render.final", args, test_actor()).await;
        assert!(!result.ok, "wrong container must be refused");
        assert_eq!(result.error.unwrap().code, error_codes::INVALID_ARGS);
    }

    let result = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true, "hardware": "off", "format": "vp9", "path": "exports/final.webm"}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
}

#[tokio::test]
async fn export_paths_must_match_their_serialized_type() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    create_project(&state, &dir).await;

    for (verb, args) in [
        (
            "export.audio",
            json!({"format": "wav", "path": "exports/mix.mp3"}),
        ),
        (
            "export.xml",
            json!({"format": "premiere", "path": "exports/timeline.fcpxml"}),
        ),
        ("export.srt", json!({"path": "exports/captions.vtt"})),
        (
            "export.transcript",
            json!({"format": "md", "path": "exports/transcript.txt"}),
        ),
    ] {
        let result = dispatch(&state, verb, args, test_actor()).await;
        assert!(!result.ok, "{verb} wrong suffix must be refused");
        assert_eq!(
            result.error.unwrap().code,
            error_codes::INVALID_ARGS,
            "{verb}"
        );
    }
}
