use super::*;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;

#[path = "tests/command_contract.rs"]
mod command_contract;
#[path = "tests/edit_return.rs"]
mod edit_return;
#[path = "tests/lineage_integrity.rs"]
mod lineage_integrity;

static MOTION_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct EnvRestore {
    key: &'static str,
    value: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            key,
            value: previous,
        }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self {
            key,
            value: previous,
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match &self.value {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn parses_last_json_line_from_motion_cli_output() {
    let out = "progress\n{\"ok\":true,\"warnings\":[\"low contrast\"],\"render\":{\"outputPath\":\"motion-out.mp4\"}}\n";
    let parsed = parse_motion_connector_stdout(out).expect("json parses");
    assert_eq!(parsed["ok"], json!(true));
    assert_eq!(motion_warnings(&parsed), vec!["low contrast".to_string()]);
}

#[test]
fn rejects_motion_cli_ok_false() {
    let err = parse_motion_connector_stdout("{\"ok\":false,\"error\":{\"message\":\"no render\"}}")
        .unwrap_err();
    assert_eq!(err.code, error_codes::SIDECAR);
    assert!(err.message.contains("ok:false"));
}

#[test]
fn tail_handles_multibyte_utf8_boundaries() {
    let result = std::panic::catch_unwind(|| tail("αβγδε", 3));
    assert!(
        result.is_ok(),
        "tail must not slice through a UTF-8 codepoint"
    );
    assert_eq!(result.unwrap(), "γδε");
}

#[tokio::test]
async fn insert_without_open_project_returns_no_project_before_motion_cli() {
    let _guard = MOTION_ENV_LOCK.lock().await;
    let _bin = EnvRestore::remove(ENV_MOTION_BIN);
    let _template_root = EnvRestore::remove(ENV_MOTION_TEMPLATE_ROOT);
    let _root = EnvRestore::remove(ENV_MOTION_ROOT);

    let err = motion_template_to_cut(&AppState::new(), json!({}), Actor::system())
        .await
        .unwrap_err();

    assert_eq!(err.code, error_codes::NO_PROJECT);
    assert_eq!(err.message, "no project is open");
}

#[tokio::test]
async fn dispatch_map_import_accepts_rendered_media_cut_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let package_dir = tmp.path().join("package");
    fs::create_dir_all(&package_dir).unwrap();
    let render_path = tmp.path().join("render").join("lower-third.mp4");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, true);
    add_motion_integration(&plan_path, 1);
    let mut rich_fallback: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    rich_fallback["unsupported"] = json!([{
        "layerId": "hologram-field",
        "feature": "layer.type:shader",
        "reason": "Target shellx-cut cannot lower shader layers to editable Cut operations."
    }]);
    rich_fallback["receipt"]["output"]["unsupportedCount"] = json!(1);
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&rich_fallback).unwrap(),
    )
    .unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({
            "path": plan_path,
            "packageDir": package_dir,
        }),
        Actor::system(),
    )
    .await;

    assert!(result.ok, "expected map_import success, got {result:?}");
    let body = result.result.expect("map result payload");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["schema"], json!("shellx-cut/motion-import-map@1"));
    assert_eq!(body["mode"], json!("rendered_media"));
    assert_eq!(body["packageId"], json!("pkg-lower-third"));
    assert_eq!(
        body["operations"][0]["verb"],
        json!("cut.media.import_rendered")
    );
    assert_eq!(body["renderedMedia"]["dryRun"], json!(true));
    assert_eq!(
        body["integration"]["schema"],
        json!("shellx-motion/integration-negotiation@1")
    );
    assert_eq!(body["integration"]["selectedProtocol"], json!(1));
    assert_eq!(
        body["warnings"],
        json!(["Target shellx-cut cannot lower shader layers to editable Cut operations."])
    );
}

#[tokio::test]
async fn dispatch_map_import_rejects_protocol_skew_before_operation_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.mp4");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, true);
    add_motion_integration(&plan_path, 2);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["operations"] = json!([{
        "verb": "cut.media.import_rendered",
        "renderedMedia": { "dryRun": true, "plannedPath": "../../outside" }
    }]);
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result.error.expect("protocol skew must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("protocol ranges do not overlap"));
    assert!(!error.message.contains("renderedMedia"));
}

#[tokio::test]
async fn dispatch_map_import_verifies_attested_rendered_media() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    assert!(result.ok, "expected verified map success: {result:?}");
    let body = result.result.expect("map result payload");
    assert_eq!(body["artifactHandles"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        body["artifactHandles"][0]["schema"],
        json!("shellx-motion/artifact-handle@1")
    );
    assert_eq!(
        body["planned"][0]["args"]["path"],
        json!(render_path.canonicalize().unwrap())
    );
    assert_eq!(body["lineageProofs"][0]["status"], json!("verified"));
    assert_eq!(
        body["lineageProofs"][0]["schema"],
        json!("shellx-cut/motion-import-attestation@1")
    );
    assert!(body["lineageProofs"][0]["connectorReceipt"].is_null());
    assert!(body["lineageProofs"][0]["cutPlanReceipt"]["id"].is_string());
}

#[tokio::test]
async fn dispatch_map_import_accepts_warned_cut_plan_receipt() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("warned.png");
    let plan_path = tmp.path().join("warned-cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let warning = "Target shellx-cut cannot lower shader layers to editable Cut operations.";
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["unsupported"] = json!([{
        "layerId": "hologram-field",
        "feature": "layer.type:shader",
        "reason": warning,
    }]);
    plan["receipt"]["status"] = json!("warning");
    plan["receipt"]["warnings"] = json!([warning]);
    plan["receipt"]["output"]["unsupportedCount"] = json!(1);
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    assert!(
        result.ok,
        "warned Motion receipt remains successful: {result:?}"
    );
    assert_eq!(result.result.unwrap()["warnings"], json!([warning]));
}

#[tokio::test]
async fn dispatch_map_import_resolves_current_sdk_nested_handoff_root() {
    let tmp = tempfile::tempdir().unwrap();
    let artifact_root = tmp.path().join("sdk-artifact-root");
    let render_path = artifact_root.join("lineaged.png");
    let plan_path = artifact_root
        .join(".shellx-motion")
        .join("cut")
        .join("operation.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    assert!(
        result.ok,
        "current SDK nested handoff should resolve from artifactRoot: {result:?}"
    );
    let body = result.result.unwrap();
    assert_eq!(body["lineageProofs"][0]["status"], json!("verified"));
    assert_eq!(
        body["planned"][0]["args"]["path"],
        json!(render_path.canonicalize().unwrap())
    );
}

#[tokio::test]
async fn dispatch_map_import_preserves_legacy_connector_compatibility_without_claiming_lineage() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("legacy.png");
    let plan_path = tmp.path().join("legacy-cut-import-plan.json");
    write_legacy_rendered_media_plan(&plan_path, &render_path);

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    assert!(
        result.ok,
        "legacy connector plan should remain accepted: {result:?}"
    );
    let body = result.result.unwrap();
    assert_eq!(
        body["lineageProofs"][0]["status"],
        json!("legacy-unverified")
    );
    assert!(body["lineageProofs"][0]["packageLineage"].is_null());
    assert!(body["lineageProofs"][0]["connectorReceipt"]["id"].is_string());
    assert!(body["lineageProofs"][0]["cutPlanReceipt"].is_null());
}

#[tokio::test]
async fn dispatch_map_import_verifies_gltf_five_hash_lineage() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("gltf.png");
    let plan_path = tmp.path().join("gltf-cut-import-plan.json");
    write_gltf_rendered_media_plan(&plan_path, &render_path);

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    assert!(
        result.ok,
        "five-hash glTF lineage should verify: {result:?}"
    );
    let proof = &result.result.unwrap()["lineageProofs"][0];
    assert_eq!(proof["status"], json!("verified"));
    assert_eq!(proof["packageLineage"]["adapterId"], json!("adapter.gltf"));
    for field in [
        "manifestSha256",
        "motionSha256",
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ] {
        assert_eq!(
            proof["packageLineage"][field].as_str().map(str::len),
            Some(64),
            "missing {field}"
        );
    }
}

