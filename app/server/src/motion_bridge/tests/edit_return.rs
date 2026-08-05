use super::*;

#[cfg(unix)]
#[tokio::test]
async fn verified_canvas_return_refreshes_the_stable_linked_clip() {
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
}
