use super::adapter::{AcceptedMotionDelivery, BoundMotionJob};
use super::ConsumerError;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Validates the private, authenticated delivery resolution.  This seam is
/// intentionally distinct from `job-status@1`: Motion status contains only
/// lifecycle and private receipt references, never artifacts or import plans.
pub(super) fn validate_delivery(
    job: &BoundMotionJob,
    value: Value,
) -> Result<AcceptedMotionDelivery, ConsumerError> {
    let delivery = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion connector delivery must be an object"))?;
    exact_fields(
        delivery,
        &[
            "schema",
            "jobId",
            "binding",
            "artifacts",
            "receipts",
            "importPlan",
        ],
        "Motion connector delivery",
    )?;
    exact_text(
        delivery,
        "schema",
        "shellx-cut/motion-connector-delivery@1",
        96,
    )?;
    if text(delivery, "jobId", 128)? != job.job_id {
        return Err(ConsumerError::refusal(
            "Motion connector delivery did not bind the terminal job id",
        ));
    }
    validate_binding(delivery.get("binding").unwrap_or(&Value::Null), job)?;
    let artifacts = delivery
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ConsumerError::refusal("Motion connector delivery has no artifacts array")
        })?;
    let receipts = delivery
        .get("receipts")
        .and_then(Value::as_array)
        .ok_or_else(|| ConsumerError::refusal("Motion connector delivery has no receipts array"))?;
    let mut seen = BTreeSet::new();
    let mut projected_artifacts = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| ConsumerError::refusal("Motion artifact must be an object"))?;
        exact_fields(
            artifact,
            &["role", "schema", "mediaKind"],
            "Motion artifact",
        )?;
        let role = text(artifact, "role", 64)?;
        if !matches!(role.as_str(), "artifact_handle" | "rendered_media")
            || !seen.insert(role.clone())
        {
            return Err(ConsumerError::refusal(
                "Motion delivery has an unexpected or duplicate artifact role",
            ));
        }
        let class = job.prepared.outputs.get(&role).ok_or_else(|| {
            ConsumerError::refusal("Motion artifact role was not advertised by the descriptor")
        })?;
        if text(artifact, "mediaKind", 128)? != class.media_kind
            || text(artifact, "schema", 192)? != class.schema
            || contains_direct_authority(&Value::Object(artifact.clone()))
        {
            return Err(ConsumerError::refusal(
                "Motion artifact does not match the safe descriptor output class",
            ));
        }
        projected_artifacts.push(Value::Object(artifact.clone()));
    }
    let mut projected_receipts = Vec::with_capacity(receipts.len());
    for receipt in receipts {
        let receipt = receipt
            .as_object()
            .ok_or_else(|| ConsumerError::refusal("Motion receipt must be an object"))?;
        exact_fields(receipt, &["role", "schema", "status"], "Motion receipt")?;
        let role = text(receipt, "role", 64)?;
        if role != "receipt"
            || !seen.insert(role)
            || text(receipt, "schema", 96)? != "shellx-motion/receipt@1"
            || !matches!(text(receipt, "status", 16)?.as_str(), "passed" | "warning")
            || contains_direct_authority(&Value::Object(receipt.clone()))
        {
            return Err(ConsumerError::refusal(
                "Motion receipt is not one safe advertised attestation",
            ));
        }
        projected_receipts.push(Value::Object(receipt.clone()));
    }
    if !job.prepared.outputs.contains_key("cut_import_plan")
        || !seen.insert("cut_import_plan".to_owned())
    {
        return Err(ConsumerError::refusal(
            "Motion delivery has no single advertised Cut import-plan role",
        ));
    }
    let import_plan = delivery.get("importPlan").ok_or_else(|| {
        ConsumerError::refusal("Motion connector delivery has no Cut import plan")
    })?;
    validate_import_plan(import_plan)?;
    if seen != job.prepared.outputs.keys().cloned().collect() {
        return Err(ConsumerError::refusal(
            "Motion delivery is missing or adds a descriptor output role",
        ));
    }
    Ok(AcceptedMotionDelivery {
        artifacts: projected_artifacts,
        receipts: projected_receipts,
        import_plan: import_plan.clone(),
    })
}

fn validate_binding(value: &Value, job: &BoundMotionJob) -> Result<(), ConsumerError> {
    let binding = value.as_object().ok_or_else(|| {
        ConsumerError::refusal("Motion connector delivery has no immutable binding")
    })?;
    exact_fields(
        binding,
        &[
            "capabilityId",
            "descriptorRevision",
            "descriptorFingerprint",
            "requestSchemaId",
            "catalogFingerprint",
            "bindingFingerprint",
        ],
        "Motion connector delivery binding",
    )?;
    if text(binding, "capabilityId", 128)? != job.prepared.capability_id
        || binding.get("descriptorRevision").and_then(Value::as_u64)
            != Some(job.prepared.descriptor_revision)
        || text(binding, "descriptorFingerprint", 64)? != job.prepared.descriptor_fingerprint
        || text(binding, "requestSchemaId", 192)? != job.prepared.request_schema_id
        || text(binding, "catalogFingerprint", 64)? != job.prepared.catalog_fingerprint
        || text(binding, "bindingFingerprint", 64)? != job.binding_fingerprint
    {
        return Err(ConsumerError::refusal(
            "Motion connector delivery binding drifted from the submitted request",
        ));
    }
    Ok(())
}