#[tokio::test]
async fn dispatch_map_import_rejects_each_changed_gltf_lineage_hash() {
    for field in [
        "manifestSha256",
        "motionSha256",
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let render_path = tmp.path().join("render").join(format!("{field}.png"));
        let plan_path = tmp.path().join(format!("{field}.cut-import-plan.json"));
        write_gltf_rendered_media_plan(&plan_path, &render_path);
        let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan["operations"][0]["renderedMedia"]["handle"]["packageLineage"][field] =
            json!("f".repeat(64));
        fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

        let result = crate::dispatch::dispatch(
            &AppState::new(),
            "motion.map_import",
            json!({"path": plan_path}),
            Actor::system(),
        )
        .await;
        assert!(!result.ok, "changed {field} must fail closed");
        assert!(result
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("lineage does not match")));
    }
}

#[tokio::test]
async fn dispatch_map_import_rejects_partial_gltf_lineage_and_stale_handle_identity() {
    let partial = tempfile::tempdir().unwrap();
    let partial_render = partial.path().join("render").join("partial.png");
    let partial_plan = partial.path().join("partial.cut-import-plan.json");
    write_gltf_rendered_media_plan(&partial_plan, &partial_render);
    let mut plan: Value = serde_json::from_slice(&fs::read(&partial_plan).unwrap()).unwrap();
    plan["operations"][0]["renderedMedia"]["handle"]["packageLineage"]
        .as_object_mut()
        .unwrap()
        .remove("loweringReceiptSha256");
    fs::write(&partial_plan, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let partial_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":partial_plan}),
        Actor::system(),
    )
    .await;
    assert!(!partial_result.ok, "partial glTF lineage must fail closed");

    let stale = tempfile::tempdir().unwrap();
    let stale_render = stale.path().join("render").join("stale.png");
    let stale_plan = stale.path().join("stale.cut-import-plan.json");
    write_rendered_media_plan(&stale_plan, &stale_render, false);
    rewrite_handle_descriptor(&stale_plan, |handle| {
        handle["id"] = json!("artifact-000000000000000000000000");
    });
    let mut plan: Value = serde_json::from_slice(&fs::read(&stale_plan).unwrap()).unwrap();
    plan["operations"][0]["renderedMedia"]["handle"]["id"] =
        json!("artifact-000000000000000000000000");
    fs::write(&stale_plan, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let stale_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":stale_plan}),
        Actor::system(),
    )
    .await;
    assert!(!stale_result.ok, "stale handle identity must fail closed");
    assert!(stale_result
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("does not bind its identity")));
}

#[tokio::test]
async fn dispatch_map_import_rejects_tampered_cut_receipt_and_operation_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("receipt.png");
    let plan_path = tmp.path().join("receipt.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["receipt"]["inputHashes"]["manifestSha256"] = json!("f".repeat(64));
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let receipt_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path}),
        Actor::system(),
    )
    .await;
    assert!(!receipt_result.ok, "tampered Cut receipt must fail closed");
    assert!(receipt_result.error.as_ref().is_some_and(|error| error
        .message
        .contains("does not bind the artifact and package lineage")));

    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("source.png");
    let plan_path = tmp.path().join("source.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["operations"][0]["source"]["packageId"] = json!("pkg-other");
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let source_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path}),
        Actor::system(),
    )
    .await;
    assert!(
        !source_result.ok,
        "source identity mismatch must fail closed"
    );

    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("media.png");
    let plan_path = tmp.path().join("media.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["operations"][0]["media"]["fps"] = json!(0);
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let media_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path}),
        Actor::system(),
    )
    .await;
    assert!(!media_result.ok, "invalid media metadata must fail closed");

    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("extra.png");
    let plan_path = tmp.path().join("extra.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["operations"][0]["renderedMedia"]["unexpected"] = json!(true);
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let extra_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path":plan_path}),
        Actor::system(),
    )
    .await;
    assert!(
        !extra_result.ok,
        "extra renderedMedia fields must fail closed"
    );
}

#[tokio::test]
async fn dispatch_map_import_rejects_tampered_render_receipt_commitment() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("render-receipt.png");
    let plan_path = tmp.path().join("render-receipt.cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    rewrite_render_receipt(&plan_path, |receipt| {
        receipt["inputHashes"]["operationHash"] = json!("f".repeat(64));
    });

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    assert!(!result.ok, "tampered render receipt must fail closed");
    assert!(result
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("render receipt inputHashes")));
}

#[tokio::test]
async fn dispatch_map_import_verifies_and_rejects_placement_commitments() {
    let valid = tempfile::tempdir().unwrap();
    let valid_render = valid.path().join("render").join("placement.png");
    let valid_plan = valid.path().join("placement.cut-import-plan.json");
    write_rendered_media_plan(&valid_plan, &valid_render, false);
    add_plan_placement(&valid_plan);
    let accepted = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": valid_plan}),
        Actor::system(),
    )
    .await;
    assert!(
        accepted.ok,
        "exact placement commitment should verify: {accepted:?}"
    );

    let mismatch = tempfile::tempdir().unwrap();
    let mismatch_render = mismatch.path().join("render").join("placement.png");
    let mismatch_plan = mismatch.path().join("placement.cut-import-plan.json");
    write_rendered_media_plan(&mismatch_plan, &mismatch_render, false);
    add_plan_placement(&mismatch_plan);
    let mut plan: Value = serde_json::from_slice(&fs::read(&mismatch_plan).unwrap()).unwrap();
    plan["receipt"]["output"]["placement"]["track"] = json!("v2");
    let changed = plan["receipt"]["output"]["placement"].clone();
    plan["receipt"]["inputHashes"]["placement"] = json!(placement_hash(&changed));
    fs::write(&mismatch_plan, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let mismatched = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": mismatch_plan}),
        Actor::system(),
    )
    .await;
    assert!(
        !mismatched.ok,
        "placement/operation mismatch must fail closed"
    );
    assert!(mismatched.error.as_ref().is_some_and(|error| error
        .message
        .contains("placement does not match its operation")));

    let stale = tempfile::tempdir().unwrap();
    let stale_render = stale.path().join("render").join("placement.png");
    let stale_plan = stale.path().join("placement.cut-import-plan.json");
    write_rendered_media_plan(&stale_plan, &stale_render, false);
    add_plan_placement(&stale_plan);
    let mut plan: Value = serde_json::from_slice(&fs::read(&stale_plan).unwrap()).unwrap();
    plan["receipt"]["inputHashes"]["placement"] = json!("f".repeat(64));
    fs::write(&stale_plan, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    let stale_result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": stale_plan}),
        Actor::system(),
    )
    .await;
    assert!(!stale_result.ok, "stale placement hash must fail closed");
    assert!(stale_result
        .error
        .as_ref()
        .is_some_and(|error| error.message.contains("placement hash mismatch")));
}

#[tokio::test]
async fn dispatch_map_import_rejects_artifact_bytes_swapped_after_attestation() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    fs::write(&render_path, b"swapped after attestation").unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result.error.expect("swapped artifact must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("does not match its handle"));
}

#[tokio::test]
async fn dispatch_map_import_rejects_oversized_plan_before_reading_json() {
    let tmp = tempfile::tempdir().unwrap();
    let plan_path = tmp.path().join("cut-import-plan.json");
    let file = fs::File::create(&plan_path).unwrap();
    file.set_len(MOTION_IMPORT_PLAN_MAX_BYTES + 1).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result.error.expect("oversized plan must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("bounded regular file"));
}

#[tokio::test]
async fn dispatch_map_import_rejects_descriptor_hash_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let descriptor_path = tmp.path().join("artifacts/lower-third.artifact.json");
    let mut bytes = fs::read(&descriptor_path).unwrap();
    bytes.push(b'\n');
    fs::write(descriptor_path, bytes).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result.error.expect("modified descriptor must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("descriptor hash mismatch"));
}

#[tokio::test]
async fn dispatch_map_import_rejects_failed_receipt_attestation() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    rewrite_handle_descriptor(&plan_path, |handle| {
        handle["receipts"][0]["status"] = json!("failed");
    });

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result.error.expect("failed receipt must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("receipt is not successful"));
}

#[cfg(unix)]
#[tokio::test]
async fn dispatch_map_import_rejects_descriptor_symlink_escape() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let descriptor_path = tmp.path().join("artifacts/lower-third.artifact.json");
    let outside_descriptor = outside.path().join("artifact.json");
    fs::copy(&descriptor_path, &outside_descriptor).unwrap();
    fs::remove_file(&descriptor_path).unwrap();
    symlink(&outside_descriptor, &descriptor_path).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result
        .error
        .expect("descriptor symlink escape must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("escapes the Motion handoff root"));
}

#[cfg(unix)]
#[tokio::test]
async fn dispatch_map_import_rejects_artifact_symlink_escape() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    let outside_render = outside.path().join("lower-third.png");
    fs::copy(&render_path, &outside_render).unwrap();
    fs::remove_file(&render_path).unwrap();
    symlink(&outside_render, &render_path).unwrap();

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({"path": plan_path}),
        Actor::system(),
    )
    .await;

    let error = result
        .error
        .expect("artifact symlink escape must be rejected");
    assert!(!result.ok);
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains("escapes the Motion handoff root"));
}

