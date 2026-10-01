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
use std::time::{Duration, Instant};

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

#[test]
fn render_final_path_must_match_the_selected_container() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
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
    });
}

/// An explicit path outside the authorized output roots must be refused before
/// the automatic hardware-encoder capability probe. That probe runs ffmpeg at
/// the selected output dimensions, so running it for a path the public fence
/// will reject needlessly blocks the server's request loop.
#[test]
fn render_final_rejects_outside_path_before_hardware_probe() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new();
        create_project(&state, &dir).await;
        let outside = dir.path().parent().unwrap().join("outside.mp4");

        let started = Instant::now();
        let result = dispatch(
            &state,
            "render.final",
            json!({"path": outside}),
            test_actor(),
        )
        .await;

        assert!(!result.ok, "outside output path must be refused");
        assert_eq!(result.error.unwrap().code, error_codes::INVALID_ARGS);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "outside path must fail before the hardware capability probe; elapsed: {:?}",
            started.elapsed()
        );
        assert!(
            !dir.path()
                .join("p.cutproj/receipts/.render_001.reserved")
                .exists(),
            "a rejected output path must not reserve a render receipt id"
        );
    });
}

#[test]
fn export_paths_must_match_their_serialized_type() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
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
    });
}
