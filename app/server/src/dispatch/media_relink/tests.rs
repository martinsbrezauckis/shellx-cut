use super::metadata::MetadataAssessment;
use super::plan::{accepted_changes, build_plan, metadata_for_source_identity, preview_result};
use super::scan::full_sha256;
#[cfg(unix)]
use super::scan::scan_folder;
use super::transition_gate::{install_relink_project_transition_gate, RelinkProjectTransitionGate};
use super::*;
use cut_core::store::is_exact_sha256;
use std::fs;

fn actor() -> Actor {
    Actor {
        kind: cut_core::ActorKind::Agent,
        name: "bulk-relink-test".into(),
        via: "test".into(),
        request: None,
    }
}

struct RelinkTransitionGateReset;

impl Drop for RelinkTransitionGateReset {
    fn drop(&mut self) {
        install_relink_project_transition_gate(None);
    }
}

fn plan(disposition: &str) -> RelinkPlan {
    RelinkPlan {
        project_identity: json!({"schema": "shellx-cut/project-identity/1", "origin_path_sha256": "sha256:test"}),
        project_revision: "op_000002".into(),
        root: "/safe".into(),
        scan_files: 1,
        scan_directories: 1,
        assets: vec![PlanAsset {
            asset_id: "a1".into(),
            expected_hash: format!("sha256:{}", "a".repeat(64)),
            old_path: "/gone/a.mov".into(),
            display_name: "a.mov".into(),
            disposition: disposition.into(),
            chosen_path: Some("/safe/a.mov".into()),
            chosen_hash: Some(format!("sha256:{}", "a".repeat(64))),
            diagnostics: vec![],
        }],
    }
}

#[test]
fn metadata_or_ambiguous_rows_are_never_applyable() {
    let error = accepted_changes(&plan("metadata_review"), &["a1".into()]).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    let accepted = accepted_changes(&plan("eligible_exact_hash"), &["a1".into()]).unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].chosen_path, "/safe/a.mov");
}

#[test]
fn sampled_identity_keeps_a_unique_strong_candidate_review_only() {
    let result = metadata_for_source_identity(
        "sha256s:sampled",
        MetadataAssessment {
            disposition: "metadata_review",
            diagnostics: vec!["basename_match".into(), "byte_size_match".into()],
        },
    );
    assert_eq!(result.disposition, "metadata_review");
    assert_eq!(
        result.diagnostics,
        vec![
            "basename_match",
            "byte_size_match",
            "complete_sha256_unavailable"
        ]
    );

    let unavailable = metadata_for_source_identity(
        "sha256s:sampled",
        MetadataAssessment {
            disposition: "no_match",
            diagnostics: Vec::new(),
        },
    );
    assert_eq!(unavailable.disposition, "hash_unavailable");
    assert_eq!(unavailable.diagnostics, vec!["complete_sha256_unavailable"]);
}

#[test]
fn metadata_review_preview_exposes_only_safe_fact_labels() {
    let mut review = plan("metadata_review");
    review.assets[0].chosen_path = Some("/private/recovery/clip.mov".into());
    review.assets[0].chosen_hash = Some("sha256:private-candidate".into());
    review.assets[0].diagnostics = vec![
        "basename_match".into(),
        "kind_match".into(),
        "byte_size_match".into(),
        "duration_match".into(),
    ];
    let preview = preview_result(&PreparedPlan {
        plan: review,
        plan_hash: "sha256:preview".into(),
        project_dir: std::path::PathBuf::from("/safe-project"),
    });
    assert_eq!(preview["assets"][0]["disposition"], "metadata_review");
    assert!(preview["assets"][0].get("candidate").is_none());
    assert_eq!(
        preview["assets"][0]["diagnostics"],
        json!([
            "basename_match",
            "kind_match",
            "byte_size_match",
            "duration_match"
        ])
    );
    let exposed = serde_json::to_string(&preview).unwrap();
    assert!(!exposed.contains("/private/recovery"));
    assert!(!exposed.contains("private-candidate"));
    assert!(!exposed.contains("/safe"));
}

#[test]
fn complete_hasher_never_returns_sampled_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("candidate.bin");
    fs::write(&path, b"complete content").unwrap();
    let hash = full_sha256(&path).unwrap();
    assert!(is_exact_sha256(&hash));
    assert!(!hash.starts_with("sha256s:"));
}

#[cfg(unix)]
#[test]
fn preview_refuses_symlink_tree_entries() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real.bin");
    fs::write(&target, b"x").unwrap();
    symlink(&target, dir.path().join("link.bin")).unwrap();
    let error = scan_folder(dir.path()).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
}

