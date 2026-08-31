use super::*;

async fn seeded_preview(state: &AppState, root: &std::path::Path) -> serde_json::Value {
    let created = request_idempotency::create_project(state, root).await;
    assert!(created.ok, "{:?}", created.error);
    for (text, range_ms) in [
        ("teh first caption", [0, 900]),
        ("teh second caption", [1000, 1900]),
    ] {
        let added = dispatch(
            state,
            "captions.add_text",
            json!({"text": text, "range_ms": range_ms, "position": "bottom"}),
            test_actor(),
        )
        .await;
        assert!(added.ok, "{:?}", added.error);
    }
    let preview = dispatch(
        state,
        "captions.bulk_preview",
        json!({
            "track": "txt1",
            "find": "teh",
            "replace_with": "the",
            "match_mode": "whole_word",
            "case_sensitive": false,
        }),
        test_actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    preview.result.unwrap()
}

async fn seeded_many_preview(
    state: &AppState,
    root: &std::path::Path,
    count: usize,
) -> serde_json::Value {
    assert!(count > 0);
    let created = request_idempotency::create_project(state, root).await;
    assert!(created.ok, "{:?}", created.error);
    let added = dispatch(
        state,
        "captions.add_text",
        json!({"text": "teh cue 0", "range_ms": [0, 900], "position": "bottom"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().expect("created project");
        let track = store.project.track_mut("txt1").expect("caption track");
        for index in 1..count {
            track
                .clips
                .push(cut_core::Clip::Caption(cut_core::CaptionClip {
                    id: format!("bulk-{index}"),
                    text: format!("teh cue {index}"),
                    style_ref: None,
                    range_ms: [index as u64 * 1000, index as u64 * 1000 + 900],
                }));
        }
    }
    let preview = dispatch(
        state,
        "captions.bulk_preview",
        json!({
            "track": "txt1",
            "find": "teh",
            "replace_with": "the",
            "match_mode": "whole_word",
            "case_sensitive": false,
        }),
        test_actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    preview.result.unwrap()
}

#[tokio::test]
async fn caption_bulk_preview_apply_is_one_undoable_request_keyed_operation() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let preview = seeded_preview(&state, root.path()).await;
    assert_eq!(preview["match_count"], 2);
    assert_eq!(preview["affected_cue_count"], 2);
    assert_eq!(preview["rows"].as_array().unwrap().len(), 2);
    assert_eq!(preview["timing_refresh"]["available"], false);

    let apply_args = json!({
        "preview_hash": preview["preview_hash"],
        "request_id": "caption-bulk-apply-0001",
        "expected_revision": preview["project_revision"],
    });
    let applied = dispatch(
        &state,
        "captions.bulk_apply",
        apply_args.clone(),
        test_actor(),
    )
    .await;
    assert!(applied.ok, "{:?}", applied.error);
    assert_eq!(applied.op_ids.as_ref().unwrap().len(), 1);
    assert_eq!(applied.result.as_ref().unwrap()["timing_preserved"], true);

    // The same durable request is replayed, not appended a second time.
    let retry = dispatch(&state, "captions.bulk_apply", apply_args, test_actor()).await;
    assert_eq!(retry, applied, "a retry must replay the original response");

    let after = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(after.result.unwrap()).unwrap();
    let captions = project
        .track("txt1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|clip| match clip {
            cut_core::Clip::Caption(cue) => Some((cue.text.as_str(), cue.range_ms)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        captions,
        vec![
            ("the first caption", [0, 900]),
            ("the second caption", [1000, 1900])
        ]
    );

    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "{:?}", undo.error);
    let restored = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(restored.result.unwrap()).unwrap();
    let captions = project
        .track("txt1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|clip| match clip {
            cut_core::Clip::Caption(cue) => Some(cue.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(captions, vec!["teh first caption", "teh second caption"]);
}

#[tokio::test]
async fn caption_bulk_refuses_stale_and_previous_server_preview() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let preview = seeded_preview(&state, root.path()).await;

    // Reopening the same durable project in a fresh server deliberately drops
    // volatile reviewed-preview state. Its revision still matches, so this
    // specifically proves restart refusal rather than ordinary stale revision.
    let project_dir = root.path().join("request-test.cutproj");
    let restarted = AppState::new();
    let opened = dispatch(
        &restarted,
        "project.open",
        json!({"path": project_dir}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);
    let after_restart = dispatch(
        &restarted,
        "captions.bulk_apply",
        json!({
            "preview_hash": preview["preview_hash"],
            "request_id": "caption-bulk-restart-0001",
            "expected_revision": preview["project_revision"],
        }),
        test_actor(),
    )
    .await;
    assert!(!after_restart.ok);
    assert_eq!(after_restart.error.unwrap().code, error_codes::CONFLICT);

    let edit = dispatch(
        &state,
        "captions.set_text",
        json!({"clip": preview["rows"][0]["cue_id"], "text": "changed elsewhere"}),
        test_actor(),
    )
    .await;
    assert!(edit.ok, "{:?}", edit.error);
    let stale = dispatch(
        &state,
        "captions.bulk_apply",
        json!({
            "preview_hash": preview["preview_hash"],
            "request_id": "caption-bulk-stale-0001",
            "expected_revision": preview["project_revision"],
        }),
        test_actor(),
    )
    .await;
    assert!(!stale.ok);
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
}

#[tokio::test]
async fn caption_bulk_applies_private_targets_beyond_the_bounded_preview() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let preview = seeded_many_preview(&state, root.path(), 101).await;
    assert_eq!(preview["match_count"], 101);
    assert_eq!(preview["affected_cue_count"], 101);
    assert_eq!(preview["rows"].as_array().unwrap().len(), 100);
    assert_eq!(preview["omitted_rows"], 1);
    assert_eq!(preview["can_apply"], true);

    let applied = dispatch(
        &state,
        "captions.bulk_apply",
        json!({
            "preview_hash": preview["preview_hash"],
            "request_id": "caption-bulk-many-0001",
            "expected_revision": preview["project_revision"],
        }),
        test_actor(),
    )
    .await;
    assert!(applied.ok, "{:?}", applied.error);
    assert_eq!(applied.op_ids.as_ref().unwrap().len(), 1);
    let after = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(after.result.unwrap()).unwrap();
    let texts = project
        .track("txt1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|clip| match clip {
            cut_core::Clip::Caption(cue) => Some(cue.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(texts.len(), 101);
    assert!(texts.iter().all(|text| text.starts_with("the cue")));

    assert!(
        dispatch(&state, "project.undo", json!({}), test_actor())
            .await
            .ok
    );
    let restored = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(restored.result.unwrap()).unwrap();
    assert!(project
        .track("txt1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|clip| match clip {
            cut_core::Clip::Caption(cue) => Some(cue.text.as_str()),
            _ => None,
        })
        .all(|text| text.starts_with("teh cue")));
}

#[tokio::test]
async fn caption_bulk_fails_closed_when_the_private_target_cap_is_exceeded() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = request_idempotency::create_project(&state, root.path()).await;
    assert!(created.ok, "{:?}", created.error);
    let added = dispatch(
        &state,
        "captions.add_text",
        json!({"text": "teh cue 0", "range_ms": [0, 900], "position": "bottom"}),
        test_actor(),
    )
    .await;
    assert!(added.ok, "{:?}", added.error);
    {
        let mut project = state.project.write().await;
        let store = project.as_mut().expect("created project");
        let track = store.project.track_mut("txt1").expect("caption track");
        // Keep this one past the documented server-private 5,000-cue cap.
        for index in 1..=5_000 {
            track
                .clips
                .push(cut_core::Clip::Caption(cut_core::CaptionClip {
                    id: format!("over-cap-{index}"),
                    text: "teh".into(),
                    style_ref: None,
                    range_ms: [index as u64 * 1000, index as u64 * 1000 + 900],
                }));
        }
    }
    let refused = dispatch(
        &state,
        "captions.bulk_preview",
        json!({"track": "txt1", "find": "teh", "replace_with": "the", "match_mode": "contains"}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    assert_eq!(refused.error.unwrap().code, error_codes::GUARDRAIL);
}