#[tokio::test]
async fn dispatch_apply_import_dry_run_is_non_mutating_without_project() {
    let tmp = tempfile::tempdir().unwrap();
    let render_path = tmp.path().join("render").join("lower-third.mp4");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, true);

    let result = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.apply_import",
        json!({
            "path": plan_path,
            "dryRun": true,
        }),
        Actor::system(),
    )
    .await;

    assert!(result.ok, "expected dry-run apply success, got {result:?}");
    assert!(result.op_ids.as_ref().is_none_or(Vec::is_empty));
    let body = result.result.expect("apply result payload");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["schema"], json!("shellx-cut/motion-import-apply@1"));
    assert_eq!(body["dryRun"], json!(true));
    assert_eq!(body["wouldMutate"], json!(false));
    assert_eq!(body["planned"][0]["verb"], json!("media.import"));
    assert_eq!(body["planned"][1]["verb"], json!("edit.insert"));
}

#[tokio::test]
async fn editable_video_reuses_cut_assets_and_reimports_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-editable-video.cutproj");
    let plan_path = tmp.path().join("editable-video-plan.json");
    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({ "name": "motion-editable-video", "dir": project_dir }),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("first-edit-sample.mp4");
    let (first_asset, second_asset) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let make_asset = |hash: &str| Asset {
            path: source_path.display().to_string(),
            hash: hash.into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        };
        let first = store
            .record_import(None, make_asset("sha256:first"), Actor::system(), None)
            .unwrap()
            .0;
        let second = store
            .record_import(None, make_asset("sha256:second"), Actor::system(), None)
            .unwrap()
            .0;
        (first, second)
    };
    write_editable_video_plan(&plan_path, &first_asset);
    add_motion_integration(&plan_path, 1);

    let applied = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(applied.ok, "editable video apply failed: {applied:?}");
    let body = applied.result.unwrap();
    let binding = &body["bindings"][0];
    assert_eq!(binding["assetId"], json!(first_asset));
    assert_eq!(binding["cutVerb"], json!("edit.insert"));
    let clip_id = binding["clipId"].as_str().unwrap().to_string();

    let mut changed: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    changed["operations"][0]["payload"]["source"] = json!(format!("cut-asset:{second_asset}"));
    fs::write(&plan_path, serde_json::to_vec_pretty(&changed).unwrap()).unwrap();
    let reapplied = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(
        reapplied.ok,
        "editable video reimport failed: {reapplied:?}"
    );
    let rebound = &reapplied.result.as_ref().unwrap()["bindings"][0];
    assert_eq!(rebound["clipId"], json!(clip_id));
    assert_eq!(rebound["assetId"], json!(second_asset));
    let current_asset = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&clip_id).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("Motion video must remain a media clip"),
        }
    };
    assert_eq!(current_asset, second_asset);
    let undo = crate::dispatch::dispatch(&state, "project.undo", json!({}), Actor::system()).await;
    assert!(undo.ok, "editable video reimport undo failed: {undo:?}");
    let restored_asset = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&clip_id).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("Motion video must remain a media clip after undo"),
        }
    };
    assert_eq!(restored_asset, first_asset);
}

#[tokio::test]
async fn editable_audio_reuses_a_cut_asset_on_the_native_audio_track() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-editable-audio.cutproj");
    let plan_path = tmp.path().join("editable-audio-plan.json");
    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({ "name": "motion-editable-audio", "dir": project_dir }),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("first-edit-sample.mp4");
    let asset_id = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .record_import(
                None,
                Asset {
                    path: source_path.display().to_string(),
                    hash: "sha256:audio".into(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                Actor::system(),
                None,
            )
            .unwrap()
            .0
    };
    write_editable_audio_plan(&plan_path, &asset_id);
    add_motion_integration(&plan_path, 1);
    let applied = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(applied.ok, "editable audio apply failed: {applied:?}");
    let body = applied.result.unwrap();
    let binding = &body["bindings"][0];
    assert_eq!(binding["assetId"], json!(asset_id));
    assert_eq!(binding["trackId"], json!("a1t"));
    let clip_id = binding["clipId"].as_str().unwrap();
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let (track_id, _) = store.project.find_clip(clip_id).unwrap();
    assert_eq!(
        store.project.track(track_id).unwrap().kind,
        cut_core::TrackKind::Audio
    );
}