fn validate_import_plan(value: &Value) -> Result<(), ConsumerError> {
    let plan = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion Cut import plan must be an object"))?;
    exact_fields(
        plan,
        &["schema", "ok", "mode", "operations", "receipt"],
        "Motion Cut import plan",
    )?;
    if text(plan, "schema", 96)? != "shellx-motion/cut-import-plan@1"
        || plan.get("ok").and_then(Value::as_bool) != Some(true)
        || text(plan, "mode", 32)? != "rendered_media"
    {
        return Err(ConsumerError::refusal(
            "Motion Cut import plan editor-operation class is unsupported",
        ));
    }
    let operations = plan
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| ConsumerError::refusal("Motion Cut import plan has no operations"))?;
    if operations.is_empty() || operations.len() > 10_000 || contains_direct_authority(value) {
        return Err(ConsumerError::refusal(
            "Motion Cut import plan is not a bounded opaque rendered-media plan",
        ));
    }
    for operation in operations {
        let operation = operation.as_object().ok_or_else(|| {
            ConsumerError::refusal("Motion Cut import plan operation must be an object")
        })?;
        exact_fields(
            operation,
            &["verb", "renderedMedia"],
            "Motion Cut import-plan operation",
        )?;
        if text(operation, "verb", 64)? != "cut.media.import_rendered" {
            return Err(ConsumerError::refusal(
                "Motion Cut import plan contains an unsupported editor operation",
            ));
        }
        let rendered = operation
            .get("renderedMedia")
            .and_then(Value::as_object)
            .ok_or_else(|| ConsumerError::refusal("Motion rendered-media operation is invalid"))?;
        exact_fields(
            rendered,
            &["dryRun", "handle"],
            "Motion rendered-media operation",
        )?;
        if rendered.get("dryRun").and_then(Value::as_bool) != Some(false) {
            return Err(ConsumerError::refusal(
                "Motion rendered-media operation must not be a dry run",
            ));
        }
        let handle = rendered
            .get("handle")
            .and_then(Value::as_object)
            .ok_or_else(|| ConsumerError::refusal("Motion rendered-media handle is invalid"))?;
        exact_fields(handle, &["schema"], "Motion rendered-media handle")?;
        if text(handle, "schema", 192)? != "shellx-motion/artifact-handle-ref@1" {
            return Err(ConsumerError::refusal(
                "Motion rendered-media handle has an unsupported schema",
            ));
        }
    }
    let receipt = plan
        .get("receipt")
        .and_then(Value::as_object)
        .ok_or_else(|| ConsumerError::refusal("Motion Cut import plan has no receipt"))?;
    exact_fields(
        receipt,
        &["schema", "status"],
        "Motion Cut import-plan receipt",
    )?;
    if text(receipt, "schema", 96)? != "shellx-motion/receipt@1"
        || !matches!(text(receipt, "status", 16)?.as_str(), "passed" | "warning")
    {
        return Err(ConsumerError::refusal(
            "Motion Cut import plan receipt is not successful",
        ));
    }
    Ok(())
}

pub(super) fn contains_direct_authority(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_direct_authority),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| unsafe_authority_key(key) || contains_direct_authority(value)),
        _ => false,
    }
}

fn unsafe_authority_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "path",
        "url",
        "uri",
        "root",
        "directory",
        "argv",
        "command",
        "executable",
        "callback",
    ]
    .iter()
    .any(|suffix| key == *suffix || key.ends_with(suffix))
}

fn exact_fields(
    value: &Map<String, Value>,
    fields: &[&str],
    label: &str,
) -> Result<(), ConsumerError> {
    if value.len() != fields.len()
        || fields.iter().any(|field| !value.contains_key(*field))
        || value.keys().any(|key| !fields.contains(&key.as_str()))
    {
        return Err(ConsumerError::refusal(format!(
            "{label} has unknown or missing fields"
        )));
    }
    Ok(())
}

fn text(value: &Map<String, Value>, field: &str, maximum: usize) -> Result<String, ConsumerError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= maximum)
        .map(str::to_owned)
        .ok_or_else(|| {
            ConsumerError::refusal(format!(
                "Motion delivery {field} must be a bounded non-empty string"
            ))
        })
}

fn exact_text(
    value: &Map<String, Value>,
    field: &str,
    expected: &str,
    maximum: usize,
) -> Result<(), ConsumerError> {
    if text(value, field, maximum)? == expected {
        Ok(())
    } else {
        Err(ConsumerError::refusal(format!(
            "Motion delivery {field} is unsupported"
        )))
    }
}
