use super::*;

async fn assert_lineaged_mutation_rejected(label: &str, mutate: impl FnOnce(&mut Value)) {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join(format!("{label}.png"));
    let plan_path = tmp.path().join(format!("{label}.cut-import-plan.json"));
    write_rendered_media_plan(&plan_path, &render_path, false);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    mutate(&mut plan);
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;
    assert!(!result.ok, "{label} mutation must fail closed: {result:?}");
}

#[tokio::test]
async fn current_sdk_receipt_binds_default_timing_track_and_media() {
    for (label, pointer, replacement) in [
        ("start", "/operations/0/startMs", json!(1)),
        ("duration", "/operations/0/durationMs", json!(1499)),
        ("width", "/operations/0/media/width", json!(1279)),
        ("height", "/operations/0/media/height", json!(719)),
        ("fps", "/operations/0/media/fps", json!(29)),
    ] {
        assert_lineaged_mutation_rejected(label, |plan| {
            *plan.pointer_mut(pointer).expect("fixture field") = replacement;
        })
        .await;
    }

    assert_lineaged_mutation_rejected("track", |plan| {
        plan["operations"][0]["track"] = json!("overlay-2");
    })
    .await;
}

#[tokio::test]
async fn current_sdk_receipt_cannot_authorize_duplicate_operations() {
    assert_lineaged_mutation_rejected("duplicate", |plan| {
        let operation = plan["operations"][0].clone();
        plan["operations"].as_array_mut().unwrap().push(operation);
    })
    .await;
}

#[tokio::test]
async fn current_sdk_receipt_binds_document_and_diagnostics() {
    assert_lineaged_mutation_rejected("document", |plan| {
        plan["document"]["width"] = json!(1279);
    })
    .await;
    assert_lineaged_mutation_rejected("unsupported", |plan| {
        plan["unsupported"] = json!([{
            "layerId": "layer-1",
            "feature": "effect.blur",
            "reason": "Rendered fallback required"
        }]);
    })
    .await;
}

#[tokio::test]
async fn map_reports_exact_changed_and_unavailable_current_package_lineage() {
    let tmp = tempfile::tempdir().unwrap();
    let package_dir = tmp.path().join("package");
    let lineage = write_current_package(&package_dir, false);
    let render_path = tmp.path().join("render").join("current-package.png");
    let plan_path = tmp.path().join("current-package.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    replace_rendered_plan_lineage(&plan_path, &render_path, lineage.clone());

    let omitted = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path}),
        Actor::system(),
    )
    .await;
    let omitted_proof = &omitted.result.as_ref().unwrap()["lineageProofs"][0]["currentPackage"];
    assert_eq!(omitted_proof["status"], json!("unavailable"));
    assert_eq!(omitted_proof["reason"], json!("package-dir-not-provided"));

    let exact = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path, "packageDir":package_dir}),
        Actor::system(),
    )
    .await;
    assert!(
        exact.ok,
        "exact current package should not disturb import: {exact:?}"
    );
    let exact_proof = &exact.result.as_ref().unwrap()["lineageProofs"][0]["currentPackage"];
    assert_eq!(
        exact_proof["schema"],
        json!("shellx-cut/current-motion-package-lineage@1")
    );
    assert_eq!(exact_proof["status"], json!("exact"));
    assert_eq!(exact_proof["lineage"], lineage);
    assert_eq!(exact_proof["changedFields"], json!([]));
    assert!(exact_proof["reason"].is_null());

    fs::write(
        package_dir.join("motion.json"),
        serde_json::to_vec_pretty(&json!({
            "schema": "shellx-motion/motion@1",
            "id": "motion-lower-third",
            "durationMs": 2400,
        }))
        .unwrap(),
    )
    .unwrap();
    let changed = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path, "packageDir":package_dir}),
        Actor::system(),
    )
    .await;
    assert!(
        changed.ok,
        "changed package is reporting evidence, not artifact authorization: {changed:?}"
    );
    let changed_proof = &changed.result.as_ref().unwrap()["lineageProofs"][0]["currentPackage"];
    assert_eq!(changed_proof["status"], json!("changed"));
    assert_eq!(changed_proof["changedFields"], json!(["motionSha256"]));
    assert_ne!(
        changed_proof["lineage"]["motionSha256"],
        lineage["motionSha256"]
    );

    let unavailable = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path, "packageDir":tmp.path().join("missing-package")}),
        Actor::system(),
    )
    .await;
    assert!(
        unavailable.ok,
        "missing optional package must not invalidate attested media: {unavailable:?}"
    );
    let unavailable_proof =
        &unavailable.result.as_ref().unwrap()["lineageProofs"][0]["currentPackage"];
    assert_eq!(unavailable_proof["status"], json!("unavailable"));
    assert_eq!(unavailable_proof["reason"], json!("package-unreadable"));
    assert!(unavailable_proof["lineage"].is_null());
}