#[tokio::test]
async fn editable_import_maps_and_applies_native_text_shape_with_stable_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-editable.cutproj");
    let plan_path = tmp.path().join("editable-cut-import-plan.json");
    write_editable_plan(&plan_path);
    add_motion_integration(&plan_path, 1);

    let mapped = crate::dispatch::dispatch(
        &AppState::new(),
        "motion.map_import",
        json!({ "path": plan_path }),
        Actor::system(),
    )
    .await;
    assert!(mapped.ok, "editable map failed: {mapped:?}");
    let mapped_body = mapped.result.unwrap();
    assert_eq!(mapped_body["mode"], json!("editable_lowering"));
    let planned = mapped_body["planned"].as_array().unwrap();
    assert!(planned
        .iter()
        .any(|step| step["verb"] == json!("edit.add_shape")));
    assert!(planned
        .iter()
        .any(|step| step["verb"] == json!("title.add")));
    assert_eq!(
        planned
            .iter()
            .filter(|step| step["verb"] == json!("edit.keyframe"))
            .count(),
        4
    );
    assert!(mapped_body["renderedMedia"].is_null());

    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({ "name": "motion-editable", "dir": project_dir }),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");
    let format = crate::dispatch::dispatch(
        &state,
        "project.format",
        json!({ "width": 320, "height": 180, "fps": 10 }),
        Actor::system(),
    )
    .await;
    assert!(format.ok, "project format failed: {format:?}");

    let applied = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(applied.ok, "editable apply failed: {applied:?}");
    let body = applied.result.unwrap();
    assert_eq!(body["mode"], json!("editable_lowering"));
    assert_eq!(body["bindings"].as_array().map(Vec::len), Some(2));
    assert_eq!(body["assets"].as_array().map(Vec::len), Some(2));
    let title_clip = body["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|binding| binding["sourceLayerId"] == json!("title"))
        .and_then(|binding| binding["clipId"].as_str())
        .unwrap()
        .to_string();
    let panel_clip = body["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|binding| binding["sourceLayerId"] == json!("panel"))
        .and_then(|binding| binding["clipId"].as_str())
        .unwrap()
        .to_string();
    let first_clip_count = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .project
            .all_sequence_tracks()
            .flat_map(|track| track.clips.iter())
            .filter(|clip| clip.id().is_some())
            .count()
    };
    assert_eq!(first_clip_count, 2);
    let initial_opacity_tracks = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.keyframes.clone(),
            _ => panic!("imported Motion title must be media-backed"),
        }
    };
    assert_eq!(initial_opacity_tracks.len(), 3);
    assert!(initial_opacity_tracks
        .iter()
        .any(|track| track.param == cut_core::KfParam::Opacity));
    let pos_x = initial_opacity_tracks
        .iter()
        .find(|track| track.param == cut_core::KfParam::PosX)
        .unwrap();
    let pos_y = initial_opacity_tracks
        .iter()
        .find(|track| track.param == cut_core::KfParam::PosY)
        .unwrap();
    assert_eq!(pos_x.interp, cut_core::KfInterp::EaseOutQuad);
    assert_eq!(pos_x.points[0].value, -0.1);
    assert_eq!(pos_x.points[1].value, 320.0 / 1920.0);
    assert_eq!(pos_y.points[0].value, 0.9);
    assert_eq!(pos_y.points[1].value, 0.75);
    let initial_fade_tracks = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&panel_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.keyframes.clone(),
            _ => panic!("imported Motion shape must be media-backed"),
        }
    };
    assert_eq!(initial_fade_tracks.len(), 1);
    assert_eq!(initial_fade_tracks[0].param, cut_core::KfParam::Opacity);
    assert_eq!(initial_fade_tracks[0].points.len(), 4);

    let repeated = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(
        repeated.ok,
        "editable idempotent retry failed: {repeated:?}"
    );
    assert_eq!(
        repeated.result.as_ref().unwrap()["status"],
        json!("already_applied")
    );
    assert_eq!(
        repeated.result.as_ref().unwrap()["bindings"],
        body["bindings"]
    );

    let undone =
        crate::dispatch::dispatch(&state, "project.undo", json!({}), Actor::system()).await;
    assert!(undone.ok, "grouped editable undo failed: {undone:?}");
    let after_undo = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .project
            .all_sequence_tracks()
            .flat_map(|track| track.clips.iter())
            .filter(|clip| clip.id().is_some())
            .count()
    };
    assert_eq!(
        after_undo, 0,
        "one undo must remove the complete native import group"
    );
    let redone =
        crate::dispatch::dispatch(&state, "project.redo", json!({}), Actor::system()).await;
    assert!(redone.ok, "grouped editable redo failed: {redone:?}");
    let after_redo = {
        let guard = state.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .project
            .all_sequence_tracks()
            .flat_map(|track| track.clips.iter())
            .filter(|clip| clip.id().is_some())
            .count()
    };
    assert_eq!(
        after_redo, 2,
        "one redo must restore the complete native import group"
    );

    let initial_title_asset = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("imported Motion title must be a media-backed native title clip"),
        }
    };
    let mut changed_plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    changed_plan["operations"][0]["payload"]["fill"] = json!("#334455");
    changed_plan["operations"][1]["payload"]["text"] = json!("Reimported in place");
    changed_plan["operations"][1]["payload"]
        .as_object_mut()
        .unwrap()
        .remove("keyframes");
    changed_plan["operations"][1]["payload"]
        .as_object_mut()
        .unwrap()
        .remove("opacity");
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&changed_plan).unwrap(),
    )
    .unwrap();

    let reimported = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(reimported.ok, "editable reimport failed: {reimported:?}");
    let reimported_body = reimported.result.unwrap();
    assert_eq!(reimported_body["reimported"], json!(true));
    assert_eq!(reimported_body["alreadyApplied"], json!(false));
    assert_eq!(
        reimported_body["bindings"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        reimported_body["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|binding| binding["clipId"].clone())
            .collect::<Vec<_>>(),
        body["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|binding| binding["clipId"].clone())
            .collect::<Vec<_>>()
    );
    let reimported_title_asset = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("reimported Motion title must stay media-backed"),
        }
    };
    assert_ne!(reimported_title_asset, initial_title_asset);
    let opacity_tracks_after_reimport = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.keyframes.clone(),
            _ => panic!("reimported Motion title must stay media-backed"),
        }
    };
    assert!(opacity_tracks_after_reimport.is_empty());
    let fade_tracks_after_reimport = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&panel_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.keyframes.clone(),
            _ => panic!("reimported Motion shape must stay media-backed"),
        }
    };
    assert_eq!(fade_tracks_after_reimport, initial_fade_tracks);

    let undo_reimport =
        crate::dispatch::dispatch(&state, "project.undo", json!({}), Actor::system()).await;
    assert!(
        undo_reimport.ok,
        "grouped reimport undo failed: {undo_reimport:?}"
    );
    let asset_after_reimport_undo = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("Motion title must remain after reimport undo"),
        }
    };
    assert_eq!(asset_after_reimport_undo, initial_title_asset);
    let opacity_tracks_after_reimport_undo = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.keyframes.clone(),
            _ => panic!("Motion title must remain after reimport undo"),
        }
    };
    assert_eq!(opacity_tracks_after_reimport_undo, initial_opacity_tracks);
    let redo_reimport =
        crate::dispatch::dispatch(&state, "project.redo", json!({}), Actor::system()).await;
    assert!(
        redo_reimport.ok,
        "grouped reimport redo failed: {redo_reimport:?}"
    );
    let asset_after_reimport_redo = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("Motion title must remain after reimport redo"),
        }
    };
    assert_eq!(asset_after_reimport_redo, reimported_title_asset);
    let opacity_tracks_after_reimport_redo = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.keyframes.clone(),
            _ => panic!("Motion title must remain after reimport redo"),
        }
    };
    assert!(opacity_tracks_after_reimport_redo.is_empty());

    let repeated_reimport = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(
        repeated_reimport.ok,
        "reimport retry failed: {repeated_reimport:?}"
    );
    assert_eq!(
        repeated_reimport.result.as_ref().unwrap()["status"],
        json!("already_applied")
    );

    let mut retimed_plan = changed_plan.clone();
    retimed_plan["operations"][1]["startMs"] = json!(10);
    retimed_plan["operations"][1]["durationMs"] = json!(190);
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&retimed_plan).unwrap(),
    )
    .unwrap();
    let retimed = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(!retimed.ok, "retimed native reimport must fail closed");
    assert!(
        retimed
            .error
            .as_ref()
            .is_some_and(|error| error.message.contains("changes layer timing")),
        "retimed reimport returned the wrong error: {retimed:?}"
    );
    let asset_after_retime_refusal = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&title_clip).unwrap();
        match &store.project.track(track_id).unwrap().clips[index] {
            cut_core::Clip::Media(clip) => clip.asset.clone(),
            _ => panic!("retime refusal must preserve the Motion title"),
        }
    };
    assert_eq!(asset_after_retime_refusal, reimported_title_asset);

    let undo_active_reimport =
        crate::dispatch::dispatch(&state, "project.undo", json!({}), Actor::system()).await;
    assert!(
        undo_active_reimport.ok,
        "active reimport undo failed: {undo_active_reimport:?}"
    );
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&changed_plan).unwrap(),
    )
    .unwrap();
    let reapplied_after_undo = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({ "path": plan_path, "dryRun": false }),
        Actor::system(),
    )
    .await;
    assert!(
        reapplied_after_undo.ok,
        "undone reimport was mistaken for active idempotency: {reapplied_after_undo:?}"
    );
    assert_eq!(
        reapplied_after_undo.result.as_ref().unwrap()["reimported"],
        json!(true)
    );
    assert_eq!(
        reapplied_after_undo.result.as_ref().unwrap()["alreadyApplied"],
        json!(false)
    );

    let updated = crate::dispatch::dispatch(
        &state,
        "title.update",
        json!({ "clip": title_clip, "text": "Still editable in Cut" }),
        Actor::system(),
    )
    .await;
    assert!(updated.ok, "native title update failed: {updated:?}");
}