#[tokio::test]
async fn request_guarded_apply_writes_one_receipt_and_reopens_without_jobs() {
    let root = tempfile::tempdir().unwrap();
    let recovery = root.path().join("recovery");
    fs::create_dir(&recovery).unwrap();
    let candidate = recovery.join("restored.bin");
    fs::write(&candidate, b"B5 exact offline fixture").unwrap();
    let canonical_candidate = candidate
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let expected_hash = full_sha256(&candidate).unwrap();
    let project_dir = root.path().join("bulk-relink.cutproj");
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name": "bulk-relink", "dir": project_dir}),
        actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: root
                        .path()
                        .join("offline.bin")
                        .to_string_lossy()
                        .into_owned(),
                    hash: expected_hash.clone(),
                    probe: None,
                    transcript: Some("receipts/a1.words.json".into()),
                    perception: Some("receipts/a1.perception.json".into()),
                    proxy: Some("proxies/a1.mp4".into()),
                    filmstrip: Some("filmstrip/a1.jpg".into()),
                },
                actor(),
                None,
            )
            .unwrap();
    }
    let preview = crate::dispatch::dispatch(
        &state,
        "media.relink_preview",
        json!({"root": recovery}),
        actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    let body = preview.result.as_ref().unwrap();
    assert_eq!(body["assets"][0]["disposition"], "eligible_exact_hash");
    let expected_revision = body["project_revision"].as_str().unwrap();
    let plan_hash = body["plan_hash"].as_str().unwrap();
    let apply_args = json!({
        "root": recovery,
        "plan_hash": plan_hash,
        "accept": ["a1"],
        "request_id": "bulk-relink-request-1",
        "expected_revision": expected_revision,
    });
    let applied =
        crate::dispatch::dispatch(&state, "media.relink_apply", apply_args.clone(), actor()).await;
    assert!(applied.ok, "{:?}", applied.error);
    let receipt = applied.result.as_ref().unwrap();
    assert_eq!(receipt["schema"], RECEIPT_SCHEMA);
    assert_eq!(receipt["immutable"], true);
    assert_eq!(receipt["assets"][0]["expected_hash"], expected_hash);
    assert_eq!(receipt["assets"][0]["disposition"], "relinked");
    assert!(receipt["grouped_op_id"].as_str().is_some());
    assert!(
        state.jobs.list().is_empty(),
        "bulk relink must not start a job"
    );
    let (dir, op_count) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert_eq!(store.project.assets["a1"].path, canonical_candidate);
        assert_eq!(
            store.project.assets["a1"].proxy.as_deref(),
            Some("proxies/a1.mp4")
        );
        assert_eq!(
            store
                .log
                .read_all()
                .unwrap()
                .iter()
                .filter(|op| op.verb == "media.relink_apply")
                .count(),
            1
        );
        (store.dir.clone(), store.log.read_all().unwrap().len())
    };
    assert_eq!(
        fs::read_dir(dir.join("request-receipts")).unwrap().count(),
        1
    );
    let retry = crate::dispatch::dispatch(&state, "media.relink_apply", apply_args, actor()).await;
    assert_eq!(
        retry, applied,
        "request retry must replay the immutable result"
    );
    let reopened = ProjectStore::open(&dir).unwrap();
    assert_eq!(reopened.log.read_all().unwrap().len(), op_count);
    assert_eq!(reopened.project.assets["a1"].path, canonical_candidate);
}

#[tokio::test]
async fn prepared_source_guard_refuses_copied_same_revision_project() {
    let root = tempfile::tempdir().unwrap();
    let recovery = root.path().join("recovery");
    fs::create_dir(&recovery).unwrap();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let state = AppState::new();

    for (name, path) in [("a", &a_path), ("b", &b_path)] {
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": path}),
            actor(),
        )
        .await;
        assert!(created.ok, "{name} create failed: {:?}", created.error);
    }
    let a = ProjectStore::open(&a_path).unwrap();
    let b = ProjectStore::open(&b_path).unwrap();
    assert_eq!(
        a.log.current_revision().unwrap(),
        b.log.current_revision().unwrap(),
        "fixture requires copied projects with the same local revision"
    );

    let prepared = build_plan(snapshot(&state, recovery).await.unwrap()).unwrap();
    let error = ensure_prepared_source(&a, &prepared).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("project changed"));
}