#[tokio::test]
async fn map_derives_exact_current_gltf_five_hash_lineage() {
    let tmp = tempfile::tempdir().unwrap();
    let package_dir = tmp.path().join("gltf-package");
    let lineage = write_current_package(&package_dir, true);
    let render_path = tmp.path().join("render").join("current-gltf.png");
    let plan_path = tmp.path().join("current-gltf.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    replace_rendered_plan_lineage(&plan_path, &render_path, lineage.clone());

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path, "packageDir":package_dir}),
        Actor::system(),
    )
    .await;
    assert!(
        result.ok,
        "current glTF package should derive independently: {result:?}"
    );
    let current = &result.result.as_ref().unwrap()["lineageProofs"][0]["currentPackage"];
    assert_eq!(current["status"], json!("exact"));
    assert_eq!(current["lineage"], lineage);
    assert_eq!(current["changedFields"], json!([]));
}

fn write_current_package(root: &Path, gltf: bool) -> Value {
    fs::create_dir_all(root).unwrap();
    let motion_bytes = serde_json::to_vec_pretty(&json!({
        "schema": "shellx-motion/motion@1",
        "id": "motion-lower-third",
        "durationMs": 1500,
    }))
    .unwrap();
    let mut manifest = json!({
        "schema": "shellx-motion/package-manifest@1",
        "id": "pkg-lower-third",
        "motion": "motion.json",
    });
    let mut adapter_lineage = None;
    if gltf {
        let source_bytes = br#"{"asset":{"version":"2.0"},"scenes":[{}],"scene":0}"#;
        let normalized_bytes = br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{}]}"#;
        let receipt_bytes = serde_json::to_vec_pretty(&json!({
            "schema": "shellx-motion/receipt@1",
            "id": "adapter-lowering-gltf-current",
            "operation": "adapter.lower",
            "status": "passed",
            "packageId": "pkg-lower-third",
            "inputHashes": {"source":sha256_hex(normalized_bytes)},
            "createdAt": "2026-07-16T00:00:00.000Z",
            "lane": "adapter",
            "output": {"adapterId":"adapter.gltf", "motionId":"motion-lower-third"},
            "warnings": [],
        }))
        .unwrap();
        fs::create_dir_all(root.join("source")).unwrap();
        fs::create_dir_all(root.join("receipts")).unwrap();
        fs::write(root.join("source/input.gltf"), source_bytes).unwrap();
        fs::write(root.join("source/normalized.gltf.json"), normalized_bytes).unwrap();
        fs::write(
            root.join("receipts/adapter-lowering.receipt.json"),
            &receipt_bytes,
        )
        .unwrap();
        manifest["data"] = json!({
            "adapter": {
                "schema": "shellx-motion/adapter-source@1",
                "id": "adapter.gltf",
                "source": "source/input.gltf",
                "sourceSha256": sha256_hex(source_bytes),
                "loweringSource": "source/normalized.gltf.json",
                "loweringSourceSha256": sha256_hex(normalized_bytes),
                "loweringReceipt": "receipts/adapter-lowering.receipt.json",
            }
        });
        adapter_lineage = Some(json!({
            "adapterId": "adapter.gltf",
            "sourceSha256": sha256_hex(source_bytes),
            "normalizedSourceSha256": sha256_hex(normalized_bytes),
            "loweringReceiptSha256": sha256_hex(&receipt_bytes),
        }));
    }
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(root.join("manifest.json"), &manifest_bytes).unwrap();
    fs::write(root.join("motion.json"), &motion_bytes).unwrap();
    let mut lineage = json!({
        "schema": "shellx-motion/package-render-lineage@1",
        "manifestSha256": sha256_hex(&manifest_bytes),
        "motionSha256": sha256_hex(&motion_bytes),
    });
    if let Some(adapter) = adapter_lineage {
        for (field, value) in adapter.as_object().unwrap() {
            lineage[field] = value.clone();
        }
    }
    lineage
}

fn replace_rendered_plan_lineage(plan_path: &Path, render_path: &Path, lineage: Value) {
    let rendered_media = write_attested_rendered_media_kind(
        plan_path,
        render_path,
        "pkg-lower-third",
        "motion-lower-third",
        "current-package",
        Some(lineage),
    );
    let mut plan: Value = serde_json::from_slice(&fs::read(plan_path).unwrap()).unwrap();
    plan["operations"][0]["renderedMedia"] = rendered_media.clone();
    plan["receipt"] =
        lineaged_cut_plan_receipt("pkg-lower-third", "motion-lower-third", &rendered_media, 1);
    fs::write(plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}
