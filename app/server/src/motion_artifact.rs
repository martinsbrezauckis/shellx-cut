//! Attested ShellX Motion artifact verification for connector imports.

use cut_core::{error_codes, CutError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

#[path = "motion_package_lineage.rs"]
mod motion_package_lineage;
use motion_package_lineage::current_package_lineage;

const DESCRIPTOR_MAX_BYTES: u64 = 4 * 1024 * 1024;
const RECEIPT_MAX_BYTES: u64 = 4 * 1024 * 1024;
const ARTIFACT_MAX_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const FFPROBE_TIMEOUT: Duration = Duration::from_secs(10);
const FFPROBE_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct VerifiedMotionArtifact {
    pub path: PathBuf,
    pub handle: Value,
    pub proof: Value,
}

#[derive(Debug)]
struct VerifiedReceiptProof {
    render: Value,
    connector: Option<Value>,
}

pub(crate) async fn verify_motion_artifact_reference(
    plan_path: &Path,
    package_dir: Option<&Path>,
    plan: &Value,
    operation: &Value,
    rendered_media: &Value,
    plan_receipt: &Value,
    package_id: &str,
    motion_id: &str,
) -> Result<VerifiedMotionArtifact, CutError> {
    let plan_path = tokio::fs::canonicalize(plan_path).await.map_err(|error| {
        artifact_error(
            "canonicalize Motion import plan",
            format!("{}: {error}", plan_path.display()),
        )
    })?;
    let root = artifact_root_for_plan(&plan_path)?;
    let reference = object_field(rendered_media, "handle")?;
    require_string(reference, "schema", "shellx-motion/artifact-handle-ref@1")?;
    let reference_id = required_string(reference, "id")?;
    require_artifact_id(&reference_id, "artifact handle reference id")?;
    let reference_operation_hash = required_sha256(reference, "operationHash")?;
    let reference_lineage =
        optional_package_lineage(reference, "packageLineage", "Motion artifact reference")?;
    require_exact_fields(
        reference,
        "Motion artifact reference",
        &[
            "schema",
            "id",
            "operationHash",
            "rootRelativePath",
            "sha256",
        ],
        if reference_lineage.is_some() {
            &["packageLineage"]
        } else {
            &[]
        },
    )?;
    let descriptor_relative = required_canonical_relative(reference, "rootRelativePath")?;
    let descriptor_expected_hash = required_sha256(reference, "sha256")?;
    let descriptor_path =
        canonical_root_file(&root, &descriptor_relative, "artifact handle descriptor").await?;
    let descriptor_bytes = read_hashed_file(
        &descriptor_path,
        DESCRIPTOR_MAX_BYTES,
        DESCRIPTOR_MAX_BYTES as usize,
        "artifact handle descriptor",
    )
    .await?;
    if descriptor_bytes.sha256 != descriptor_expected_hash {
        return Err(artifact_error(
            "Motion artifact handle descriptor hash mismatch",
            format!(
                "expected {descriptor_expected_hash}, got {}",
                descriptor_bytes.sha256
            ),
        ));
    }
    let handle: Value = serde_json::from_slice(&descriptor_bytes.bytes).map_err(|error| {
        artifact_error("Motion artifact handle is invalid JSON", error.to_string())
    })?;
    require_string(&handle, "schema", "shellx-motion/artifact-handle@1")?;
    require_string(&handle, "id", &reference_id)?;
    require_string(&handle, "operationHash", &reference_operation_hash)?;
    require_string(&handle, "packageId", package_id)?;
    require_string(&handle, "motionId", motion_id)?;
    let handle_lineage =
        optional_package_lineage(&handle, "packageLineage", "Motion artifact handle")?;
    require_exact_fields(
        &handle,
        "Motion artifact handle",
        &[
            "schema",
            "id",
            "packageId",
            "motionId",
            "operationHash",
            "preset",
            "mediaType",
            "rootRelativePath",
            "byteLength",
            "sha256",
            "createdAt",
            "receipts",
        ],
        if handle_lineage.is_some() {
            &["packageLineage", "probe", "qualityEvidence"]
        } else {
            &["probe", "qualityEvidence"]
        },
    )?;
    if handle_lineage != reference_lineage {
        return Err(artifact_error(
            "Motion artifact reference package lineage does not match its descriptor",
            "the handoff must preserve one exact lineage object",
        ));
    }
    let preset = required_string(&handle, "preset")?;
    let media_type = required_string(&handle, "mediaType")?;
    let created_at = required_string(&handle, "createdAt")?;
    if chrono::DateTime::parse_from_rfc3339(&created_at).is_err() {
        return Err(artifact_error(
            "Motion artifact handle createdAt is invalid",
            created_at,
        ));
    }
    let artifact_relative = required_canonical_relative(&handle, "rootRelativePath")?;
    let artifact_expected_hash = required_sha256(&handle, "sha256")?;
    let expected_handle_id = expected_motion_artifact_handle_id(
        package_id,
        motion_id,
        &reference_operation_hash,
        &artifact_expected_hash,
        handle_lineage.as_ref(),
    )?;
    if reference_id != expected_handle_id {
        return Err(artifact_error(
            "Motion artifact handle id does not bind its identity and package lineage",
            format!("expected {expected_handle_id}, got {reference_id}"),
        ));
    }
    let artifact_expected_size = handle
        .get("byteLength")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            artifact_error(
                "Motion artifact byteLength is invalid",
                "expected a non-negative integer",
            )
        })?;
    if artifact_expected_size > ARTIFACT_MAX_BYTES {
        return Err(artifact_error(
            "Motion artifact exceeds the Cut size limit",
            artifact_expected_size.to_string(),
        ));
    }
    let artifact_path = canonical_root_file(&root, &artifact_relative, "rendered artifact").await?;
    let artifact =
        read_hashed_file(&artifact_path, ARTIFACT_MAX_BYTES, 16, "rendered artifact").await?;
    if artifact.byte_length != artifact_expected_size || artifact.sha256 != artifact_expected_hash {
        return Err(artifact_error(
            "Motion rendered artifact does not match its handle",
            format!(
                "expected {} bytes / {}, got {} bytes / {}",
                artifact_expected_size,
                artifact_expected_hash,
                artifact.byte_length,
                artifact.sha256
            ),
        ));
    }
    validate_media_magic(&artifact.bytes, &media_type)?;
    let receipt_proof = verify_receipts(
        &root,
        &handle,
        package_id,
        &reference_operation_hash,
        &preset,
        &artifact_path,
        &artifact_expected_hash,
        handle_lineage.as_ref(),
    )
    .await?;
    if let Some(lineage) = handle_lineage.as_ref() {
        verify_plan_receipt(
            plan_receipt,
            plan,
            operation,
            rendered_media,
            package_id,
            motion_id,
            &descriptor_expected_hash,
            &reference_operation_hash,
            lineage,
        )?;
    }
    probe_media_bounded(&artifact_path, &media_type).await?;
    let proof = json_lineage_proof(
        &reference_id,
        &reference_operation_hash,
        &descriptor_expected_hash,
        handle_lineage.as_ref(),
        current_package_lineage(package_dir, handle_lineage.as_ref()).await,
        receipt_proof,
        plan_receipt,
    );
    Ok(VerifiedMotionArtifact {
        path: artifact_path,
        handle,
        proof,
    })
}

