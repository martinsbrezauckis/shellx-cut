use super::binding::text;
use super::delivery::contains_direct_authority;
use super::ConsumerError;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MotionJobEvent {
    pub(crate) sequence: u64,
    pub(crate) event_type: String,
    pub(crate) at_ms: u64,
    pub(crate) data: Option<Value>,
}

pub(super) fn project_event(value: Value) -> Result<MotionJobEvent, ConsumerError> {
    let event = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion job event must be an object"))?;
    if event
        .keys()
        .any(|key| !["schema", "seq", "atMs", "type", "data"].contains(&key.as_str()))
    {
        return Err(ConsumerError::refusal(
            "Motion job event has an unknown field",
        ));
    }
    if text(event, "schema", 96)? != "shellx-motion/job-event@1" {
        return Err(ConsumerError::refusal(
            "Motion job event schema is unsupported",
        ));
    }
    let sequence = event
        .get("seq")
        .and_then(Value::as_u64)
        .filter(|item| *item > 0)
        .ok_or_else(|| ConsumerError::refusal("Motion job event sequence is invalid"))?;
    let at_ms = event
        .get("atMs")
        .and_then(Value::as_u64)
        .ok_or_else(|| ConsumerError::refusal("Motion job event timestamp is invalid"))?;
    let event_type = text(event, "type", 96)?;
    if !matches!(
        event_type.as_str(),
        "submitted"
            | "running"
            | "cancel_requested"
            | "succeeded"
            | "failed"
            | "cancelled"
            | "retry_submitted"
    ) {
        return Err(ConsumerError::refusal("Motion job event type is invalid"));
    }
    let data = event.get("data").cloned();
    if data
        .as_ref()
        .is_some_and(|value| !value.is_object() || contains_direct_authority(value))
    {
        return Err(ConsumerError::refusal(
            "Motion job event contains a direct path, URL, or command authority",
        ));
    }
    Ok(MotionJobEvent {
        sequence,
        event_type,
        at_ms,
        data,
    })
}
