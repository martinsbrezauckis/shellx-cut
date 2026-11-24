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
    let open_error = project_open(&state, json!({"path": path}), Actor::system())
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
    let state_error = project_state(&state, json!({}), Actor::system())
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
        let error = project_open(&state, json!({"path": path}), Actor::system())
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
        project_open(&state, json!({"path": path}), Actor::system())
            .await
            .unwrap();
        let opened = entry().expect("an accepted open registers the project");
        assert_eq!(opened.name, "refused");
        assert_eq!(opened.clip_count, Some(0));
        assert_eq!(opened.duration_ms, Some(0));
    }
}

#[tokio::test]
async fn open_with_ordinary_file_at_screen_record_refuses_without_switching_project_or_event() {
    let cases: Vec<(&str, bool)> = if cfg!(unix) {
        vec![("file", false), ("symlink", true)]
    } else {
        vec![("file", false)]
    };
    for (label, as_symlink) in cases {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let previous = ProjectStore::create(root.path(), "previous", None).unwrap();
        let previous_dir = previous.dir.clone();
        activate_project_locked(&state, previous).await.unwrap();
        let mut events = state.events.subscribe();

        let crafted = ProjectStore::create(root.path(), "crafted", None).unwrap();
        let crafted_dir = crafted.dir.clone();
        drop(crafted);

        let before = crate::projects_index::load_for_test()
            .entries
            .into_iter()
            .find(|entry| {
                entry.path == crate::projects_index::normalize_path(&crafted_dir.to_string_lossy())
            });

        let cache_dir = crafted_dir.join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let screen_record_path = cache_dir.join("screen_record");
        if as_symlink {
            #[cfg(unix)]
            {
                let outside = root.path().join("outside-screen-record");
                std::fs::create_dir(&outside).unwrap();
                std::os::unix::fs::symlink(&outside, &screen_record_path).unwrap();
            }
        } else {
            std::fs::write(&screen_record_path, b"not-a-dir").unwrap();
        }

        let err = project_open(&state, json!({"path": crafted_dir}), Actor::system())
            .await
            .expect_err(&format!(
                "case {label}: project.open must refuse bad cache/screen_record before switching"
            ));
        assert_eq!(err.code, error_codes::INVALID_ARGS, "case {label}: {err:?}");

        let active = state.project.read().await;
        let active_store = active.as_ref().expect("previous project must remain open");
        assert_eq!(
            active_store.dir, previous_dir,
            "case {label}: active project changed despite failed project.open"
        );
        assert_eq!(active_store.project.name, "previous");

        while let Ok(ev) = events.try_recv() {
            assert!(
                !matches!(ev, Event::ProjectChanged { .. }),
                "case {label}: refused project.open must not emit ProjectChanged: {ev:?}"
            );
        }

        if !as_symlink {
            assert_eq!(std::fs::read(&screen_record_path).unwrap(), b"not-a-dir");
        }
        let after = crate::projects_index::load_for_test()
            .entries
            .into_iter()
            .find(|entry| {
                entry.path == crate::projects_index::normalize_path(&crafted_dir.to_string_lossy())
            });
        assert_eq!(
            after, before,
            "case {label}: refused open must not update recent projects"
        );
    }
}

#[tokio::test]
async fn open_after_activation_completes_with_warning_if_recent_index_save_fails() {
    const CHILD_ENV: &str = "SHELLX_CUT_TEST_OPEN_INDEX_FAIL_CHILD";
    const TEST_NAME: &str =
        "dispatch::project_workspace::tests::open_after_activation_completes_with_warning_if_recent_index_save_fails";

    if std::env::var_os(CHILD_ENV).is_some() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        let previous = ProjectStore::create(root.path(), "previous", None).unwrap();
        activate_project_locked(&state, previous).await.unwrap();
        let next = ProjectStore::create(root.path(), "next", None).unwrap();
        let next_dir = next.dir.clone();
        drop(next);
        let mut events = state.events.subscribe();

        let result = match project_open(&state, json!({"path": next_dir}), Actor::system()).await {
            Ok(res) => res,
            Err(err) => {
                let active_after = state.project.read().await.as_ref().map(|s| s.dir.clone());
                panic!(
                    "post-activation project.open returned Err after switching active project to {active_after:?}: {err:?}"
                );
            }
        };
        assert!(result.ok, "open reply must be ok: {result:?}");
        assert!(
            !result.warnings.as_deref().unwrap_or(&[]).is_empty(),
            "recent-index failure must surface as a plain warning on the open reply"
        );
        let active_dir = state
            .project
            .read()
            .await
            .as_ref()
            .expect("new project active")
            .dir
            .clone();
        assert_eq!(active_dir, next_dir);
        let mut saw_change = false;
        while let Ok(ev) = events.try_recv() {
            if matches!(ev, Event::ProjectChanged { open: true, .. }) {
                saw_change = true;
            }
        }
        assert!(saw_change, "successful open must emit ProjectChanged");
        return;
    }

    let broken_home = tempfile::tempdir().unwrap();
    // Putting a directory where `projects.json` lives makes index updates fail
    // deterministically on Linux, macOS, and Windows without process-wide env races.
    std::fs::create_dir(broken_home.path().join("projects.json")).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_ENV, "1")
        .env("SHELLX_CUT_HOME", broken_home.path())
        .output()
        .expect("spawn child test for recent-index warning path");
    assert!(
        output.status.success(),
        "child failed ({:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