/// Resolve the root against which an artifact reference's `rootRelativePath`
/// is defined. Current local-SDK handoffs live at
/// `<artifactRoot>/.shellx-motion/cut/<hash>.cut-import-plan.json` and bind
/// paths relative to `artifactRoot`; legacy connector plans bind paths relative
/// to the plan directory itself. The canonical plan path makes this structural
/// distinction path-only and keeps both roots inside the selected handoff tree.
fn artifact_root_for_plan(plan_path: &Path) -> Result<PathBuf, CutError> {
    let parent = plan_path.parent().ok_or_else(|| {
        artifact_error(
            "Motion import plan has no trusted parent directory",
            plan_path.display().to_string(),
        )
    })?;
    if parent.file_name().and_then(|name| name.to_str()) == Some("cut") {
        if let Some(shellx_motion) = parent.parent() {
            if shellx_motion.file_name().and_then(|name| name.to_str()) == Some(".shellx-motion") {
                if let Some(artifact_root) = shellx_motion.parent() {
                    return Ok(artifact_root.to_path_buf());
                }
            }
        }
    }
    Ok(parent.to_path_buf())
}

async fn verify_receipts(
    root: &Path,
    handle: &Value,
    package_id: &str,
    operation_hash: &str,
    preset: &str,
    artifact_path: &Path,
    artifact_hash: &str,
    package_lineage: Option<&Value>,
) -> Result<VerifiedReceiptProof, CutError> {
    let receipts = handle
        .get("receipts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            artifact_error(
                "Motion artifact handle has no receipt attestations",
                "expected render and connector receipts",
            )
        })?;
    let mut saw_render = false;
    let mut saw_connector = false;
    let mut render_proof = None;
    let mut connector_proof = None;
    for attestation in receipts {
        require_exact_fields(
            attestation,
            "Motion artifact receipt attestation",
            &[
                "role",
                "id",
                "operation",
                "status",
                "rootRelativePath",
                "sha256",
            ],
            &[],
        )?;
        let role = required_string(attestation, "role")?;
        if role != "render" && role != "connector" {
            return Err(artifact_error(
                "Motion artifact receipt role is unsupported",
                role,
            ));
        }
        if (role == "render" && saw_render) || (role == "connector" && saw_connector) {
            return Err(artifact_error(
                "Motion artifact has duplicate receipt attestations",
                role,
            ));
        }
        saw_render |= role == "render";
        saw_connector |= role == "connector";
        let expected_id = required_string(attestation, "id")?;
        let expected_operation = required_string(attestation, "operation")?;
        let expected_status = required_success_status(attestation, "status")?;
        let receipt_relative = required_canonical_relative(attestation, "rootRelativePath")?;
        let expected_hash = required_sha256(attestation, "sha256")?;
        let receipt_path =
            canonical_root_file(root, &receipt_relative, &format!("{role} receipt")).await?;
        let bytes = read_hashed_file(
            &receipt_path,
            RECEIPT_MAX_BYTES,
            RECEIPT_MAX_BYTES as usize,
            &format!("{role} receipt"),
        )
        .await?;
        if bytes.sha256 != expected_hash {
            return Err(artifact_error(
                format!("Motion {role} receipt hash mismatch"),
                format!("expected {expected_hash}, got {}", bytes.sha256),
            ));
        }
        let receipt: Value = serde_json::from_slice(&bytes.bytes).map_err(|error| {
            artifact_error(
                format!("Motion {role} receipt is invalid JSON"),
                error.to_string(),
            )
        })?;
        require_string(&receipt, "schema", "shellx-motion/receipt@1")?;
        require_string(&receipt, "id", &expected_id)?;
        require_string(&receipt, "operation", &expected_operation)?;
        require_string(&receipt, "status", &expected_status)?;
        require_string(&receipt, "packageId", package_id)?;
        let input_hashes = if package_lineage.is_some() {
            required_hash_record(&receipt, "inputHashes", &format!("Motion {role} receipt"))?
        } else {
            required_nonempty_string_record(
                &receipt,
                "inputHashes",
                &format!("Motion {role} receipt"),
            )?
        };
        let created_at = required_string(&receipt, "createdAt")?;
        if chrono::DateTime::parse_from_rfc3339(&created_at).is_err() {
            return Err(artifact_error(
                format!("Motion {role} receipt createdAt is invalid"),
                created_at,
            ));
        }
        required_string(&receipt, "lane")?;
        if !receipt.get("warnings").is_some_and(Value::is_array) {
            return Err(artifact_error(
                format!("Motion {role} receipt warnings are invalid"),
                "expected an array",
            ));
        }
        if role == "render" {
            let output = object_field(&receipt, "output")?;
            require_string(output, "sha256", artifact_hash)?;
            if output.get("preset").and_then(Value::as_str).is_some() {
                require_string(output, "preset", preset)?;
            }
            let output_path = required_string(output, "path")?;
            let output_path =
                canonical_existing_path(root, &output_path, "render receipt output").await?;
            if output_path != artifact_path {
                return Err(artifact_error(
                    "Motion render receipt points to another artifact",
                    output_path.display().to_string(),
                ));
            }
            if let Some(lineage) = package_lineage {
                let mut expected = vec![("operationHash", operation_hash.to_string())];
                expected.extend(package_lineage_hashes(lineage)?);
                require_exact_hash_record(
                    &input_hashes,
                    &expected,
                    "Motion render receipt inputHashes",
                )?;
            } else if !input_hashes.values().any(|value| value == operation_hash) {
                return Err(artifact_error(
                    "Motion render receipt does not bind the artifact operation hash",
                    operation_hash.to_string(),
                ));
            }
            render_proof = Some(json_receipt_proof(&receipt, &expected_hash));
        } else {
            if !input_hashes.values().any(|value| value == operation_hash) {
                return Err(artifact_error(
                    "Motion connector receipt does not bind the artifact operation hash",
                    operation_hash.to_string(),
                ));
            }
            connector_proof = Some(json_receipt_proof(&receipt, &expected_hash));
        }
    }
    let missing_required_receipt = !saw_render || (package_lineage.is_none() && !saw_connector);
    if missing_required_receipt {
        return Err(artifact_error(
            "Motion artifact handle is missing required receipt attestations",
            if package_lineage.is_some() {
                "lineage-backed SDK handoffs require a render receipt and a bound Cut plan receipt"
            } else {
                "legacy handoffs require both render and connector receipts"
            },
        ));
    }
    Ok(VerifiedReceiptProof {
        render: render_proof.expect("required render receipt was checked"),
        connector: connector_proof,
    })
}

