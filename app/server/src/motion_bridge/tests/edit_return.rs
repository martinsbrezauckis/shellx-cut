// Every test in this file is unix-only (they build symlink/permission cases), so
// on Windows the glob has no consumer and would be an unused import.
#[cfg(unix)]
use super::*;

#[cfg(unix)]
#[test]
fn verified_canvas_return_refreshes_the_stable_linked_clip() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use std::os::unix::fs::PermissionsExt;

        let _guard = MOTION_ENV_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("canvas-return.cutproj");
        let source_package = root.path().join("source-package");
        fs::create_dir_all(&source_package).unwrap();
        fs::write(
            source_package.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "schema": "shellx-motion/package-manifest@1",
                "id": "pkg-lower-third",
                "motion": "motion.json",
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            source_package.join("motion.json"),
            crate::motion_test_fixtures::linked_effect_motion_document(),
        )
        .unwrap();

        let plan_path = root.path().join("initial-plan.json");
        write_rendered_media_plan(&plan_path, &root.path().join("initial.mp4"), false);
        let state = AppState::new();
        assert!(
            crate::dispatch::dispatch(
                &state,
                "project.create",
                json!({"name":"canvas-return", "dir":project_dir}),
                Actor::system(),
            )
            .await
            .ok
        );
        let imported = crate::dispatch::dispatch(
            &state,
            "motion.apply_import",
            json!({"path":plan_path, "packageDir":source_package, "dryRun":false}),
            Actor::system(),
        )
        .await;
        assert!(imported.ok, "initial linked import failed: {imported:?}");
        let clip = imported.result.as_ref().unwrap()["clips"][0]
            .as_str()
            .unwrap()
            .to_string();

        let edited_package = root.path().join("canvas-edited-package");
        fs::create_dir_all(&edited_package).unwrap();
        fs::copy(
            source_package.join("manifest.json"),
            edited_package.join("manifest.json"),
        )
        .unwrap();
        let mut edited_motion: Value =
            serde_json::from_slice(&fs::read(source_package.join("motion.json")).unwrap()).unwrap();
        edited_motion["durationMs"] = json!(4200);
        fs::write(
            edited_package.join("motion.json"),
            serde_json::to_vec_pretty(&edited_motion).unwrap(),
        )
        .unwrap();
        let edited_package = edited_package.canonicalize().unwrap();
        let source_revision = motion_package_revision(&source_package).unwrap();
        let edited_revision = motion_package_revision(&edited_package).unwrap();
        let request = crate::motion_edit_return::create_request(
            &project_dir,
            &clip,
            &safe_fragment(&clip),
            "pkg-lower-third",
            "motion-lower-third",
            &source_revision,
        )
        .unwrap();
        let request_json: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
        let revision_token = "r".repeat(32);
        fs::write(
            request
                .parent()
                .unwrap()
                .join(format!("ready-{revision_token}.json")),
            serde_json::to_vec_pretty(&json!({
                "schema": "shellx-canvas/motion-edit-return-ready@1",
                "state": "ready",
                "sessionToken": request_json["sessionToken"],
                "clip": clip,
                "packageId": "pkg-lower-third",
                "motionId": "motion-lower-third",
                "packageDir": edited_package,
                "sourceRevision": edited_revision,
                "revisionToken": revision_token,
                "completedAtUnixMs": 42,
                "localOnly": true,
                "remotePublish": false,
            }))
            .unwrap(),
        )
        .unwrap();

        let fake_motion = root.path().join("fake-motion.sh");
        fs::write(
            &fake_motion,
            r#"#!/bin/sh
    out=""
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "--out" ]; then shift; out="$1"; fi
      shift
    done
    mkdir -p "$(dirname "$out")"
    printf 'canvas-return-render' > "$out"
    sha="$(sha256sum "$out" | cut -d' ' -f1)"
    receipt="$out.receipt.json"
    printf '{"id":"canvas-return-receipt","packageId":"pkg-lower-third"}\n' > "$receipt"
    printf '{"ok":true,"output":{"path":"%s","sha256":"%s"},"receiptPath":"%s","receipt":{"id":"canvas-return-receipt","packageId":"pkg-lower-third"}}\n' "$out" "$sha" "$receipt"
    "#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake_motion).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_motion, permissions).unwrap();
        let _motion_bin = EnvRestore::set(ENV_MOTION_BIN, &fake_motion);

        let refreshed = crate::dispatch::dispatch(
            &state,
            "motion.link.refresh",
            json!({"clip":clip}),
            Actor::system(),
        )
        .await;
        assert!(refreshed.ok, "Canvas-return refresh failed: {refreshed:?}");
        assert_eq!(
            refreshed.result.as_ref().unwrap()["canvasReturn"]["applied"],
            json!(true)
        );
        assert_eq!(
            refreshed.result.as_ref().unwrap()["sourceRevision"],
            json!(edited_revision)
        );
        let materialized =
            crate::dispatch::dispatch(&state, "project.state", json!({}), Actor::system()).await;
        let linked = materialized.result.as_ref().unwrap()["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|track| track["clips"].as_array().into_iter().flatten())
            .find(|candidate| candidate["id"] == json!(clip))
            .unwrap();
        assert_eq!(linked["motion_link"]["sourcePath"], json!(edited_package));
    });
}