#[tokio::test]
async fn relink_apply_pins_a_project_through_commit_before_project_open() {
    let root = tempfile::tempdir().unwrap();
    let recovery = root.path().join("recovery");
    fs::create_dir(&recovery).unwrap();
    let candidate = recovery.join("restored.bin");
    fs::write(&candidate, b"same revision relink fixture").unwrap();
    let candidate_path = candidate
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let expected_hash = full_sha256(&candidate).unwrap();
    let missing_path = root
        .path()
        .join("offline.bin")
        .to_string_lossy()
        .into_owned();
    let a_path = root.path().join("a.cutproj");
    let b_path = root.path().join("b.cutproj");
    let state = AppState::new();

    for (name, path) in [("a", &a_path), ("b", &b_path)] {
        let created = crate::dispatch::dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": path}),
            actor(),
        )
        .await;
        assert!(created.ok, "{name} create failed: {:?}", created.error);
        let mut guard = state.project.write().await;
        guard
            .as_mut()
            .unwrap()
            .record_import(
                Some("a1".into()),
                cut_core::Asset {
                    path: missing_path.clone(),
                    hash: expected_hash.clone(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                actor(),
                None,
            )
            .unwrap();
    }
    let a_revision = ProjectStore::open(&a_path)
        .unwrap()
        .log
        .current_revision()
        .unwrap();
    let b_revision = ProjectStore::open(&b_path)
        .unwrap()
        .log
        .current_revision()
        .unwrap();
    assert_eq!(
        a_revision, b_revision,
        "fixture requires colliding local revisions"
    );
    assert!(
        crate::dispatch::dispatch(&state, "project.open", json!({"path": a_path}), actor(),)
            .await
            .ok
    );
    let preview = crate::dispatch::dispatch(
        &state,
        "media.relink_preview",
        json!({"root": recovery}),
        actor(),
    )
    .await;
    assert!(preview.ok, "A preview failed: {:?}", preview.error);
    let preview = preview.result.unwrap();
    assert_eq!(preview["project_revision"], a_revision.unwrap());
    assert_eq!(preview["assets"][0]["disposition"], "eligible_exact_hash");

    let recovery_text = recovery.to_string_lossy().into_owned();
    let gate = RelinkProjectTransitionGate::new(recovery_text.clone());
    install_relink_project_transition_gate(Some(gate.clone()));
    let _gate_reset = RelinkTransitionGateReset;
    let project_pinned = gate.project_pinned.notified();
    let relink_committed = gate.relink_committed.notified();
    let apply_state = state.clone();
    let plan_hash = preview["plan_hash"].clone();
    let expected_revision = preview["project_revision"].clone();
    let apply = tokio::spawn(async move {
        crate::dispatch::dispatch(
            &apply_state,
            "media.relink_apply",
            json!({
                "root": recovery_text,
                "plan_hash": plan_hash,
                "accept": ["a1"],
                "request_id": "held-a-relink-request",
                "expected_revision": expected_revision,
            }),
            actor(),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), project_pinned)
        .await
        .expect("relink did not pin project A before timeout");
    assert!(
        state.project_transition.try_lock().is_err(),
        "B5 must own project-transition before snapshotting A"
    );

    let open_started = std::sync::Arc::new(tokio::sync::Notify::new());
    let open_started_in_task = open_started.clone();
    let open_state = state.clone();
    let open = tokio::spawn(async move {
        open_started_in_task.notify_one();
        crate::dispatch::dispatch(
            &open_state,
            "project.open",
            json!({"path": b_path}),
            actor(),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), open_started.notified())
        .await
        .expect("project.open(B) task did not start");
    tokio::task::yield_now().await;
    assert!(
        !open.is_finished(),
        "project.open(B) must wait while B5 owns project A"
    );
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.name,
        "a"
    );

    gate.continue_after_pin.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), relink_committed)
        .await
        .expect("B5 did not commit A before timeout");
    assert_eq!(
        state.project.read().await.as_ref().unwrap().project.assets["a1"].path,
        candidate_path,
        "the held relink must commit only to A"
    );
    assert!(
        !open.is_finished(),
        "B must wait until A's grouped relink returns"
    );

    gate.continue_after_commit.notify_one();
    let applied = tokio::time::timeout(std::time::Duration::from_secs(2), apply)
        .await
        .expect("B5 request did not return")
        .expect("B5 task panicked");
    assert!(applied.ok, "A B5 apply failed: {:?}", applied.error);
    let opened_b = tokio::time::timeout(std::time::Duration::from_secs(2), open)
        .await
        .expect("project.open(B) did not return")
        .expect("project.open(B) task panicked");
    assert!(opened_b.ok, "B open failed: {:?}", opened_b.error);
    let b = state.project.read().await;
    let b = b.as_ref().unwrap();
    assert_eq!(b.project.name, "b");
    assert_eq!(
        b.project.assets["a1"].path, missing_path,
        "B must not receive A's relinked path"
    );
    assert!(
        b.log
            .read_all()
            .unwrap()
            .iter()
            .all(|op| op.verb != "media.relink_apply"),
        "B must not receive A's relink operation or identity"
    );
}
