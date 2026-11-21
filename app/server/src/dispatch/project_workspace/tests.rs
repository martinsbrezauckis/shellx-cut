use super::*;

fn oversized_overlay_store() -> (tempfile::TempDir, ProjectStore) {
    let root = tempfile::tempdir().expect("temporary project root");
    let mut store =
        ProjectStore::create(root.path(), "oversized-overlay", None).expect("create project store");
    let (asset_id, _) = store
        .record_import(
            Some("a1".into()),
            cut_core::Asset {
                path: "/testdata/title.mov".into(),
                hash: "sha256:title".into(),
                probe: Some(json!({"duration_ms": 100})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            Actor::system(),
            None,
        )
        .expect("record title asset");
    store
        .apply_lowered(
            "title.add",
            json!({
                "text": "x".repeat(16 * 1024 + 1),
                "range_ms": [0, 100],
            }),
            Actor::system(),
            None,
            vec![
                InverseOp {
                    verb: "edit.add_track".into(),
                    args: json!({"kind": "video", "id": "title1"}),
                },
                InverseOp {
                    verb: "edit.insert".into(),
                    args: json!({
                        "asset": asset_id,
                        "track": "title1",
                        "at_ms": 0,
                        "src_range_ms": [0, 100],
                        "ripple": false,
                    }),
                },
            ],
            vec![],
        )
        .expect("record oversized title metadata");
    (root, store)
}

fn effect(detail: Value) -> OpEffect {
    OpEffect {
        track: Some("title1".into()),
        detail: detail.as_object().expect("effect object").clone(),
    }
}

fn op(verb: &str, args: Value, effects: Vec<OpEffect>) -> OpRecord {
    OpRecord {
        op_id: format!("op_{verb}"),
        ts: OpRecord::now_ts(),
        actor: Actor::system(),
        verb: verb.into(),
        args,
        rationale: None,
        effects,
        inverse: None,
        status: cut_core::OpStatus::Applied,
    }
}

#[test]
fn split_projects_title_and_shape_metadata_to_independent_right_halves() {
    let ops = vec![
        op(
            "title.add",
            json!({"text":"ORIGINAL TITLE","range_ms":[0,1000]}),
            vec![effect(json!({"added_clip":"title-left"}))],
        ),
        op(
            "edit.add_shape",
            json!({"shape":"rect","text":"ORIGINAL SHAPE","fill":"#FF0000","range_ms":[0,1000]}),
            vec![effect(json!({"added_clip":"shape-left"}))],
        ),
        op(
            "edit.split",
            json!({"track":"title1","at_ms":500}),
            vec![effect(json!({"left":"title-left","right":"title-right"}))],
        ),
        op(
            "edit.split",
            json!({"track":"title2","at_ms":500}),
            vec![effect(json!({"left":"shape-left","right":"shape-right"}))],
        ),
        op(
            "title.update",
            json!({"clip":"title-right","text":"RIGHT TITLE"}),
            vec![],
        ),
        op(
            "shape.update",
            json!({"clip":"shape-right","label":"RIGHT SHAPE","fill":"#00FF00"}),
            vec![],
        ),
    ];

    let titles = title_clip_texts(&ops);
    assert_eq!(
        titles.get("title-left").map(String::as_str),
        Some("ORIGINAL TITLE")
    );
    assert_eq!(
        titles.get("title-right").map(String::as_str),
        Some("RIGHT TITLE")
    );

    let shapes = shape_clip_props(&ops);
    assert_eq!(shapes["shape-left"]["text"], "ORIGINAL SHAPE");
    assert_eq!(shapes["shape-left"]["fill"], "#FF0000");
    assert_eq!(shapes["shape-right"]["text"], "RIGHT SHAPE");
    assert_eq!(shapes["shape-right"]["fill"], "#00FF00");
}

#[tokio::test]
async fn project_open_and_state_reject_oversized_overlay_metadata() {
    let (_root, store) = oversized_overlay_store();
    let path = store.dir.clone();

    let state = AppState::new();
    let open_error = project_open(&state, json!({"path": path}))
        .await
        .expect_err("project.open must reject oversized imported overlay metadata");
    assert_eq!(open_error.code, error_codes::INVALID_ARGS);
    assert!(
        open_error.message.contains("metadata exceeds"),
        "{open_error:?}"
    );
    assert!(
        state.project.read().await.is_none(),
        "failed open must not activate"
    );

    *state.project.write().await = Some(store);
    let state_error = project_state(&state, json!({}))
        .await
        .expect_err("project.state must reject oversized overlay metadata");
    assert_eq!(state_error.code, error_codes::INVALID_ARGS);
    assert!(
        state_error.message.contains("metadata exceeds"),
        "{state_error:?}"
    );
}

#[tokio::test]
async fn refused_project_activation_preserves_recent_entries() {
    for indexed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let previous = ProjectStore::create(root.path(), "previous", None).unwrap();
        let previous_dir = previous.dir.clone();
        activate_project_locked(&state, previous).await.unwrap();
        let target = ProjectStore::create(root.path(), "refused", None).unwrap();
        let path = target.dir.clone();
        if indexed {
            crate::projects_index::note_created("Earlier name", &path.to_string_lossy()).unwrap();
        }
        let entry = || {
            crate::projects_index::load_for_test()
                .entries
                .into_iter()
                // The index stores the plain form of a path; a created project's
                // directory is canonical, which on Windows carries the `\\?\` prefix.
                .find(|entry| {
                    entry.path == crate::projects_index::normalize_path(&path.to_string_lossy())
                })
        };
        let before = entry();
        std::fs::create_dir_all(path.join("jobs")).unwrap();
        std::fs::write(
            path.join("jobs/job_001.json"),
            serde_json::to_vec(&json!({
                "job_id": "job_001", "kind": "render", "state": "failed",
                "progress": 1.0, "created_ts": "old", "updated_ts": "old",
                "retry": {"eligible": false, "root_job_id": "job_999", "attempt": 1}
            }))
            .unwrap(),
        )
        .unwrap();
        ProjectStore::open(&path).expect("project loads before job-store activation");
        let error = project_open(&state, json!({"path": path}))
            .await
            .unwrap_err();
        assert!(
            error
                .message
                .contains("inconsistent persisted retry lineage"),
            "{error:?}"
        );
        assert_eq!(
            state.project.read().await.as_ref().unwrap().dir,
            previous_dir
        );
        assert_eq!(
            entry(),
            before,
            "refused open must neither register nor refresh recents"
        );
        std::fs::remove_file(path.join("jobs/job_001.json")).unwrap();
        project_open(&state, json!({"path": path})).await.unwrap();
        let opened = entry().expect("an accepted open registers the project");
        assert_eq!(opened.name, "refused");
        assert_eq!(opened.clip_count, Some(0));
        assert_eq!(opened.duration_ms, Some(0));
    }
}