#[tokio::test]
async fn dispatch_apply_import_real_apply_imports_and_inserts_rendered_media() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-real.cutproj");
    let render_dir = tmp.path().join("render");
    fs::create_dir_all(&render_dir).unwrap();
    let render_path = render_dir.join("lower-third.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);

    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({
            "name": "motion-real",
            "dir": project_dir,
        }),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");

    let result = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({
            "path": plan_path,
            "dryRun": false,
        }),
        Actor::system(),
    )
    .await;

    assert!(result.ok, "expected real apply success, got {result:?}");
    assert!(result.op_ids.as_ref().is_some_and(|ids| ids.len() == 1));
    let body = result.result.expect("apply result payload");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["schema"], json!("shellx-cut/motion-import-apply@1"));
    assert_eq!(body["dryRun"], json!(false));
    assert_eq!(body["assets"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["clips"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["lineageProofs"][0]["status"], json!("verified"));
    assert_eq!(body["rollback"]["committedOps"], json!(1));
    assert!(body["restore_hint"]
        .as_str()
        .unwrap_or("")
        .contains("project.undo"));
    let clip_id = body["clips"][0].as_str().unwrap().to_string();
    let projected =
        crate::dispatch::dispatch(&state, "project.state", json!({}), Actor::system()).await;
    assert!(projected.ok, "project state failed: {projected:?}");
    let clip = projected.result.as_ref().unwrap()["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().into_iter().flatten())
        .find(|clip| clip["id"] == json!(clip_id))
        .expect("rendered Motion clip must stay visible in project.state");
    assert_eq!(
        clip["motion_link"]["schema"],
        json!("shellx-cut/motion-link@1")
    );
    assert_eq!(clip["motion_link"]["clipId"], json!(clip_id));
    assert_eq!(clip["motion_link"]["state"], json!("missing-source"));
    assert_eq!(clip["motion_link"]["availability"]["source"], json!(false));
    assert_eq!(clip["motion_link"]["availability"]["render"], json!(true));
    assert_eq!(clip["motion_link"]["packageId"], body["packageId"]);
    assert_eq!(clip["motion_link"]["motionId"], body["motionId"]);
    assert_eq!(
        clip["motion_link"]["render"]["path"],
        json!(render_path.canonicalize().unwrap())
    );
    assert_eq!(
        clip["motion_link"]["originAttestation"],
        body["lineageProofs"][0]
    );

    let repeated = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({
            "path": plan_path,
            "dryRun": false,
        }),
        Actor::system(),
    )
    .await;
    assert!(repeated.ok, "idempotent retry failed: {repeated:?}");
    assert_eq!(repeated.op_ids, result.op_ids);
    let repeated_body = repeated.result.unwrap();
    assert_eq!(repeated_body["status"], json!("already_applied"));
    assert_eq!(repeated_body["alreadyApplied"], json!(true));
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.project.assets.len(), 1);
    assert_eq!(
        store
            .project
            .track("v1")
            .unwrap()
            .clips
            .iter()
            .filter(|clip| clip.id().is_some())
            .count(),
        1
    );
    assert_eq!(
        store
            .log
            .read_all()
            .unwrap()
            .iter()
            .filter(|op| op.verb == "motion.apply_import")
            .count(),
        1
    );
    drop(guard);

    let undo = crate::dispatch::dispatch(&state, "project.undo", json!({}), Actor::system()).await;
    assert!(undo.ok, "Motion import undo failed: {undo:?}");
    let redo = crate::dispatch::dispatch(&state, "project.redo", json!({}), Actor::system()).await;
    assert!(redo.ok, "Motion import redo failed: {redo:?}");
    let mut close =
        crate::dispatch::dispatch(&state, "project.close", json!({}), Actor::system()).await;
    for _ in 0..60 {
        if close.ok
            || close.error.as_ref().map(|error| error.code.as_str()) != Some("job_cancel_pending")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        close =
            crate::dispatch::dispatch(&state, "project.close", json!({}), Actor::system()).await;
    }
    assert!(close.ok, "project close failed: {close:?}");
    let reopen = crate::dispatch::dispatch(
        &state,
        "project.open",
        json!({"path": project_dir}),
        Actor::system(),
    )
    .await;
    assert!(reopen.ok, "project reopen failed: {reopen:?}");
    let reopened =
        crate::dispatch::dispatch(&state, "project.state", json!({}), Actor::system()).await;
    let reopened_clip = reopened.result.unwrap()["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().into_iter().flatten())
        .find(|clip| clip["id"] == json!(clip_id))
        .cloned()
        .expect("redo/reopen must restore the Motion clip");
    assert_eq!(
        reopened_clip["motion_link"]["originAttestation"],
        body["lineageProofs"][0]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn linked_motion_refresh_and_relink_preserve_clip_identity() {
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
    let _canvas_bin = EnvRestore::set(ENV_CANVAS_BIN, &fake_canvas);

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
    assert!(editing.ok, "Canvas Motion launch failed: {editing:?}");
    let launch_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < launch_deadline {
        if canvas_args.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let launched_args = fs::read_to_string(&canvas_args)
        .expect("fake Canvas did not record launch arguments within 5s");
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
}

#[tokio::test]
async fn dispatch_apply_import_background_cancel_commits_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-cancel.cutproj");
    let render_dir = tmp.path().join("render");
    fs::create_dir_all(&render_dir).unwrap();
    let render_path = render_dir.join("cancel.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);

    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"motion-cancel", "dir":project_dir}),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");
    let (before_project, before_ops, before_cache, before_log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (
            store.project.clone(),
            store.log.read_all().unwrap(),
            fs::read(store.dir.join("project.json")).unwrap(),
            fs::read(store.dir.join("ops.jsonl")).unwrap(),
        )
    };

    let queued = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({"path":plan_path, "dryRun":false, "background":true}),
        Actor::system(),
    )
    .await;
    assert!(queued.ok, "background apply was not queued: {queued:?}");
    let job_id = queued.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let cancelled = crate::dispatch::dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id":job_id}),
        Actor::system(),
    )
    .await;
    assert!(
        cancelled.ok,
        "background apply was not cancellable: {cancelled:?}"
    );
    let record = state.jobs.get(&job_id).unwrap();
    assert_eq!(record.state, crate::jobs::JobState::Failed);
    assert_eq!(
        record.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancelled")
    );

    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.project, before_project);
    assert_eq!(store.log.read_all().unwrap(), before_ops);
    assert_eq!(
        fs::read(store.dir.join("project.json")).unwrap(),
        before_cache
    );
    assert_eq!(fs::read(store.dir.join("ops.jsonl")).unwrap(), before_log);
}

#[tokio::test]
async fn dispatch_apply_import_insert_failure_is_atomic() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-partial.cutproj");
    let render_dir = tmp.path().join("render");
    fs::create_dir_all(&render_dir).unwrap();
    let render_a = render_dir.join("a.png");
    let render_b = render_dir.join("b.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    let rendered_a = write_legacy_attested_rendered_media(
        &plan_path,
        &render_a,
        "pkg-partial",
        "motion-partial",
        "a",
    );
    let rendered_b = write_legacy_attested_rendered_media(
        &plan_path,
        &render_b,
        "pkg-partial",
        "motion-partial",
        "b",
    );
    fs::write(
            &plan_path,
            serde_json::to_vec_pretty(&json!({
                "schema": "shellx-motion/cut-import-plan@1",
                "ok": true,
                "packageId": "pkg-partial",
                "motionId": "motion-partial",
                "targetId": "shellx-cut",
                "mode": "rendered_media",
                "operations": [
                    {
                        "verb": "cut.media.import_rendered",
                        "source": {"packageId":"pkg-partial","motionId":"motion-partial","render":"artifact"},
                        "startMs": 0,
                        "durationMs": 1000,
                        "media": {"width":1280,"height":720,"fps":30},
                        "renderedMedia": rendered_a
                    },
                    {
                        "verb": "cut.media.import_rendered",
                        "source": {"packageId":"pkg-partial","motionId":"motion-partial","render":"artifact"},
                        "track": "missing-track",
                        "startMs": 1000,
                        "durationMs": 1000,
                        "media": {"width":1280,"height":720,"fps":30},
                        "renderedMedia": rendered_b
                    }
                ],
                "unsupported": [],
                "receipt": {
                    "schema": "shellx-motion/receipt@1",
                    "id": "partial",
                    "operation": "cut.import.plan",
                    "status": "passed",
                    "output": {"operationCount": 2, "unsupportedCount": 0},
                    "warnings": []
                }
            }))
            .unwrap(),
        )
        .unwrap();

    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({
            "name": "motion-partial",
            "dir": project_dir,
        }),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");
    let (before_project, before_ops, before_cache, before_log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (
            store.project.clone(),
            store.log.read_all().unwrap(),
            fs::read(store.dir.join("project.json")).unwrap(),
            fs::read(store.dir.join("ops.jsonl")).unwrap(),
        )
    };

    let result = crate::dispatch::dispatch(
        &state,
        "motion.apply_import",
        json!({
            "path": plan_path,
            "dryRun": false,
        }),
        Actor::system(),
    )
    .await;

    assert!(!result.ok, "invalid plan should fail: {result:?}");
    assert!(result.op_ids.as_ref().is_none_or(Vec::is_empty));
    assert_eq!(
        result.error.as_ref().map(|error| error.code.as_str()),
        Some("not_found")
    );
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert_eq!(store.project, before_project);
    assert_eq!(store.log.read_all().unwrap(), before_ops);
    assert_eq!(
        fs::read(store.dir.join("project.json")).unwrap(),
        before_cache
    );
    assert_eq!(fs::read(store.dir.join("ops.jsonl")).unwrap(), before_log);
}

