//! refusal.rs — the one way a refusal names a missing tool or condition.
//!
//! A refusal for something missing carries `error.requirement {id, name, get}`
//! read from schema/requirements.json, so the person and the agent see the
//! same plain name and the same way forward (the Settings entry, the button,
//! the verb, and what only a person can do). Crates below the server tag an
//! error with `CutError::with_requirement(id)`; [`complete`] fills the rest on
//! the way out of `dispatch` and of a failed job. Server code builds new
//! refusals with [`refusal`].

use super::registry;
use cut_core::{CutError, VerbResult};
use serde_json::Value;

/// A refusal because `id` (a requirement of the registry) is missing.
/// `message` and `cause` say what happened in plain words; the way forward is
/// the registry's, unless `action` overrides it for this case.
pub(crate) fn refusal(
    code: &str,
    id: &str,
    message: impl Into<String>,
    cause: impl Into<String>,
) -> CutError {
    complete(CutError::new(code, message, cause).with_requirement(id))
}

/// The way forward a requirement's `get` describes, in one sentence.
pub(crate) fn way_forward(get: &Value) -> Option<String> {
    let text = |key: &str| get.get(key).and_then(Value::as_str);
    let mut parts = Vec::new();
    if let Some(verb) = text("verb") {
        let args = get
            .get("args")
            .filter(|args| args.as_object().is_some_and(|map| !map.is_empty()))
            .map(|args| format!(" {args}"))
            .unwrap_or_default();
        parts.push(format!("call {verb}{args}"));
    }
    if let Some(button) = text("button") {
        let place = text("settings").map(settings_place).unwrap_or_default();
        parts.push(format!("or choose {button}{place}"));
    }
    if let Some(person) = text("person") {
        parts.push(format!("({person})"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Where a `ui.open` panel id is, in the words of the Settings window.
fn settings_place(panel: &str) -> String {
    let place = match panel {
        "settings-video-performance" => "Settings > Video & performance",
        "settings-ai-transcription" => "Settings > AI & transcription",
        "settings-services-integrations" => "Settings > Services & integrations",
        "settings-agent-control" => "Settings > Agent control",
        "settings-recording" => "Settings > Recording",
        "settings-storage-privacy" => "Settings > Storage & privacy",
        "settings-general" => "Settings > General",
        "projects" => "the Projects panel",
        "transcript" => "the Transcript panel",
        _ => return String::new(),
    };
    format!(" in {place}")
}

/// Fill the name and the way forward of a tagged refusal from the registry.
/// An error that already says what to do keeps its own suggested action.
pub(crate) fn complete(mut error: CutError) -> CutError {
    let Some(tag) = error.requirement.as_mut() else {
        return error;
    };
    let Some(item) = registry().requirement(&tag.id) else {
        return error;
    };
    if tag.name.is_none() {
        tag.name = Some(item.name.clone());
    }
    if tag.get.is_none() {
        tag.get = Some(item.get.clone());
    }
    if error.suggested_action.is_none() {
        error.suggested_action = way_forward(&item.get);
    }
    error
}

/// [`complete`] for a verb's answer.
pub(crate) fn complete_result(mut result: VerbResult) -> VerbResult {
    if let Some(error) = result.error.take() {
        result.error = Some(complete(error));
    }
    result
}