#[cfg(unix)]
#[test]
fn linked_motion_refresh_and_relink_preserve_clip_identity() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use std::os::unix::fs::PermissionsExt;

        let _guard = MOTION_ENV_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("motion-linked.cutproj");
        let package_dir = tmp.path().join("package");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(
            package_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&json!({
                "schema":"shellx-motion/package-manifest@1",
                "id":"pkg-lower-third",
                "motion":"motion.json"
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            package_dir.join("motion.json"),
            crate::motion_test_fixtures::linked_effect_motion_document(),
        )
        .unwrap();
        let render_path = tmp.path().join("render").join("initial.png");
        let plan_path = tmp.path().join("cut-import-plan.json");
        write_rendered_media_plan(&plan_path, &render_path, false);

        let fake_motion = tmp.path().join("fake-motion.sh");
        let fake_motion_script = r#"#!/bin/sh
    out=""
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "--out" ]; then shift; out="$1"; fi
      shift
    done
    mkdir -p "$(dirname "$out")"
    printf 'verified linked motion render' > "$out"
    sha="$(sha256sum "$out" | cut -d' ' -f1)"
    receipt="$out.receipt.json"
    printf '{"id":"refresh-receipt","packageId":"pkg-lower-third"}\n' > "$receipt"
    printf '{"ok":true,"output":{"path":"%s","sha256":"%s"},"receiptPath":"%s","receipt":{"id":"refresh-receipt","packageId":"pkg-lower-third"}}\n' "$out" "$sha" "$receipt"
    "#;
        fs::write(&fake_motion, fake_motion_script).unwrap();
        let mut permissions = fs::metadata(&fake_motion).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_motion, permissions).unwrap();
        let _bin = EnvRestore::set(ENV_MOTION_BIN, &fake_motion);
        let fake_canvas = tmp.path().join("fake-canvas.sh");
        let canvas_args = tmp.path().join("canvas-args.txt");
        fs::write(
            &fake_canvas,
            format!(
                "#!/bin/sh\nprintf '%s\\n%s\\n%s\\n%s\\n' \"$1\" \"$2\" \"$3\" \"$4\" > \"{}\"\n",
                canvas_args.display()
            ),
        )
        .unwrap();
        let mut canvas_permissions = fs::metadata(&fake_canvas).unwrap().permissions();
        canvas_permissions.set_mode(0o755);
        fs::set_permissions(&fake_canvas, canvas_permissions).unwrap();
        let _canvas_bin = EnvRestore::set(LEGACY_CANVAS_BIN_ENV, &fake_canvas);

        let state = AppState::new();
        assert!(
            crate::dispatch::dispatch(
                &state,
                "project.create",
                json!({"name":"motion-linked", "dir":project_dir}),
                Actor::system(),
            )
            .await
            .ok
        );
        let imported = crate::dispatch::dispatch(
            &state,
            "motion.apply_import",
            json!({"path":plan_path, "packageDir":package_dir, "dryRun":false}),
            Actor::system(),
        )
        .await;
        assert!(imported.ok, "linked import failed: {imported:?}");
        let origin_attestation = imported.result.as_ref().unwrap()["lineageProofs"][0].clone();
        let clip_id = imported.result.as_ref().unwrap()["clips"][0]
            .as_str()
            .unwrap()
            .to_string();

        fs::write(
            &fake_motion,
            "#!/bin/sh\nprintf 'intentional linked render failure\\n' >&2\nexit 17\n",
        )
        .unwrap();
        let failed = crate::dispatch::dispatch(
            &state,
            "motion.link.refresh",
            json!({"clip":clip_id}),
            Actor::system(),
        )
        .await;
        assert!(!failed.ok, "failed linked render must be surfaced");
        {
            let guard = state.project.read().await;
            let store = guard.as_ref().unwrap();
            let (track_id, index) = store.project.find_clip(&clip_id).unwrap();
            assert!(matches!(
                &store.project.track(track_id).unwrap().clips[index],
                cut_core::Clip::Media(media) if media.asset == "a1"
            ));
        }
        fs::write(&fake_motion, fake_motion_script).unwrap();

        let refreshed = crate::dispatch::dispatch(
            &state,
            "motion.link.refresh",
            json!({"clip":clip_id}),
            Actor::system(),
        )
        .await;
        assert!(refreshed.ok, "linked refresh failed: {refreshed:?}");
        assert_eq!(refreshed.result.as_ref().unwrap()["clip"], json!(clip_id));
        assert!(refreshed.result.as_ref().unwrap()["receiptPath"]
            .as_str()
            .is_some_and(|path| path.ends_with(".receipt.json")));
        let state_after =
            crate::dispatch::dispatch(&state, "project.state", json!({}), Actor::system()).await;
        let clip = state_after.result.as_ref().unwrap()["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|track| track["clips"].as_array().into_iter().flatten())
            .find(|clip| clip["id"] == json!(clip_id))
            .unwrap();
        assert_eq!(clip["motion_link"]["state"], json!("linked-current"));
        assert_eq!(
            clip["motion_link"]["sourceRevisionKind"],
            json!("motion-package")
        );
        assert_eq!(clip["motion_link"]["originAttestation"], origin_attestation);
        assert!(clip["motion_link"]["lastReceiptPath"]
            .as_str()
            .is_some_and(|path| path.ends_with(".receipt.json")));
        crate::motion_test_fixtures::assert_linked_effect_summary(clip);
        assert_ne!(clip["asset"], json!("a1"));

        let editing = crate::dispatch::dispatch(
            &state,
            "motion.link.edit",
            json!({"clip":clip_id}),
            Actor::system(),
        )
        .await;
        assert!(editing.ok, "ShellX Motion launch failed: {editing:?}");
        let editing_result = editing.result.as_ref().expect("editing result");
        assert_eq!(
            editing_result["schema"],
            json!("shellx-cut/motion-link-edit@1")
        );
        assert_eq!(
            editing_result["sourceRevision"],
            clip["motion_link"]["sourceRevision"]
        );
        assert_eq!(
            editing_result["returnChannel"],
            json!({"state":"pending", "pathPrivate":true})
        );
        let launch_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < launch_deadline {
            if canvas_args.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let launched_args = fs::read_to_string(&canvas_args)
            .expect("fake ShellX Motion editor did not record launch arguments within 5s");
        let mut launched_args = launched_args.lines();
        assert_eq!(launched_args.next(), Some("--motion-package"));
        assert_eq!(
            launched_args.next(),
            Some(package_dir.canonicalize().unwrap().to_str().unwrap())
        );
        assert_eq!(launched_args.next(), Some("--motion-cut-return-request"));
        assert!(PathBuf::from(launched_args.next().unwrap()).is_file());

        let wrong_package = tmp.path().join("wrong-package");
        fs::create_dir_all(&wrong_package).unwrap();
        fs::write(
            wrong_package.join("manifest.json"),
            r#"{"id":"pkg-other","motion":"motion.json"}"#,
        )
        .unwrap();
        fs::write(
            wrong_package.join("motion.json"),
            r#"{"id":"motion-other"}"#,
        )
        .unwrap();
        let refused = crate::dispatch::dispatch(
            &state,
            "motion.link.relink",
            json!({"clip":clip_id, "package_dir":wrong_package}),
            Actor::system(),
        )
        .await;
        assert!(!refused.ok, "identity-changing relink must fail");

        let undone =
            crate::dispatch::dispatch(&state, "project.undo", json!({}), Actor::system()).await;
        assert!(undone.ok, "refresh undo failed: {undone:?}");
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&clip_id).unwrap();
        assert!(matches!(
            &store.project.track(track_id).unwrap().clips[index],
            cut_core::Clip::Media(media) if media.asset == "a1"
        ));
    });
}