fn json_receipt_proof(receipt: &Value, sha256: &str) -> Value {
    serde_json::json!({
        "id": receipt.get("id").cloned().unwrap_or(Value::Null),
        "operation": receipt.get("operation").cloned().unwrap_or(Value::Null),
        "status": receipt.get("status").cloned().unwrap_or(Value::Null),
        "sha256": sha256,
    })
}

fn json_lineage_proof(
    artifact_handle_id: &str,
    operation_hash: &str,
    descriptor_hash: &str,
    package_lineage: Option<&Value>,
    current_package: Value,
    receipts: VerifiedReceiptProof,
    plan_receipt: &Value,
) -> Value {
    serde_json::json!({
        "schema": "shellx-cut/motion-import-attestation@1",
        "status": if package_lineage.is_some() { "verified" } else { "legacy-unverified" },
        "artifactHandleId": artifact_handle_id,
        "artifactOperationHash": operation_hash,
        "artifactDescriptorSha256": descriptor_hash,
        "packageLineage": package_lineage.cloned().unwrap_or(Value::Null),
        "currentPackage": current_package,
        "renderReceipt": receipts.render,
        "connectorReceipt": receipts.connector.unwrap_or(Value::Null),
        "cutPlanReceipt": if package_lineage.is_some() {
            serde_json::json!({
                "id": plan_receipt.get("id").cloned().unwrap_or(Value::Null),
                "operation": plan_receipt.get("operation").cloned().unwrap_or(Value::Null),
                "status": plan_receipt.get("status").cloned().unwrap_or(Value::Null),
            })
        } else {
            Value::Null
        },
    })
}