#[tokio::test]
async fn template_insert_failure_keeps_atomic_import_uncommitted() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join("motion-template-partial.cutproj");
    let render_path = tmp.path().join("render").join("template.png");
    let plan_path = tmp.path().join("cut-import-plan.json");
    write_rendered_media_plan(&plan_path, &render_path, false);
    add_motion_integration(&plan_path, 1);
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    plan["operations"][0]["track"] = json!("missing-track");
    plan["operations"][0]["startMs"] = json!(0);
    plan["operations"][0]["durationMs"] = json!(1000);
    fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    add_plan_placement(&plan_path);

    let state = AppState::new();
    let create = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({
            "name": "motion-template-partial",
            "dir": project_dir,
        }),
        Actor::system(),
    )
    .await;
    assert!(create.ok, "project create failed: {create:?}");

    let result = apply_motion_template_insert(
        &state,
        MotionTemplateRequest {
            template: "editable-lower-third".to_string(),
            params: Map::new(),
            policy: MotionTemplatePolicy::Insert,
            out_dir: tmp.path().join("out"),
            at_ms: 0,
            track: "missing-track".to_string(),
            duration_ms: Some(1000),
            dry_run_render: false,
            checkpoint: true,
            rationale: None,
            motion_job_id: None,
            caller_scope: project_dir.clone(),
        },
        json!({
            "ok": true,
            "render": {"outputPath": render_path},
            "cutPlanPath": plan_path,
            "artifacts": [],
            "warnings": []
        }),
        Actor::system(),
    )
    .await
    .expect("template insert handler should return a structured result");

    assert!(!result.ok, "invalid atomic insert should fail: {result:?}");
    assert_eq!(
        result.error.as_ref().map(|error| error.code.as_str()),
        Some("not_found")
    );
    assert_eq!(result.op_ids.as_ref().map(Vec::len), Some(1));
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    assert!(store.project.assets.is_empty());
    assert!(store
        .project
        .track("v1")
        .is_some_and(|track| track.clips.is_empty()));
    assert_eq!(
        store
            .log
            .read_all()
            .unwrap()
            .iter()
            .map(|op| op.verb.as_str())
            .collect::<Vec<_>>(),
        vec!["project.create", "project.checkpoint"]
    );
}

fn write_rendered_media_plan(path: &Path, render_path: &Path, dry_run: bool) {
    write_rendered_media_plan_kind(path, render_path, dry_run, true);
}

fn write_legacy_rendered_media_plan(path: &Path, render_path: &Path) {
    write_rendered_media_plan_kind(path, render_path, false, false);
}

fn write_gltf_rendered_media_plan(path: &Path, render_path: &Path) {
    write_rendered_media_plan(path, render_path, false);
    let rendered_media = write_gltf_attested_rendered_media(
        path,
        render_path,
        "pkg-lower-third",
        "motion-lower-third",
        "lower-third",
    );
    let mut plan: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    plan["operations"][0]["renderedMedia"] = rendered_media.clone();
    plan["receipt"] =
        lineaged_cut_plan_receipt("pkg-lower-third", "motion-lower-third", &rendered_media, 1);
    fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn write_rendered_media_plan_kind(path: &Path, render_path: &Path, dry_run: bool, lineaged: bool) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    if let Some(parent) = render_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let rendered_media = if dry_run {
        json!({
            "plannedPath": render_path,
            "receiptPath": render_path.with_extension("receipt.json"),
            "dryRun": true
        })
    } else if lineaged {
        write_attested_rendered_media(
            path,
            render_path,
            "pkg-lower-third",
            "motion-lower-third",
            "lower-third",
        )
    } else {
        write_legacy_attested_rendered_media(
            path,
            render_path,
            "pkg-lower-third",
            "motion-lower-third",
            "lower-third",
        )
    };
    let receipt = if !dry_run && lineaged {
        lineaged_cut_plan_receipt("pkg-lower-third", "motion-lower-third", &rendered_media, 1)
    } else {
        json!({
            "schema": "shellx-motion/receipt@1",
            "id": "cut-import-rendered-media",
            "operation": "cut.import.plan",
            "status": "passed",
            "packageId": "pkg-lower-third",
            "inputHashes": {
                "motion": sha256_hex(b"motion:pkg-lower-third"),
                "targetCapabilities": sha256_hex(b"target:shellx-cut")
            },
            "createdAt": "2026-07-01T00:00:00.000Z",
            "lane": "cut",
            "output": {
                "mode": "rendered_media",
                "targetId": "shellx-cut",
                "operationCount": 1,
                "unsupportedCount": 0
            },
            "warnings": []
        })
    };
    let plan = json!({
        "schema": "shellx-motion/cut-import-plan@1",
        "ok": true,
        "packageId": "pkg-lower-third",
        "motionId": "motion-lower-third",
        "targetId": "shellx-cut",
        "mode": "rendered_media",
        "operations": [
            {
                "verb": "cut.media.import_rendered",
                "source": {
                    "packageId": "pkg-lower-third",
                    "motionId": "motion-lower-third",
                    "render": if dry_run { "dry_run" } else { "artifact" }
                },
                "startMs": 0,
                "durationMs": 1500,
                "media": {
                    "width": 1280,
                    "height": 720,
                    "fps": 30
                },
                "renderedMedia": rendered_media
            }
        ],
        "unsupported": [],
        "document": {
            "width": 1280,
            "height": 720,
            "fps": 30,
            "durationMs": 1500
        },
        "receipt": receipt
    });
    fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    add_motion_integration(path, 1);
}

fn lineaged_cut_plan_receipt(
    package_id: &str,
    motion_id: &str,
    rendered_media: &Value,
    operation_count: usize,
) -> Value {
    let reference = &rendered_media["handle"];
    let lineage = &reference["packageLineage"];
    let descriptor_hash = reference["sha256"].as_str().unwrap();
    let operation_hash = reference["operationHash"].as_str().unwrap();
    let receipt_id = crate::motion_artifact::expected_cut_plan_receipt_id(
        package_id,
        motion_id,
        descriptor_hash,
        operation_hash,
        lineage,
    )
    .unwrap();
    let mut input_hashes = serde_json::Map::from_iter([
        ("motion".to_string(), json!(sha256_hex(b"motion:current"))),
        (
            "targetCapabilities".to_string(),
            json!(sha256_hex(b"target:shellx-cut")),
        ),
        (
            "artifactDescriptorSha256".to_string(),
            json!(descriptor_hash),
        ),
        ("artifactOperationHash".to_string(), json!(operation_hash)),
        (
            "manifestSha256".to_string(),
            lineage["manifestSha256"].clone(),
        ),
        ("motionSha256".to_string(), lineage["motionSha256"].clone()),
    ]);
    for field in [
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ] {
        if let Some(value) = lineage.get(field) {
            input_hashes.insert(field.to_string(), value.clone());
        }
    }
    json!({
        "schema": "shellx-motion/receipt@1",
        "id": receipt_id,
        "operation": "cut.import.plan",
        "status": "passed",
        "packageId": package_id,
        "inputHashes": input_hashes,
        "createdAt": "2026-07-11T00:00:03.000Z",
        "lane": "cut",
        "output": {
            "mode": "rendered_media",
            "targetId": "shellx-cut",
            "operationCount": operation_count,
            "unsupportedCount": 0,
            "document": {
                "width": 1280,
                "height": 720,
                "fps": 30,
                "durationMs": 1500
            },
            "renderedMedia": rendered_media,
        },
        "warnings": []
    })
}

