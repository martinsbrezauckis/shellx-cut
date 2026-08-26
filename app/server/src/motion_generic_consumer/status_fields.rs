use super::delivery::contains_direct_authority;
use super::{ConsumerError, MotionFailure};
use serde_json::{Map, Value};

pub(super) const REQUIRED_FIELDS: [&str; 11] = [
    "schema",
    "jobId",
    "callerId",
    "lane",
    "operation",
    "lifecycle",
    "outcome",
    "state",
    "createdAtMs",
    "cancelRequested",
    "warnings",
];
pub(super) const OPTIONAL_FIELDS: [&str; 12] = [
    "frameLane",
    "startedAtMs",
    "endedAtMs",
    "durationMs",
    "queueWaitMs",
    "pid",
    "error",
    "cancellation",
    "skip",
    "receiptPath",
    "receiptId",
    "producerEvidence",
];
pub(super) const TERMINAL_OPTIONAL_FIELDS: [&str; 2] = ["lineage", "pollAfterMs"];

pub(super) fn validate_shape(job: &Map<String, Value>) -> Result<(), ConsumerError> {
    if job.keys().any(|key| {
        !REQUIRED_FIELDS.contains(&key.as_str())
            && !OPTIONAL_FIELDS.contains(&key.as_str())
            && !TERMINAL_OPTIONAL_FIELDS.contains(&key.as_str())
    }) || REQUIRED_FIELDS
        .iter()
        .any(|field| !job.contains_key(*field))
    {
        return Err(ConsumerError::refusal(
            "Motion job status has unknown or missing fields",
        ));
    }
    if let Some(frame_lane) = job.get("frameLane") {
        token(frame_lane, 64, "Motion frame lane")?;
    }
    if let Some(pid) = job.get("pid") {
        integer_value(pid, "Motion job pid", 1)?;
    }
    if let Some(value) = job.get("producerEvidence") {
        bounded_opaque_object(value, "Motion producer evidence")?;
    }
    Ok(())
}
pub(super) fn cancel_requested(job: &Map<String, Value>) -> Result<bool, ConsumerError> {
    let value = job
        .get("cancelRequested")
        .ok_or_else(|| ConsumerError::refusal("Motion job has no cancellation state"))?;
    if value.is_null() {
        return Ok(false);
    }
    let value = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion cancelRequested must be object or null"))?;
    exact_optional(
        value,
        &["requestedBy", "reason", "requestedAtMs"],
        &["requestedBy", "requestedAtMs"],
        "Motion cancelRequested",
    )?;
    bounded_text(value, "requestedBy", 256, "Motion cancellation requester")?;
    if value.contains_key("reason") {
        bounded_text(value, "reason", 512, "Motion cancellation reason")?;
    }
    timestamp(value, "requestedAtMs")?;
    Ok(true)
}
pub(super) fn cancellation(value: &Value) -> Result<(), ConsumerError> {
    let value = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion cancelled job has no cancellation"))?;
    exact_optional(
        value,
        &["requestedBy", "reason"],
        &["requestedBy"],
        "Motion cancellation",
    )?;
    bounded_text(value, "requestedBy", 256, "Motion cancellation requester")?;
    if value.contains_key("reason") {
        bounded_text(value, "reason", 512, "Motion cancellation reason")?;
    }
    Ok(())
}
pub(super) fn skip(value: &Value) -> Result<(), ConsumerError> {
    let value = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion skipped job has no skip data"))?;
    exact_optional(value, &["code", "reason"], &["code"], "Motion skip")?;
    if !matches!(
        bounded_text(value, "code", 64, "Motion skip code")?.as_str(),
        "already_satisfied" | "precondition_unmet" | "batch_halted" | "dependency_failed"
    ) {
        return Err(ConsumerError::refusal("Motion skip code is unsupported"));
    }
    if value.contains_key("reason") {
        bounded_text(value, "reason", 512, "Motion skip reason")?;
    }
    Ok(())
}
pub(super) fn failure(value: &Value) -> Result<MotionFailure, ConsumerError> {
    let value = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion failed job has no typed error"))?;
    exact_optional(
        value,
        &[
            "code",
            "message",
            "retryable",
            "remedy",
            "retryAfterMs",
            "suggestedAction",
        ],
        &["code", "message", "retryable"],
        "Motion typed failure",
    )?;
    let code = bounded_text(value, "code", 96, "Motion error code")?;
    if !code.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !code.bytes().all(|item| {
            item.is_ascii_lowercase()
                || item.is_ascii_digit()
                || matches!(item, b'.' | b'_' | b':' | b'-')
        })
    {
        return Err(ConsumerError::refusal(
            "Motion error code is outside the bounded opaque contract",
        ));
    }
    Ok(MotionFailure {
        code,
        message: bounded_text(value, "message", 4096, "Motion error message")?,
        retryable: value
            .get("retryable")
            .and_then(Value::as_bool)
            .ok_or_else(|| ConsumerError::refusal("Motion typed failure has no retryable flag"))?,
        remedy: optional_text(value, "remedy", 64, "Motion error remedy")?,
        retry_after_ms: value
            .get("retryAfterMs")
            .map(|item| integer_value(item, "Motion retryAfterMs", 1))
            .transpose()?,
        suggested_action: optional_text(value, "suggestedAction", 1024, "Motion suggested action")?,
    })
}
pub(super) fn warnings(job: &Map<String, Value>) -> Result<(), ConsumerError> {
    let values = job
        .get("warnings")
        .and_then(Value::as_array)
        .ok_or_else(|| ConsumerError::refusal("Motion warnings must be an array"))?;
    if values.len() > 64
        || values.iter().any(|item| {
            item.as_str()
                .is_none_or(|item| item.is_empty() || item.len() > 1024)
        })
    {
        Err(ConsumerError::refusal("Motion warnings are outside bounds"))
    } else {
        Ok(())
    }
}
pub(super) fn lineage(job: &Map<String, Value>) -> Result<(), ConsumerError> {
    let Some(value) = job.get("lineage") else {
        return Ok(());
    };
    let value = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion lineage must be an object"))?;
    exact_optional(
        value,
        &["priorJobId", "priorReceiptId", "retryAttempt"],
        &[],
        "Motion lineage",
    )?;
    if value.contains_key("priorJobId") {
        token(value.get("priorJobId").unwrap(), 128, "Motion prior job id")?;
    }
    if value.contains_key("priorReceiptId") {
        token(
            value.get("priorReceiptId").unwrap(),
            128,
            "Motion prior receipt id",
        )?;
    }
    if value.contains_key("retryAttempt") {
        integer_value(
            value.get("retryAttempt").unwrap(),
            "Motion retry attempt",
            1,
        )?;
    }
    Ok(())
}
pub(super) fn private_receipt_path(value: &Value) -> Result<(), ConsumerError> {
    let value = value
        .as_str()
        .filter(|item| {
            !item.is_empty() && item.len() <= 4096 && !item.chars().any(char::is_control)
        })
        .ok_or_else(|| ConsumerError::refusal("Motion private receipt path is invalid"))?;
    let absolute = value.starts_with('/')
        || (value.len() >= 3
            && value.as_bytes()[0].is_ascii_alphabetic()
            && value.as_bytes()[1] == b':'
            && matches!(value.as_bytes()[2], b'/' | b'\\'));
    if !absolute || value.split(['/', '\\']).any(|segment| segment == "..") {
        Err(ConsumerError::refusal(
            "Motion private receipt path is not an absolute controlled transport path",
        ))
    } else {
        Ok(())
    }
}
pub(super) fn absent(value: &Map<String, Value>, keys: &[&str]) -> Result<(), ConsumerError> {
    if keys.iter().any(|key| value.contains_key(*key)) {
        Err(ConsumerError::refusal(
            "Motion job carries a field forbidden for this lifecycle",
        ))
    } else {
        Ok(())
    }
}
pub(super) fn null(value: &Map<String, Value>, key: &str) -> Result<(), ConsumerError> {
    if value.get(key).is_some_and(Value::is_null) {
        Ok(())
    } else {
        Err(ConsumerError::refusal(
            "Motion job lifecycle has a non-null forbidden value",
        ))
    }
}
pub(super) fn poll_after(value: &Map<String, Value>) -> Result<u64, ConsumerError> {
    integer(value, "pollAfterMs", 1)
}
pub(super) fn timestamp(value: &Map<String, Value>, key: &str) -> Result<u64, ConsumerError> {
    integer(value, key, 1)
}
pub(super) fn integer(
    value: &Map<String, Value>,
    key: &str,
    minimum: u64,
) -> Result<u64, ConsumerError> {
    integer_value(value.get(key).unwrap_or(&Value::Null), key, minimum)
}
fn integer_value(value: &Value, label: &str, minimum: u64) -> Result<u64, ConsumerError> {
    value
        .as_u64()
        .filter(|item| *item >= minimum)
        .ok_or_else(|| ConsumerError::refusal(format!("{label} is outside bounds")))
}
pub(super) fn text(
    value: &Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<String, ConsumerError> {
    bounded_text(value, key, maximum, "Motion status")
}
fn bounded_text(
    value: &Map<String, Value>,
    key: &str,
    maximum: usize,
    label: &str,
) -> Result<String, ConsumerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|item| {
            !item.is_empty() && item.len() <= maximum && !item.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            ConsumerError::refusal(format!("{label} {key} must be a bounded non-empty string"))
        })
}
fn optional_text(
    value: &Map<String, Value>,
    key: &str,
    maximum: usize,
    label: &str,
) -> Result<Option<String>, ConsumerError> {
    value
        .get(key)
        .map(|_| bounded_text(value, key, maximum, label))
        .transpose()
}
pub(super) fn optional_token(
    value: &Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<Option<String>, ConsumerError> {
    value
        .get(key)
        .map(|item| token(item, maximum, "Motion receipt id"))
        .transpose()
}
fn token(value: &Value, maximum: usize, label: &str) -> Result<String, ConsumerError> {
    value
        .as_str()
        .filter(|item| {
            !item.is_empty()
                && item.len() <= maximum
                && item.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                })
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            ConsumerError::refusal(format!("{label} is outside the opaque token contract"))
        })
}
fn exact_optional(
    value: &Map<String, Value>,
    allowed: &[&str],
    required: &[&str],
    label: &str,
) -> Result<(), ConsumerError> {
    if value.keys().any(|key| !allowed.contains(&key.as_str()))
        || required.iter().any(|key| !value.contains_key(*key))
    {
        Err(ConsumerError::refusal(format!(
            "{label} has unknown or missing fields"
        )))
    } else {
        Ok(())
    }
}
fn bounded_opaque_object(value: &Value, label: &str) -> Result<(), ConsumerError> {
    let value = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal(format!("{label} must be an object")))?;
    if value.len() > 16 || contains_direct_authority(&Value::Object(value.clone())) {
        Err(ConsumerError::refusal(format!("{label} is unsafe")))
    } else {
        Ok(())
    }
}
