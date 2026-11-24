//! Truthful, path-free replies for a refused or half-finished no-project Start.

use super::RollbackOutcome;
use cut_core::CutError;
use serde_json::json;

pub(crate) fn refused_fallback_reply(
    error: CutError,
    outcome: RollbackOutcome,
) -> cut_core::VerbResult {
    tracing::warn!(error = %error, outcome = ?outcome, "no-project recording start refused");
    cut_core::VerbResult {
        ok: false,
        result: Some(outcome.as_reply_payload()),
        op_ids: None,
        warnings: None,
        project_revision: None,
        error: Some(path_free_error(&error)),
    }
}

/// Reply for a failure after the native capture already started: the take IS
/// running, so the reply carries its id and never claims the Start was refused.
pub(crate) fn running_take_failure_reply(
    error: CutError,
    capture_id: &str,
    duration_ms: Option<u64>,
) -> cut_core::VerbResult {
    tracing::warn!(error = %error, capture_id = %capture_id, "recording started but its project could not be activated");
    let mut payload = RollbackOutcome::Incomplete.as_reply_payload();
    payload["capture_id"] = json!(capture_id);
    payload["status"] = json!("recording");
    payload["take_running"] = json!(true);
    payload["duration_ms"] = json!(duration_ms);
    payload["open_ended"] = json!(duration_ms.is_none());
    cut_core::VerbResult {
        ok: false,
        result: Some(payload),
        op_ids: None,
        warnings: None,
        project_revision: None,
        error: Some(path_free_error(&error)),
    }
}

fn token_is_path_like(token: &str) -> bool {
    let t = token.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '`' | ',' | ';'));
    let bytes = t.as_bytes();
    t.contains('\\')
        || t.starts_with('/')
        || t.starts_with('~')
        || t.starts_with("./")
        || t.starts_with("../")
        || t.contains("=/")
        || t.contains(":/")
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn text_is_path_free(text: &str) -> bool {
    !text.split_whitespace().any(token_is_path_like)
}

/// Keep the error code; drop any text that names an absolute path (the full
/// error goes to the server log only) and keep a safe next step.
pub(crate) fn path_free_error(error: &CutError) -> CutError {
    let message = if text_is_path_free(&error.message) {
        error.message.clone()
    } else {
        "Recording did not start".to_string()
    };
    let cause = if text_is_path_free(&error.cause) {
        error.cause.clone()
    } else {
        "the details are in the Cut log".to_string()
    };
    let suggested_action = match error.suggested_action.as_deref() {
        Some(text) if text_is_path_free(text) => Some(text.to_string()),
        _ => Some("Check the recording settings and try again.".to_string()),
    };
    CutError {
        code: error.code.clone(),
        message,
        clip_id: error.clip_id.clone(),
        at_ms: error.at_ms,
        cause,
        suggested_action,
        requirement: error.requirement.clone(),
    }
}

/// Why a fallback project could not be created.
#[derive(Debug)]
pub(crate) enum CreateFailure {
    /// Refused before any project folder existed.
    NoFolder(CutError),
    /// The folder exists (kept, not listed, not opened); a later step failed.
    FolderLeft(CutError),
}

impl CreateFailure {
    pub(crate) fn into_reply(self) -> cut_core::VerbResult {
        match self {
            Self::NoFolder(e) => refused_fallback_reply(e, RollbackOutcome::NotCreated),
            Self::FolderLeft(e) => refused_fallback_reply(e, RollbackOutcome::Incomplete),
        }
    }
}