fn write_editable_plan(path: &Path) {
    let plan = json!({
        "schema": "shellx-motion/cut-import-plan@1",
        "ok": true,
        "packageId": "pkg-editable-demo",
        "motionId": "motion-editable-demo",
        "targetId": "shellx-cut",
        "mode": "editable_lowering",
        "operations": [
            {
                "verb": "cut.shape.create",
                "sourceLayerId": "panel",
                "startMs": 0,
                "durationMs": 200,
                "payload": {
                    "shape": "rounded-rect",
                    "fill": "#112233",
                    "transform": { "x": 64, "y": 108, "width": 960, "height": 216 },
                    "transitions": {
                        "in": { "type": "fade", "durationMs": 50, "easing": "ease-in-out" },
                        "out": { "type": "fade", "durationMs": 50, "easing": "ease-in-out" }
                    }
                }
            },
            {
                "verb": "cut.title.create",
                "sourceLayerId": "title",
                "startMs": 0,
                "durationMs": 200,
                "payload": {
                    "text": "Native Motion title",
                    "opacity": 0.85,
                    "keyframes": {
                        "opacity": [
                            { "atMs": 0, "value": 0, "easing": "linear" },
                            { "atMs": 100, "value": 0.85, "easing": "linear" },
                            { "atMs": 200, "value": 0.85 }
                        ],
                        "transform.x": [
                            { "atMs": 0, "value": -192, "easing": "ease-out" },
                            { "atMs": 200, "value": 320 }
                        ],
                        "transform.y": [
                            { "atMs": 0, "value": 972, "easing": "ease-out" },
                            { "atMs": 200, "value": 810 }
                        ]
                    },
                    "transform": { "x": 320, "y": 810 },
                    "style": { "color": "#FFFFFF", "fontSize": 64 }
                }
            }
        ],
        "unsupported": [],
        "document": { "width": 1920, "height": 1080, "fps": 30, "durationMs": 200 },
        "receipt": {
            "schema": "shellx-motion/receipt@1",
            "id": "cut-import-editable-demo",
            "operation": "cut.import.plan",
            "status": "passed",
            "packageId": "pkg-editable-demo",
            "inputHashes": { "motion": "sha256:motion", "targetCapabilities": "sha256:target" },
            "createdAt": "2026-07-12T00:00:00.000Z",
            "lane": "cut",
            "output": { "mode": "editable_lowering", "targetId": "shellx-cut", "operationCount": 2, "unsupportedCount": 0 },
            "warnings": []
        }
    });
    fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn write_editable_video_plan(path: &Path, asset_id: &str) {
    let plan = json!({
        "schema": "shellx-motion/cut-import-plan@1",
        "ok": true,
        "packageId": "pkg-editable-video",
        "motionId": "motion-editable-video",
        "targetId": "shellx-cut",
        "mode": "editable_lowering",
        "operations": [{
            "verb": "cut.media.create",
            "sourceLayerId": "footage",
            "startMs": 0,
            "durationMs": 200,
            "payload": {
                "kind": "video",
                "source": format!("cut-asset:{asset_id}"),
                "fit": "cover",
                "trimStartMs": 0,
                "trimDurationMs": 200,
                "playbackRate": 1,
                "includeAudio": false,
                "transform": { "scale": 1, "rotation": 0 },
                "style": {}
            }
        }],
        "unsupported": [],
        "document": { "width": 1920, "height": 1080, "fps": 30, "durationMs": 200 },
        "receipt": {
            "schema": "shellx-motion/receipt@1",
            "id": "cut-import-editable-video",
            "operation": "cut.import.plan",
            "status": "passed",
            "packageId": "pkg-editable-video",
            "inputHashes": { "motion": "sha256:motion", "targetCapabilities": "sha256:target" },
            "createdAt": "2026-07-12T00:00:00.000Z",
            "lane": "cut",
            "output": { "mode": "editable_lowering", "targetId": "shellx-cut", "operationCount": 1, "unsupportedCount": 0 },
            "warnings": []
        }
    });
    fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn write_editable_audio_plan(path: &Path, asset_id: &str) {
    let plan = json!({
        "schema": "shellx-motion/cut-import-plan@1",
        "ok": true,
        "packageId": "pkg-editable-audio",
        "motionId": "motion-editable-audio",
        "targetId": "shellx-cut",
        "mode": "editable_lowering",
        "operations": [{
            "verb": "cut.audio.create",
            "sourceLayerId": "music",
            "startMs": 0,
            "durationMs": 200,
            "payload": {
                "source": format!("cut-asset:{asset_id}"),
                "trimStartMs": 0,
                "trimDurationMs": 200,
                "playbackRate": 1,
                "loop": false,
                "muted": false,
                "normalizeLoudness": false
            }
        }],
        "unsupported": [],
        "document": { "width": 1920, "height": 1080, "fps": 30, "durationMs": 200 },
        "receipt": {
            "schema": "shellx-motion/receipt@1", "id": "cut-import-editable-audio",
            "operation": "cut.import.plan", "status": "passed", "packageId": "pkg-editable-audio",
            "inputHashes": {}, "createdAt": "2026-07-12T00:00:00.000Z", "lane": "cut",
            "output": { "mode": "editable_lowering" }, "warnings": []
        }
    });
    fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn add_motion_integration(path: &Path, protocol: u64) {
    let mut plan: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    plan["integration"] = json!({
        "schema": "shellx-motion/integration-envelope@1",
        "producer": {
            "schema": "shellx-motion/integration-capabilities@1",
            "host": "shellx-motion",
            "protocol": { "min": protocol, "max": protocol, "preferred": protocol },
            "schemas": {
                "package": ["shellx-motion/package-manifest@1", "shellx-motion/motion@1"],
                "artifact": ["shellx-motion/artifact-handle@1", "shellx-motion/artifact-handle-ref@1"],
                "receipt": ["shellx-motion/receipt@1"],
                "cut": ["shellx-motion/cut-import-plan@1"],
                "canvas": ["shellx-motion/canvas-bridge-package@1", "shellx-motion/canvas-frame-selection@1"]
            },
            "modes": ["package.preview", "render.frame", "render.final", "canvas.bridge", "cut.import.plan"],
            "presets": ["png-frame", "jpeg-frame", "png-sequence", "mp4-h264", "webm-vp9", "webm-vp9-alpha", "gif"],
            "features": ["artifact.attestation", "atomic-output", "browser-workflow", "deterministic-seek", "render-session"],
            "limits": { "maxPlanBytes": 4194304, "maxArtifactBytes": 8589934592_u64, "maxOperations": 10000 }
        },
        "binding": {
            "schema": "shellx-motion/integration-binding@1",
            "protocol": protocol,
            "producer": "shellx-motion",
            "consumer": "shellx-cut",
            "mode": "cut.import.plan",
            "payloadSchema": "shellx-motion/cut-import-plan@1",
            "requiredFeatures": ["artifact.attestation"]
        }
    });
    fs::write(path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn write_attested_rendered_media(
    plan_path: &Path,
    render_path: &Path,
    package_id: &str,
    motion_id: &str,
    suffix: &str,
) -> Value {
    let lineage = json!({
        "schema": "shellx-motion/package-render-lineage@1",
        "manifestSha256": sha256_hex(format!("manifest:{package_id}").as_bytes()),
        "motionSha256": sha256_hex(format!("motion:{motion_id}").as_bytes()),
    });
    write_attested_rendered_media_kind(
        plan_path,
        render_path,
        package_id,
        motion_id,
        suffix,
        Some(lineage),
    )
}

fn write_legacy_attested_rendered_media(
    plan_path: &Path,
    render_path: &Path,
    package_id: &str,
    motion_id: &str,
    suffix: &str,
) -> Value {
    write_attested_rendered_media_kind(plan_path, render_path, package_id, motion_id, suffix, None)
}

fn write_gltf_attested_rendered_media(
    plan_path: &Path,
    render_path: &Path,
    package_id: &str,
    motion_id: &str,
    suffix: &str,
) -> Value {
    let lineage = json!({
        "schema": "shellx-motion/package-render-lineage@1",
        "manifestSha256": sha256_hex(format!("manifest:{package_id}").as_bytes()),
        "motionSha256": sha256_hex(format!("motion:{motion_id}").as_bytes()),
        "adapterId": "adapter.gltf",
        "sourceSha256": sha256_hex(format!("gltf-source:{suffix}").as_bytes()),
        "normalizedSourceSha256": sha256_hex(format!("gltf-normalized:{suffix}").as_bytes()),
        "loweringReceiptSha256": sha256_hex(format!("gltf-receipt:{suffix}").as_bytes()),
    });
    write_attested_rendered_media_kind(
        plan_path,
        render_path,
        package_id,
        motion_id,
        suffix,
        Some(lineage),
    )
}

fn write_attested_rendered_media_kind(
    plan_path: &Path,
    render_path: &Path,
    package_id: &str,
    motion_id: &str,
    suffix: &str,
    package_lineage: Option<Value>,
) -> Value {
    let root = fixture_artifact_root(plan_path);
    fs::create_dir_all(root.join("artifacts")).unwrap();
    fs::create_dir_all(root.join("receipts")).unwrap();
    fs::create_dir_all(render_path.parent().expect("render has a parent")).unwrap();

    let media_bytes = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
            .unwrap();
    fs::write(render_path, &media_bytes).unwrap();
    let media_hash = sha256_hex(&media_bytes);
    let operation_hash = sha256_hex(format!("operation:{suffix}").as_bytes());

    let render_input_hashes = if let Some(lineage) = package_lineage.as_ref() {
        let mut hashes = serde_json::Map::from_iter([
            ("operationHash".to_string(), json!(operation_hash)),
            (
                "manifestSha256".to_string(),
                lineage["manifestSha256"].clone(),
            ),
            ("motionSha256".to_string(), lineage["motionSha256"].clone()),
        ]);
        for field in [
            "sourceSha256",
            "normalizedSourceSha256",
            "loweringReceiptSha256",
        ] {
            if let Some(value) = lineage.get(field) {
                hashes.insert(field.to_string(), value.clone());
            }
        }
        Value::Object(hashes)
    } else {
        json!({
            "motion": sha256_hex(format!("motion:{motion_id}").as_bytes()),
            "operation": operation_hash,
        })
    };
    let render_receipt = json!({
        "schema": "shellx-motion/receipt@1",
        "id": format!("render-{suffix}"),
        "operation": "render.final",
        "status": "passed",
        "packageId": package_id,
        "inputHashes": render_input_hashes,
        "createdAt": "2026-07-11T00:00:00.000Z",
        "lane": "render",
        "output": {
            "path": render_path,
            "sha256": media_hash,
            "preset": "png"
        },
        "warnings": []
    });
    let render_receipt_relative = format!("receipts/{suffix}.render.receipt.json");
    let render_receipt_bytes = serde_json::to_vec_pretty(&render_receipt).unwrap();
    fs::write(root.join(&render_receipt_relative), &render_receipt_bytes).unwrap();

    let connector_receipt = json!({
        "schema": "shellx-motion/receipt@1",
        "id": format!("connector-{suffix}"),
        "operation": "connector.template-to-cut",
        "status": "passed",
        "packageId": package_id,
        "inputHashes": {"operation": operation_hash},
        "createdAt": "2026-07-11T00:00:01.000Z",
        "lane": "connector",
        "output": {"targetId": "shellx-cut"},
        "warnings": []
    });
    let connector_receipt_relative = format!("receipts/{suffix}.connector.receipt.json");
    let connector_receipt_bytes = serde_json::to_vec_pretty(&connector_receipt).unwrap();
    if package_lineage.is_none() {
        fs::write(
            root.join(&connector_receipt_relative),
            &connector_receipt_bytes,
        )
        .unwrap();
    }

    let render_relative = render_path
        .strip_prefix(root)
        .expect("render remains under handoff root")
        .to_string_lossy()
        .replace('\\', "/");
    let handle_id = crate::motion_artifact::expected_motion_artifact_handle_id(
        package_id,
        motion_id,
        &operation_hash,
        &media_hash,
        package_lineage.as_ref(),
    )
    .unwrap();
    let mut receipt_attestations = vec![json!({
        "role": "render",
        "rootRelativePath": render_receipt_relative,
        "sha256": sha256_hex(&render_receipt_bytes),
        "id": format!("render-{suffix}"),
        "operation": "render.final",
        "status": "passed"
    })];
    if package_lineage.is_none() {
        receipt_attestations.push(json!({
            "role": "connector",
            "rootRelativePath": connector_receipt_relative,
            "sha256": sha256_hex(&connector_receipt_bytes),
            "id": format!("connector-{suffix}"),
            "operation": "connector.template-to-cut",
            "status": "passed"
        }));
    }
    let mut handle = json!({
        "schema": "shellx-motion/artifact-handle@1",
        "id": handle_id,
        "packageId": package_id,
        "motionId": motion_id,
        "operationHash": operation_hash,
        "preset": "png",
        "mediaType": "image/png",
        "rootRelativePath": render_relative,
        "byteLength": media_bytes.len(),
        "sha256": media_hash,
        "createdAt": "2026-07-11T00:00:02.000Z",
        "receipts": receipt_attestations,
    });
    if let Some(lineage) = package_lineage.as_ref() {
        handle["packageLineage"] = lineage.clone();
    }
    let descriptor_relative = format!("artifacts/{suffix}.artifact.json");
    let descriptor_bytes = serde_json::to_vec_pretty(&handle).unwrap();
    fs::write(root.join(&descriptor_relative), &descriptor_bytes).unwrap();

    let mut reference = json!({
        "dryRun": false,
        "handle": {
            "schema": "shellx-motion/artifact-handle-ref@1",
            "id": handle_id,
            "operationHash": operation_hash,
            "rootRelativePath": descriptor_relative,
            "sha256": sha256_hex(&descriptor_bytes)
        }
    });
    if let Some(lineage) = package_lineage {
        reference["handle"]["packageLineage"] = lineage;
    }
    reference
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fixture_artifact_root(plan_path: &Path) -> &Path {
    let parent = plan_path.parent().expect("plan has a parent");
    if parent.file_name().and_then(|name| name.to_str()) == Some("cut") {
        if let Some(shellx_motion) = parent.parent() {
            if shellx_motion.file_name().and_then(|name| name.to_str()) == Some(".shellx-motion") {
                return shellx_motion
                    .parent()
                    .expect("SDK plan has an artifact root");
            }
        }
    }
    parent
}

fn rewrite_handle_descriptor(plan_path: &Path, mutate: impl FnOnce(&mut Value)) {
    let root = fixture_artifact_root(plan_path);
    let mut plan: Value = serde_json::from_slice(&fs::read(plan_path).unwrap()).unwrap();
    let descriptor_relative = plan["operations"][0]["renderedMedia"]["handle"]["rootRelativePath"]
        .as_str()
        .unwrap()
        .to_string();
    let descriptor_path = root.join(descriptor_relative);
    let mut handle: Value = serde_json::from_slice(&fs::read(&descriptor_path).unwrap()).unwrap();
    mutate(&mut handle);
    let descriptor_bytes = serde_json::to_vec_pretty(&handle).unwrap();
    fs::write(descriptor_path, &descriptor_bytes).unwrap();
    plan["operations"][0]["renderedMedia"]["handle"]["sha256"] =
        json!(sha256_hex(&descriptor_bytes));
    fs::write(plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn rewrite_render_receipt(plan_path: &Path, mutate: impl FnOnce(&mut Value)) {
    let root = fixture_artifact_root(plan_path);
    let mut plan: Value = serde_json::from_slice(&fs::read(plan_path).unwrap()).unwrap();
    let descriptor_relative = plan["operations"][0]["renderedMedia"]["handle"]["rootRelativePath"]
        .as_str()
        .unwrap()
        .to_string();
    let descriptor_path = root.join(descriptor_relative);
    let mut handle: Value = serde_json::from_slice(&fs::read(&descriptor_path).unwrap()).unwrap();
    let render_attestation = handle["receipts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|receipt| receipt["role"] == json!("render"))
        .unwrap();
    let receipt_path = root.join(render_attestation["rootRelativePath"].as_str().unwrap());
    let mut receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    mutate(&mut receipt);
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
    fs::write(receipt_path, &receipt_bytes).unwrap();
    render_attestation["sha256"] = json!(sha256_hex(&receipt_bytes));

    let descriptor_bytes = serde_json::to_vec_pretty(&handle).unwrap();
    fs::write(descriptor_path, &descriptor_bytes).unwrap();
    plan["operations"][0]["renderedMedia"]["handle"]["sha256"] =
        json!(sha256_hex(&descriptor_bytes));
    let rendered_media = plan["operations"][0]["renderedMedia"].clone();
    plan["receipt"] = lineaged_cut_plan_receipt(
        plan["packageId"].as_str().unwrap(),
        plan["motionId"].as_str().unwrap(),
        &rendered_media,
        1,
    );
    fs::write(plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn add_plan_placement(plan_path: &Path) {
    let mut plan: Value = serde_json::from_slice(&fs::read(plan_path).unwrap()).unwrap();
    let operation = &plan["operations"][0];
    let placement = Value::Object(Map::from_iter(
        ["startMs", "durationMs", "track"]
            .into_iter()
            .filter_map(|field| {
                operation
                    .get(field)
                    .map(|value| (field.to_string(), value.clone()))
            }),
    ));
    plan["receipt"]["inputHashes"]["placement"] = json!(placement_hash(&placement));
    plan["receipt"]["output"]["placement"] = placement;
    fs::write(plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
}

fn placement_hash(placement: &Value) -> String {
    let fields = ["startMs", "durationMs", "track"]
        .into_iter()
        .filter_map(|field| {
            placement.get(field).map(|value| {
                format!(
                    "{}:{}",
                    serde_json::to_string(field).unwrap(),
                    serde_json::to_string(value).unwrap()
                )
            })
        })
        .collect::<Vec<_>>();
    sha256_hex(format!("{{{}}}", fields.join(",")).as_bytes())
}