fn verify_plan_receipt(
    receipt: &Value,
    plan: &Value,
    operation: &Value,
    rendered_media: &Value,
    package_id: &str,
    motion_id: &str,
    descriptor_hash: &str,
    operation_hash: &str,
    package_lineage: &Value,
) -> Result<(), CutError> {
    require_string(receipt, "schema", "shellx-motion/receipt@1")?;
    let receipt_id = required_string(receipt, "id")?;
    let expected_receipt_id = expected_cut_plan_receipt_id(
        package_id,
        motion_id,
        descriptor_hash,
        operation_hash,
        package_lineage,
    )?;
    if receipt_id != expected_receipt_id {
        return Err(artifact_error(
            "Motion Cut plan receipt id does not bind the artifact and package lineage",
            format!("expected {expected_receipt_id}, got {receipt_id}"),
        ));
    }
    require_string(receipt, "operation", "cut.import.plan")?;
    required_success_status(receipt, "status")?;
    require_string(receipt, "packageId", package_id)?;
    require_string(receipt, "lane", "cut")?;
    let created_at = required_string(receipt, "createdAt")?;
    if chrono::DateTime::parse_from_rfc3339(&created_at).is_err() {
        return Err(artifact_error(
            "Motion Cut plan receipt createdAt is invalid",
            created_at,
        ));
    }
    if !receipt.get("warnings").is_some_and(Value::is_array) {
        return Err(artifact_error(
            "Motion Cut plan receipt warnings are invalid",
            "expected an array",
        ));
    }
    let input_hashes = required_hash_record(receipt, "inputHashes", "Motion Cut plan receipt")?;
    let mut expected = vec![
        ("artifactDescriptorSha256", descriptor_hash.to_string()),
        ("artifactOperationHash", operation_hash.to_string()),
    ];
    expected.extend(package_lineage_hashes(package_lineage)?);
    let mut allowed = vec!["motion", "targetCapabilities", "placement"];
    allowed.extend(expected.iter().map(|(key, _)| *key));
    if !input_hashes.contains_key("motion")
        || !input_hashes.contains_key("targetCapabilities")
        || input_hashes
            .keys()
            .any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(artifact_error(
            "Motion Cut plan receipt inputHashes contain an incomplete or unsupported commitment",
            "expected motion, targetCapabilities, optional placement, and the exact artifact/lineage hashes",
        ));
    }
    for (key, value) in &expected {
        if input_hashes.get(*key) != Some(value) {
            return Err(artifact_error(
                "Motion Cut plan receipt does not bind the artifact and package lineage",
                format!("{key} does not match"),
            ));
        }
    }
    for key in [
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ] {
        if !expected
            .iter()
            .any(|(expected_key, _)| *expected_key == key)
            && input_hashes.contains_key(key)
        {
            return Err(artifact_error(
                "Motion Cut plan receipt contains lineage hashes without adapter provenance",
                key,
            ));
        }
    }
    let output = object_field(receipt, "output")?;
    require_string(output, "mode", "rendered_media")?;
    require_string(output, "targetId", "shellx-cut")?;
    let operations = plan
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            artifact_error(
                "Motion Cut plan operations are invalid",
                "expected a non-empty array",
            )
        })?;
    if operations.len() != 1
        || output.get("operationCount").and_then(Value::as_u64) != Some(operations.len() as u64)
    {
        return Err(artifact_error(
            "Motion Cut plan receipt operationCount is invalid",
            "lineage-backed SDK handoffs contain and authorize exactly one rendered-media operation",
        ));
    }
    let unsupported = plan
        .get("unsupported")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            artifact_error(
                "Motion Cut plan unsupported diagnostics are invalid",
                "expected an array",
            )
        })?;
    if output.get("unsupportedCount").and_then(Value::as_u64) != Some(unsupported.len() as u64) {
        return Err(artifact_error(
            "Motion Cut plan receipt unsupportedCount is invalid",
            "receipt.output.unsupportedCount must match the plan diagnostics",
        ));
    }
    let expected_warnings = unsupported
        .iter()
        .map(|item| {
            item.get("reason")
                .and_then(Value::as_str)
                .map(Value::from)
                .ok_or_else(|| {
                    artifact_error(
                        "Motion Cut plan unsupported diagnostic is invalid",
                        "each diagnostic requires a string reason",
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if receipt.get("warnings").and_then(Value::as_array) != Some(&expected_warnings) {
        return Err(artifact_error(
            "Motion Cut plan receipt warnings are invalid",
            "receipt warnings must exactly match unsupported diagnostic reasons",
        ));
    }
    if output.get("renderedMedia") != Some(rendered_media) {
        return Err(artifact_error(
            "Motion Cut plan receipt does not bind the rendered-media operation",
            "receipt.output.renderedMedia must exactly match the operation",
        ));
    }
    let document = object_field(plan, "document")?;
    if output.get("document") != Some(document) {
        return Err(artifact_error(
            "Motion Cut plan receipt does not bind its document metadata",
            "receipt.output.document must exactly match plan.document",
        ));
    }
    verify_plan_document(document, operation)?;
    verify_plan_placement(&input_hashes, output, document, operation)?;
    Ok(())
}

fn verify_plan_document(document: &Value, operation: &Value) -> Result<(), CutError> {
    require_exact_fields(
        document,
        "Motion Cut plan document",
        &["width", "height", "fps", "durationMs"],
        &["background", "safeAreas"],
    )?;
    let media = object_field(operation, "media")?;
    for field in ["width", "height", "fps"] {
        let value = document
            .get(field)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| {
                artifact_error(
                    "Motion Cut plan document metadata is invalid",
                    format!("{field} must be a positive finite number"),
                )
            })?;
        if media.get(field) != document.get(field) {
            return Err(artifact_error(
                "Motion Cut rendered-media metadata does not match its receipt document",
                format!("{field} differs from receipt.output.document ({value})"),
            ));
        }
    }
    if document
        .get("durationMs")
        .and_then(Value::as_u64)
        .is_none_or(|value| value == 0)
    {
        return Err(artifact_error(
            "Motion Cut plan document durationMs is invalid",
            "expected a positive integer",
        ));
    }
    Ok(())
}

fn verify_plan_placement(
    input_hashes: &std::collections::BTreeMap<String, String>,
    output: &Value,
    document: &Value,
    operation: &Value,
) -> Result<(), CutError> {
    let Some(expected_hash) = input_hashes.get("placement") else {
        if output.get("placement").is_some() {
            return Err(artifact_error(
                "Motion Cut plan receipt has uncommitted placement output",
                "receipt.output.placement requires inputHashes.placement",
            ));
        }
        verify_operation_placement_defaults(None, document, operation)?;
        return Ok(());
    };
    let placement = object_field(output, "placement")?;
    require_exact_fields(
        placement,
        "Motion Cut plan receipt placement",
        &[],
        &["startMs", "durationMs", "track"],
    )?;
    let mut fields = Vec::new();
    for field in ["startMs", "durationMs", "track"] {
        if let Some(value) = placement.get(field) {
            if operation.get(field) != Some(value) {
                return Err(artifact_error(
                    "Motion Cut plan receipt placement does not match its operation",
                    field,
                ));
            }
            fields.push(format!(
                "{}:{}",
                serde_json::to_string(field).expect("serialize placement field"),
                serde_json::to_string(value).expect("serialize placement value")
            ));
        }
    }
    let commitment = format!("{{{}}}", fields.join(","));
    let actual_hash = format!("{:x}", Sha256::digest(commitment.as_bytes()));
    if &actual_hash != expected_hash {
        return Err(artifact_error(
            "Motion Cut plan receipt placement hash mismatch",
            "regenerate the placed Cut handoff",
        ));
    }
    verify_operation_placement_defaults(Some(placement), document, operation)?;
    Ok(())
}

fn verify_operation_placement_defaults(
    placement: Option<&Value>,
    document: &Value,
    operation: &Value,
) -> Result<(), CutError> {
    let expected_start = placement
        .and_then(|value| value.get("startMs"))
        .cloned()
        .unwrap_or_else(|| Value::from(0));
    let expected_duration = placement
        .and_then(|value| value.get("durationMs"))
        .or_else(|| document.get("durationMs"))
        .cloned()
        .ok_or_else(|| {
            artifact_error(
                "Motion Cut plan document durationMs is missing",
                "regenerate the Cut handoff",
            )
        })?;
    if operation.get("startMs") != Some(&expected_start)
        || operation.get("durationMs") != Some(&expected_duration)
    {
        return Err(artifact_error(
            "Motion Cut rendered-media timing is not receipt-bound",
            "changed startMs or durationMs requires an exact placement commitment",
        ));
    }
    match placement.and_then(|value| value.get("track")) {
        Some(expected_track) if operation.get("track") != Some(expected_track) => {
            return Err(artifact_error(
                "Motion Cut rendered-media track is not receipt-bound",
                "operation.track must match receipt.output.placement.track",
            ));
        }
        None if operation.get("track").is_some() => {
            return Err(artifact_error(
                "Motion Cut rendered-media track is not receipt-bound",
                "adding a track requires an exact placement commitment",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn optional_package_lineage(
    value: &Value,
    field: &str,
    label: &str,
) -> Result<Option<Value>, CutError> {
    let Some(lineage) = value.get(field) else {
        return Ok(None);
    };
    if !lineage.is_object() {
        return Err(artifact_error(
            format!("{label} package lineage is invalid"),
            "expected shellx-motion/package-render-lineage@1",
        ));
    }
    let object = lineage.as_object().expect("checked object");
    let allowed = [
        "schema",
        "manifestSha256",
        "motionSha256",
        "adapterId",
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ];
    if let Some(field) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(artifact_error(
            format!("{label} package lineage contains an unsupported field"),
            field.clone(),
        ));
    }
    require_string(lineage, "schema", "shellx-motion/package-render-lineage@1")?;
    required_sha256(lineage, "manifestSha256")?;
    required_sha256(lineage, "motionSha256")?;
    let adapter_id = lineage.get("adapterId").and_then(Value::as_str);
    let has_adapter_hash = [
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ]
    .iter()
    .any(|field| lineage.get(*field).is_some());
    match adapter_id {
        None if has_adapter_hash => {
            return Err(artifact_error(
                format!("{label} package lineage has adapter hashes without adapterId"),
                "glTF lineage requires adapter.gltf and all three provenance hashes",
            ));
        }
        Some("adapter.gltf") => {
            required_sha256(lineage, "sourceSha256")?;
            required_sha256(lineage, "normalizedSourceSha256")?;
            required_sha256(lineage, "loweringReceiptSha256")?;
        }
        Some(other) => {
            return Err(artifact_error(
                format!("{label} package lineage adapter is unsupported"),
                other.to_string(),
            ));
        }
        None => {}
    }
    Ok(Some(lineage.clone()))
}

fn require_exact_fields(
    value: &Value,
    label: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), CutError> {
    let object = value
        .as_object()
        .ok_or_else(|| artifact_error(format!("{label} is invalid"), "expected an object"))?;
    if let Some(field) = required.iter().find(|field| !object.contains_key(**field)) {
        return Err(artifact_error(
            format!("{label} is incomplete"),
            format!("missing required field {field}"),
        ));
    }
    if let Some(field) = object
        .keys()
        .find(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(artifact_error(
            format!("{label} contains an unsupported field"),
            field.clone(),
        ));
    }
    Ok(())
}

fn package_lineage_hashes(lineage: &Value) -> Result<Vec<(&'static str, String)>, CutError> {
    let mut hashes = vec![
        (
            "manifestSha256",
            required_sha256(lineage, "manifestSha256")?,
        ),
        ("motionSha256", required_sha256(lineage, "motionSha256")?),
    ];
    if lineage.get("adapterId").and_then(Value::as_str) == Some("adapter.gltf") {
        hashes.extend([
            ("sourceSha256", required_sha256(lineage, "sourceSha256")?),
            (
                "normalizedSourceSha256",
                required_sha256(lineage, "normalizedSourceSha256")?,
            ),
            (
                "loweringReceiptSha256",
                required_sha256(lineage, "loweringReceiptSha256")?,
            ),
        ]);
    }
    Ok(hashes)
}

fn required_hash_record(
    value: &Value,
    field: &str,
    label: &str,
) -> Result<std::collections::BTreeMap<String, String>, CutError> {
    let object = value.get(field).and_then(Value::as_object).ok_or_else(|| {
        artifact_error(
            format!("{label} {field} are invalid"),
            "expected a hash object",
        )
    })?;
    let mut result = std::collections::BTreeMap::new();
    for (key, value) in object {
        let hash = value.as_str().ok_or_else(|| {
            artifact_error(
                format!("{label} {field} are invalid"),
                format!("{key} must be a lowercase sha256"),
            )
        })?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(artifact_error(
                format!("{label} {field} are invalid"),
                format!("{key} must be a lowercase sha256"),
            ));
        }
        result.insert(key.clone(), hash.to_string());
    }
    Ok(result)
}

fn required_nonempty_string_record(
    value: &Value,
    field: &str,
    label: &str,
) -> Result<std::collections::BTreeMap<String, String>, CutError> {
    let object = value.get(field).and_then(Value::as_object).ok_or_else(|| {
        artifact_error(
            format!("{label} {field} are invalid"),
            "expected a string record",
        )
    })?;
    let mut result = std::collections::BTreeMap::new();
    for (key, value) in object {
        let text = value
            .as_str()
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                artifact_error(
                    format!("{label} {field} are invalid"),
                    format!("{key} must be a non-empty string"),
                )
            })?;
        result.insert(key.clone(), text.to_string());
    }
    Ok(result)
}

fn require_exact_hash_record(
    actual: &std::collections::BTreeMap<String, String>,
    expected: &[(&str, String)],
    label: &str,
) -> Result<(), CutError> {
    if actual.len() != expected.len()
        || expected
            .iter()
            .any(|(key, value)| actual.get(*key) != Some(value))
    {
        return Err(artifact_error(
            format!("{label} do not exactly bind the artifact operation and package lineage"),
            "regenerate the handoff from the unchanged Motion package",
        ));
    }
    Ok(())
}

pub(crate) fn expected_motion_artifact_handle_id(
    package_id: &str,
    motion_id: &str,
    operation_hash: &str,
    artifact_hash: &str,
    package_lineage: Option<&Value>,
) -> Result<String, CutError> {
    let lineage = package_lineage
        .map(canonical_package_lineage_json)
        .transpose()?
        .map(|lineage| format!(",\"packageLineage\":{lineage}"))
        .unwrap_or_default();
    let identity = format!(
        "{{\"packageId\":{},\"motionId\":{},\"operationHash\":{},\"sha256\":{}{lineage}}}",
        serde_json::to_string(package_id).expect("serialize package id"),
        serde_json::to_string(motion_id).expect("serialize motion id"),
        serde_json::to_string(operation_hash).expect("serialize operation hash"),
        serde_json::to_string(artifact_hash).expect("serialize artifact hash"),
    );
    Ok(format!(
        "artifact-{}",
        &format!("{:x}", Sha256::digest(identity.as_bytes()))[..24]
    ))
}

fn canonical_package_lineage_json(lineage: &Value) -> Result<String, CutError> {
    optional_package_lineage(
        &serde_json::json!({ "packageLineage": lineage }),
        "packageLineage",
        "Motion artifact",
    )?;
    let mut fields = vec![
        format!(
            "\"schema\":{}",
            serde_json::to_string("shellx-motion/package-render-lineage@1").unwrap()
        ),
        format!(
            "\"manifestSha256\":{}",
            serde_json::to_string(&required_sha256(lineage, "manifestSha256")?).unwrap()
        ),
        format!(
            "\"motionSha256\":{}",
            serde_json::to_string(&required_sha256(lineage, "motionSha256")?).unwrap()
        ),
    ];
    if lineage.get("adapterId").and_then(Value::as_str) == Some("adapter.gltf") {
        fields.extend([
            format!(
                "\"adapterId\":{}",
                serde_json::to_string("adapter.gltf").unwrap()
            ),
            format!(
                "\"sourceSha256\":{}",
                serde_json::to_string(&required_sha256(lineage, "sourceSha256")?).unwrap()
            ),
            format!(
                "\"normalizedSourceSha256\":{}",
                serde_json::to_string(&required_sha256(lineage, "normalizedSourceSha256")?)
                    .unwrap()
            ),
            format!(
                "\"loweringReceiptSha256\":{}",
                serde_json::to_string(&required_sha256(lineage, "loweringReceiptSha256")?).unwrap()
            ),
        ]);
    }
    Ok(format!("{{{}}}", fields.join(",")))
}

pub(crate) fn expected_cut_plan_receipt_id(
    package_id: &str,
    motion_id: &str,
    descriptor_hash: &str,
    operation_hash: &str,
    lineage: &Value,
) -> Result<String, CutError> {
    let lineage_hashes = package_lineage_hashes(lineage)?;
    let mut lineage_fields = vec![format!(
        "\"schema\":{}",
        serde_json::to_string("shellx-motion/package-render-lineage@1").unwrap()
    )];
    if lineage.get("adapterId").and_then(Value::as_str) == Some("adapter.gltf") {
        lineage_fields.push(format!(
            "\"adapterId\":{}",
            serde_json::to_string("adapter.gltf").unwrap()
        ));
    }
    lineage_fields.extend(lineage_hashes.into_iter().map(|(key, value)| {
        format!(
            "\"{key}\":{}",
            serde_json::to_string(&value).expect("serialize lineage hash")
        )
    }));
    let commitment = format!(
        "{{\"packageId\":{},\"motionId\":{},\"mode\":\"rendered_media\",\"artifactDescriptorSha256\":{},\"artifactOperationHash\":{},\"packageLineage\":{{{}}}}}",
        serde_json::to_string(package_id).expect("serialize package id"),
        serde_json::to_string(motion_id).expect("serialize motion id"),
        serde_json::to_string(descriptor_hash).expect("serialize descriptor hash"),
        serde_json::to_string(operation_hash).expect("serialize operation hash"),
        lineage_fields.join(","),
    );
    Ok(format!(
        "cut-import-{}",
        &format!("{:x}", Sha256::digest(commitment.as_bytes()))[..16]
    ))
}

async fn probe_media_bounded(path: &Path, media_type: &str) -> Result<(), CutError> {
    let mut command = tokio::process::Command::new(cut_media::toolpath::ffprobe());
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=format_name,duration,size:stream=index,codec_type,codec_name,width,height",
            "-of",
            "json",
        ])
        .arg(path);
    let output = crate::jobs::run_owned(
        &mut command,
        None,
        &crate::jobs::ProcessControl::for_operation(FFPROBE_TIMEOUT)
            .with_output_cap(FFPROBE_MAX_OUTPUT_BYTES),
    )
    .await
    .map_err(|error| match error.termination() {
        Some(crate::jobs::ProcessTermination::DeadlineExceeded) => artifact_error(
            "Motion artifact ffprobe timed out",
            path.display().to_string(),
        ),
        _ => artifact_error(
            "failed to collect ffprobe output for Motion artifact",
            error.to_string(),
        ),
    })?;
    if output.diagnostics_truncated() {
        return Err(artifact_error(
            "Motion artifact ffprobe output exceeded its limit",
            path.display().to_string(),
        ));
    }
    if !output.status.success() {
        return Err(artifact_error(
            "Motion artifact failed ffprobe validation",
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let probe: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        artifact_error(
            "Motion artifact ffprobe returned invalid JSON",
            error.to_string(),
        )
    })?;
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .filter(|streams| !streams.is_empty())
        .ok_or_else(|| {
            artifact_error(
                "Motion artifact ffprobe found no media streams",
                path.display().to_string(),
            )
        })?;
    if media_type.starts_with("video/")
        && !streams
            .iter()
            .any(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"))
    {
        return Err(artifact_error(
            "Motion video artifact has no video stream",
            path.display().to_string(),
        ));
    }
    Ok(())
}

struct HashedBytes {
    bytes: Vec<u8>,
    byte_length: u64,
    sha256: String,
}

async fn read_hashed_file(
    path: &Path,
    max_bytes: u64,
    capture_bytes: usize,
    label: &str,
) -> Result<HashedBytes, CutError> {
    let path = path.to_path_buf();
    let label = label.to_string();
    let task_label = label.clone();
    tokio::task::spawn_blocking(move || {
        read_hashed_file_sync(&path, max_bytes, capture_bytes, &task_label)
    })
    .await
    .map_err(|error| artifact_error(format!("verify {label}"), error.to_string()))?
}

fn read_hashed_file_sync(
    path: &Path,
    max_bytes: u64,
    capture_bytes: usize,
    label: &str,
) -> Result<HashedBytes, CutError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| artifact_error(format!("inspect {label}"), error.to_string()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(artifact_error(
            format!("{label} is not a regular file"),
            path.display().to_string(),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(artifact_error(
            format!("{label} exceeds its byte limit"),
            metadata.len().to_string(),
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| artifact_error(format!("open {label}"), error.to_string()))?;
    let opened = file
        .metadata()
        .map_err(|error| artifact_error(format!("inspect open {label}"), error.to_string()))?;
    if !same_file_identity(&metadata, &opened) {
        return Err(artifact_error(
            format!("{label} changed before verification"),
            path.display().to_string(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity((opened.len().min(max_bytes) as usize).min(capture_bytes));
    let mut buffer = [0_u8; 1024 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| artifact_error(format!("read {label}"), error.to_string()))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > max_bytes {
            return Err(artifact_error(
                format!("{label} exceeds its byte limit"),
                total.to_string(),
            ));
        }
        hasher.update(&buffer[..read]);
        let capture = capture_bytes.saturating_sub(bytes.len()).min(read);
        bytes.extend_from_slice(&buffer[..capture]);
    }
    let after = file
        .metadata()
        .map_err(|error| artifact_error(format!("reinspect {label}"), error.to_string()))?;
    let path_after = std::fs::symlink_metadata(path).map_err(|error| {
        artifact_error(format!("reinspect path for {label}"), error.to_string())
    })?;
    if total != opened.len()
        || !same_file_identity(&opened, &after)
        || !same_file_identity(&opened, &path_after)
    {
        return Err(artifact_error(
            format!("{label} changed during verification"),
            path.display().to_string(),
        ));
    }
    Ok(HashedBytes {
        bytes,
        byte_length: total,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn same_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    if !left.file_type().is_file()
        || !right.file_type().is_file()
        || left.len() != right.len()
        || left.modified().ok() != right.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

async fn canonical_root_file(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<PathBuf, CutError> {
    let path = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|error| artifact_error(format!("canonicalize {label}"), error.to_string()))?;
    if !path.starts_with(root) {
        return Err(artifact_error(
            format!("{label} escapes the Motion handoff root"),
            path.display().to_string(),
        ));
    }
    let canonical_relative = path
        .strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"));
    if canonical_relative.as_deref() != Some(relative) {
        return Err(artifact_error(
            format!("{label} path is not canonical"),
            relative.to_string(),
        ));
    }
    if !path.is_file() {
        return Err(artifact_error(
            format!("{label} is not a file"),
            path.display().to_string(),
        ));
    }
    Ok(path)
}

async fn canonical_existing_path(root: &Path, raw: &str, label: &str) -> Result<PathBuf, CutError> {
    let requested = PathBuf::from(raw);
    let requested = if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    };
    let path = tokio::fs::canonicalize(&requested)
        .await
        .map_err(|error| artifact_error(format!("canonicalize {label}"), error.to_string()))?;
    if !path.starts_with(root) {
        return Err(artifact_error(
            format!("{label} escapes the Motion handoff root"),
            path.display().to_string(),
        ));
    }
    Ok(path)
}

fn required_canonical_relative(value: &Value, field: &str) -> Result<String, CutError> {
    let raw = required_string(value, field)?;
    let path = Path::new(&raw);
    if raw.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(artifact_error(
            format!("Motion artifact {field} is not a canonical root-relative path"),
            raw,
        ));
    }
    Ok(raw)
}

fn required_sha256(value: &Value, field: &str) -> Result<String, CutError> {
    let hash = required_string(value, field)?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(artifact_error(
            format!("Motion artifact {field} is not a lowercase sha256"),
            hash,
        ));
    }
    Ok(hash)
}

fn require_artifact_id(value: &str, label: &str) -> Result<(), CutError> {
    let suffix = value.strip_prefix("artifact-").unwrap_or_default();
    if suffix.len() != 24
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(artifact_error(
            format!("{label} is invalid"),
            value.to_string(),
        ));
    }
    Ok(())
}

fn validate_media_magic(bytes: &[u8], media_type: &str) -> Result<(), CutError> {
    let valid = match media_type {
        "video/mp4" | "video/quicktime" => bytes.get(4..8) == Some(b"ftyp"),
        "video/webm" => bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        _ => false,
    };
    if !valid {
        return Err(artifact_error(
            "Motion artifact bytes do not match mediaType",
            media_type.to_string(),
        ));
    }
    Ok(())
}

fn object_field<'a>(value: &'a Value, field: &str) -> Result<&'a Value, CutError> {
    value
        .get(field)
        .filter(|entry| entry.is_object())
        .ok_or_else(|| {
            artifact_error(
                format!("Motion artifact is missing object '{field}'"),
                "expected an object",
            )
        })
}

fn required_string(value: &Value, field: &str) -> Result<String, CutError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            artifact_error(
                format!("Motion artifact is missing '{field}'"),
                "expected a non-empty string",
            )
        })
}

fn require_string(value: &Value, field: &str, expected: &str) -> Result<(), CutError> {
    let actual = required_string(value, field)?;
    if actual != expected {
        return Err(artifact_error(
            format!("Motion artifact field '{field}' does not match"),
            format!("expected '{expected}', got '{actual}'"),
        ));
    }
    Ok(())
}

fn required_success_status(value: &Value, field: &str) -> Result<String, CutError> {
    let status = required_string(value, field)?;
    if status != "passed" && status != "warning" {
        return Err(artifact_error(
            "Motion artifact receipt is not successful",
            status,
        ));
    }
    Ok(status)
}

fn artifact_error(message: impl Into<String>, cause: impl Into<String>) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, cause)
        .with_suggested_action("Re-render the Motion handoff and use its unchanged artifact handle")
}
