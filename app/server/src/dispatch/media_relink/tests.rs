use super::plan::accepted_changes;
use super::scan::{full_sha256, scan_folder};
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

fn plan(disposition: &str) -> RelinkPlan {
    RelinkPlan {
        project_identity: json!({"schema": PROJECT_ID_SCHEMA, "origin_path_sha256": "sha256:test"}),
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
    let error = accepted_changes(&plan("metadata_only"), &["a1".into()]).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    let accepted = accepted_changes(&plan("eligible_exact_hash"), &["a1".into()]).unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].chosen_path, "/safe/a.mov");
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
        assert_eq!(store.project.assets["a1"].path, candidate.to_string_lossy());
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
    assert_eq!(
        reopened.project.assets["a1"].path,
        candidate.to_string_lossy()
    );
}
