use super::canonical::canonical_json;
use super::contract::PreparedMotionRequest;
use super::ConsumerError;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub(super) fn validate_job_id(value: &str) -> Result<(), ConsumerError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|item| item.is_ascii_alphanumeric() || matches!(item, b'.' | b'_' | b':' | b'-'))
    {
        Err(ConsumerError::refusal(
            "Motion job id is outside the fixed connector-job alphabet",
        ))
    } else {
        Ok(())
    }
}
pub(super) fn bounded_caller_id(value: String) -> Result<String, ConsumerError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(ConsumerError::refusal(
            "Motion transport caller identity is outside bounds",
        ))
    } else {
        Ok(value)
    }
}
pub(super) fn binding_fingerprint(
    prepared: &PreparedMotionRequest,
    job_id: &str,
    caller_id: &str,
) -> String {
    let content = serde_json::json!({
        "schema": "shellx-motion/connector-job-binding@1", "jobId": job_id, "callerId": caller_id,
        "capabilityId": prepared.capability_id, "descriptorRevision": prepared.descriptor_revision,
        "descriptorFingerprint": prepared.descriptor_fingerprint, "requestSchemaId": prepared.request_schema_id,
        "catalogFingerprint": prepared.catalog_fingerprint, "request": prepared.request,
    });
    hex::encode(Sha256::digest(canonical_json(&content).as_bytes()))
}
pub(super) fn validate_submission(
    value: &Value,
    prepared: &PreparedMotionRequest,
    job_id: &str,
    binding_fingerprint: &str,
) -> Result<(), ConsumerError> {
    let result = value.as_object().ok_or_else(|| {
        ConsumerError::refusal("Motion connector submission must return an object")
    })?;
    exact_fields(
        result,
        &["ok", "jobId", "state", "lifecycle", "binding"],
        "Motion connector submission",
    )?;
    if result.get("ok").and_then(Value::as_bool) != Some(true)
        || text(result, "jobId", 128)? != job_id
        || text(result, "state", 16)? != "pending"
        || text(result, "lifecycle", 16)? != "pending"
    {
        return Err(ConsumerError::refusal(
            "Motion connector submission did not return a pending bound job",
        ));
    }
    let binding = result
        .get("binding")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ConsumerError::refusal("Motion connector submission has no immutable binding")
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
        "Motion connector submission binding",
    )?;
    if text(binding, "capabilityId", 128)? != prepared.capability_id
        || binding.get("descriptorRevision").and_then(Value::as_u64)
            != Some(prepared.descriptor_revision)
        || text(binding, "descriptorFingerprint", 64)? != prepared.descriptor_fingerprint
        || text(binding, "requestSchemaId", 192)? != prepared.request_schema_id
        || text(binding, "catalogFingerprint", 64)? != prepared.catalog_fingerprint
        || text(binding, "bindingFingerprint", 64)? != binding_fingerprint
    {
        return Err(ConsumerError::refusal(
            "Motion connector submission binding drifted from the discovered descriptor",
        ));
    }
    Ok(())
}
pub(super) fn text(
    value: &Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<String, ConsumerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty() && item.len() <= maximum)
        .map(str::to_owned)
        .ok_or_else(|| {
            ConsumerError::refusal(format!(
                "Motion response {key} must be a bounded non-empty string"
            ))
        })
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
        Err(ConsumerError::refusal(format!(
            "{label} has unknown or missing fields"
        )))
    } else {
        Ok(())
    }
}
