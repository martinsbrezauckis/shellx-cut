#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::dispatch;
    use cut_core::Asset;
    use serde_json::json;

    fn actor() -> Actor {
        Actor {
            kind: cut_core::ActorKind::Agent,
            name: "portable-test".into(),
            via: "test".into(),
            request: None,
        }
    }

    async fn wait_for_job(state: &AppState, job_id: &str) -> crate::jobs::JobRecord {
        for _ in 0..100 {
            let record = state.jobs.get(job_id).expect("package job exists");
            if matches!(
                record.state,
                crate::jobs::JobState::Done | crate::jobs::JobState::Failed
            ) {
                return record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("portable package job did not finish")
    }

    #[tokio::test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    async fn packages_only_referenced_media_without_mutating_source_ops_or_cache() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let source = root.path().join("source.mov");
        let duplicate = root.path().join("duplicate.mov");
        fs::write(&source, b"portable package fixture").unwrap();
        fs::write(&duplicate, b"portable package fixture").unwrap();
        let source_hash = format!("sha256:{:x}", Sha256::digest(b"portable package fixture"));
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let (source_dir, before_log, before_cache, revision) = {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: source.to_string_lossy().into_owned(),
                        hash: source_hash,
                        probe: Some(json!({"duration_ms": 100})),
                        transcript: Some("receipts/a1.words.json".into()),
                        perception: None,
                        proxy: Some("proxies/a1.mp4".into()),
                        filmstrip: None,
                    },
                    actor(),
                    None,
                )
                .unwrap();
            store
                .record_import(
                    Some("a2".into()),
                    Asset {
                        path: duplicate.to_string_lossy().into_owned(),
                        hash: format!("sha256:{:x}", Sha256::digest(b"portable package fixture")),
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
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a2", "track":"v1", "at_ms":100, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
            (
                store.dir.clone(),
                fs::read(store.dir.join("ops.jsonl")).unwrap(),
                fs::read(store.dir.join("project.json")).unwrap(),
                store.log.current_revision().unwrap().unwrap(),
            )
        };
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(planned.ok, "{:?}", planned.error);
        let plan = planned.result.unwrap();
        assert_eq!(plan["plan"]["unique_media_count"], 1);
        assert_eq!(plan["plan"]["source_file_count"], 2);
        let created = dispatch(
            &state,
            "project.package_create",
            json!({"destination":destination, "name":"packed", "plan_hash":plan["plan_hash"]}),
            actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        let job = wait_for_job(&state, created.result.unwrap()["job_id"].as_str().unwrap()).await;
        assert_eq!(job.state, crate::jobs::JobState::Done, "{:?}", job.error);
        let packed = destination.join("packed.cutproj");
        assert!(packed.join("package.manifest.json").is_file());
        assert!(!packed.join("proxies").exists());
        assert!(!packed.join("receipts").exists());
        let reopened = ProjectStore::open(&packed).unwrap();
        assert_eq!(reopened.project.assets.len(), 2);
        assert!(reopened.project.assets["a1"]
            .path
            .starts_with("media/sha256/"));
        assert_eq!(
            reopened.project.assets["a1"].path,
            reopened.project.assets["a2"].path
        );
        assert!(reopened.project.assets["a1"].proxy.is_none());
        assert!(!fs::read(packed.join("ops.jsonl"))
            .unwrap()
            .windows(source.to_string_lossy().len())
            .any(|chunk| chunk == source.to_string_lossy().as_bytes()));
        assert_eq!(fs::read(source_dir.join("ops.jsonl")).unwrap(), before_log);
        assert_eq!(
            fs::read(source_dir.join("project.json")).unwrap(),
            before_cache
        );
        let source_revision = state
            .project
            .read()
            .await
            .as_ref()
            .unwrap()
            .log
            .current_revision()
            .unwrap()
            .unwrap();
        assert_eq!(source_revision, revision);
    }

    #[tokio::test]
    async fn rejects_a_stale_b5_receipt_before_creating_a_job() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok);
        let plan = dispatch(
            &state,
            "project.package_plan",
            json!({
                "destination":destination,
                "name":"packed",
                "b5_receipt": {"schema":B5_RECEIPT_SCHEMA, "immutable":true, "post_revision":"stale"}
            }),
            actor(),
        )
        .await;
        assert!(!plan.ok);
        assert_eq!(plan.error.unwrap().code, error_codes::CONFLICT);
        assert!(state.jobs.list().is_empty());
    }

    #[tokio::test]
    async fn consumes_the_committed_b5_receipt_instead_of_matching_media_again() {
        let root = tempfile::tempdir().unwrap();
        let recovery = root.path().join("recovery");
        let destination = root.path().join("packages");
        fs::create_dir(&recovery).unwrap();
        fs::create_dir(&destination).unwrap();
        let restored = recovery.join("restored.mov");
        fs::write(&restored, b"B5-to-B6 exact fixture").unwrap();
        let hash = format!("sha256:{:x}", Sha256::digest(b"B5-to-B6 exact fixture"));
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok);
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: root
                            .path()
                            .join("missing.mov")
                            .to_string_lossy()
                            .into_owned(),
                        hash,
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
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
        }
        let preview = dispatch(
            &state,
            "media.relink_preview",
            json!({"root":recovery}),
            actor(),
        )
        .await;
        assert!(preview.ok, "{:?}", preview.error);
        let preview = preview.result.unwrap();
        let receipt = dispatch(
            &state,
            "media.relink_apply",
            json!({
                "root":recovery,
                "plan_hash":preview["plan_hash"],
                "accept":["a1"],
                "request_id":"b6-b5-receipt-001",
                "expected_revision":preview["project_revision"],
            }),
            actor(),
        )
        .await;
        assert!(receipt.ok, "{:?}", receipt.error);
        let receipt = receipt.result.unwrap();
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed", "b5_receipt":receipt}),
            actor(),
        )
        .await;
        assert!(planned.ok, "{:?}", planned.error);
        assert!(planned.result.unwrap()["plan"]["b5_receipt_sha256"]
            .as_str()
            .is_some_and(is_exact_sha256));
    }

    #[tokio::test]
    async fn refuses_offline_referenced_media_before_creating_a_job() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let state = AppState::new();
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await;
        assert!(created.ok);
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: root.path().join("offline.mov").to_string_lossy().into_owned(),
                        hash: format!("sha256:{}", "a".repeat(64)),
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
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
        }
        let planned = dispatch(
            &state,
            "project.package_plan",
            json!({"destination":destination, "name":"packed"}),
            actor(),
        )
        .await;
        assert!(!planned.ok);
        assert_eq!(planned.error.unwrap().code, error_codes::NOT_FOUND);
        assert!(state.jobs.list().is_empty());
        assert!(fs::read_dir(&destination).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn cancellation_after_media_copy_removes_the_private_stage() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("packages");
        fs::create_dir(&destination).unwrap();
        let source = root.path().join("source.mov");
        fs::write(&source, b"cancel after copy").unwrap();
        let state = AppState::new();
        assert!(dispatch(
            &state,
            "project.create",
            json!({"name":"source", "dir":root.path().join("source.cutproj")}),
            actor(),
        )
        .await
        .ok);
        {
            let mut guard = state.project.write().await;
            let store = guard.as_mut().unwrap();
            store
                .record_import(
                    Some("a1".into()),
                    Asset {
                        path: source.to_string_lossy().into_owned(),
                        hash: format!("sha256:{:x}", Sha256::digest(b"cancel after copy")),
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
            store
                .apply(
                    "edit.insert",
                    json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,100], "ripple":false}),
                    actor(),
                    None,
                )
                .unwrap();
        }
        let prepared = prepare(
            &state,
            destination.to_string_lossy().into_owned(),
            "packed".into(),
            None,
        )
        .await
        .unwrap();
        let cancellation = crate::jobs::JobCancellation::test_active();
        let progress_cancellation = cancellation.clone();
        let result = publish_package(
            prepared,
            &cancellation,
            move |progress, _| {
                if progress >= 0.75 {
                    progress_cancellation.request_cancel();
                }
            },
            |_, _| panic!("cancelled package must not reach publication"),
        );
        let error = match result {
            Ok(_) => panic!("cancelled package unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.code, "job_cancelled");
        assert!(fs::read_dir(&destination).unwrap().next().is_none());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn directory_publication_never_replaces_a_racing_destination() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage.cutproj");
        let target = root.path().join("target.cutproj");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("stage.txt"), b"stage").unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("existing.txt"), b"existing").unwrap();
        assert!(publish_new_directory(&stage, &target).is_err());
        assert!(stage.join("stage.txt").is_file());
        assert_eq!(fs::read(target.join("existing.txt")).unwrap(), b"existing");
    }

    #[test]
    fn cleanup_retains_a_stage_with_an_unexpected_entry() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join(".packed.package-test.stage");
        let package = stage.join("packed.cutproj");
        fs::create_dir(&stage).unwrap();
        fs::create_dir(&package).unwrap();
        fs::write(package.join("unexpected.txt"), b"do not delete").unwrap();
        cleanup_stage(&stage, "packed");
        assert!(stage.exists());
        assert!(package.join("unexpected.txt").is_file());
    }
}
