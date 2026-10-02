use super::*;

struct PlacementGateReset;

impl Drop for PlacementGateReset {
    fn drop(&mut self) {
        *generated_placement_gate().lock().unwrap() = None;
    }
}

#[tokio::test]
async fn review_never_offers_step_back_for_an_off_cursor_caption_preset_save() {
    let root = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(root.path(), "captions", None).unwrap();
    let project_dir = store.dir.clone();
    let baseline = store.log.current_revision().unwrap().unwrap();
    let state = AppState::new();
    *state.project.write().await = Some(store);

    let saved = dispatch(
        &state,
        "captions.save_style",
        json!({"name": "my look", "style": {"font": "Arial", "size": 30, "color": "#fff"}}),
        Actor {
            kind: cut_core::ActorKind::Agent,
            name: "turn-preset".into(),
            via: "agent.chat".into(),
            request: None,
        },
    )
    .await;
    assert!(saved.ok, "preset fixture failed: {:?}", saved.error);

    let (actions, review) = agent_chat_turn_review(
        &state,
        &project_dir,
        1,
        &baseline,
        "turn-preset",
        "chat-preset",
        None,
    )
    .await
    .unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["verb"], "captions.save_style");
    assert!(review["diff_error"].is_null());
    assert_eq!(review["revert_safe"], false);
}

#[tokio::test]
async fn review_rejects_a_turn_after_the_open_project_changes() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a.cutproj");
    let b = root.path().join("b.cutproj");
    let state = AppState::new();
    for (name, path) in [("a", &a), ("b", &b)] {
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": path}),
            Actor::system(),
        )
        .await;
        assert!(created.ok, "project fixture failed: {:?}", created.error);
    }
    let error = agent_chat_turn_review(
        &state,
        &a,
        0,
        "baseline-from-a",
        "turn-from-a",
        "chat-from-a",
        None,
    )
    .await
    .expect_err("a turn from A must not review B's operation log");
    assert_eq!(error.code, error_codes::CONFLICT);
}

#[tokio::test]
async fn generated_replacement_commits_in_a_before_b_can_open() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let a = root_path.join("a.cutproj");
    let b = root_path.join("b.cutproj");
    let source = root_path.join("source.mp4");
    std::fs::write(&source, b"stub media").unwrap();
    let state = AppState::new();
    for (name, path) in [("a", &a), ("b", &b)] {
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": path}),
            Actor::system(),
        )
        .await;
        assert!(created.ok, "project fixture failed: {:?}", created.error);
    }
    let a = a.canonicalize().unwrap();
    let b = b.canonicalize().unwrap();
    let opened = dispatch(&state, "project.open", json!({"path": a}), Actor::system()).await;
    assert!(opened.ok, "A reopen failed: {:?}", opened.error);
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": source, "proxy": false}),
        Actor::system(),
    )
    .await;
    assert!(imported.ok, "fixture import failed: {:?}", imported.error);
    let asset = imported.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset": asset, "track": "v1", "at_ms": 0, "src_range_ms": [0, 1000]}),
        Actor::system(),
    )
    .await;
    assert!(inserted.ok, "fixture insert failed: {:?}", inserted.error);
    let clip = inserted.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let gate = GeneratedPlacementGate {
        clip: clip.clone(),
        before_edit: std::sync::Arc::new(tokio::sync::Notify::new()),
        resume: std::sync::Arc::new(tokio::sync::Notify::new()),
    };
    *generated_placement_gate().lock().unwrap() = Some(gate.clone());
    let _reset = PlacementGateReset;
    let before_edit = gate.before_edit.notified();
    let placement_state = state.clone();
    let a_dir = a.clone();
    let placement = tokio::spawn(async move {
        apply_generated_placement(
            &placement_state,
            VerbResult::ok(json!({"asset_id": asset})),
            Some(&PreparedGenerationPlacement::Replace {
                target_clip: clip,
                track: "v1".into(),
                duration_ms: 1000,
            }),
            "image",
            &a_dir,
            Actor::system(),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), before_edit)
        .await
        .expect("placement did not reach held edit");
    assert!(state.project_transition.try_lock().is_err());
    let open_state = state.clone();
    let open = tokio::spawn(async move {
        dispatch(
            &open_state,
            "project.open",
            json!({"path": b}),
            Actor::system(),
        )
        .await
    });
    tokio::task::yield_now().await;
    assert!(
        !open.is_finished(),
        "B must wait until A replacement commits"
    );
    gate.resume.notify_one();
    let placed = tokio::time::timeout(std::time::Duration::from_secs(2), placement)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(placed.result.unwrap()["placement"]["state"], "applied");
    let opened_b = tokio::time::timeout(std::time::Duration::from_secs(2), open)
        .await
        .unwrap()
        .unwrap();
    assert!(opened_b.ok, "B open failed: {:?}", opened_b.error);
    let project = state.project.read().await;
    assert_eq!(project.as_ref().unwrap().project.name, "b");
    assert!(project.as_ref().unwrap().project.assets.is_empty());
}
