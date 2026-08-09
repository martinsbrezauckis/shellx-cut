//! error.rs — `RecordError`, mirroring ShellX Cut's `CutError` shape.
//!
//! Field-compatible with Cut's `CutError` on the core fields — `{code, message,
//! cause, suggested_action}`. Cut's optional clip/time context (`clip_id`, `at_ms`)
//! is added at the cutd conversion layer rather than carried here, so RecordError
//! stays small (it's returned by value in every `Result`). `cause` is REQUIRED
//! (the actionable "why"); `suggested_action` is the recommended next step.

use serde::{Deserialize, Serialize};

/// Stable error codes (mirror of Cut's `error_codes`, recorder-specific additions).
pub mod error_codes {
    pub const NOT_FOUND: &str = "not_found";
    pub const INVALID_ARGS: &str = "invalid_args";
    pub const IO: &str = "io";
    pub const FFMPEG: &str = "ffmpeg";
    pub const CAPTURE: &str = "capture";
    pub const UNIMPLEMENTED: &str = "unimplemented";
    pub const GUARDRAIL: &str = "guardrail";
}

/// The actionable recorder error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message} (cause: {cause})")]
pub struct RecordError {
    pub code: String,
    pub message: String,
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
}

impl RecordError {
    pub fn new(code: &str, message: impl Into<String>, cause: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            cause: cause.into(),
            suggested_action: None,
        }
    }

    /// Attach a recommended next step.
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_action = Some(action.into());
        self
    }
}

impl From<std::io::Error> for RecordError {
    fn from(e: std::io::Error) -> Self {
        RecordError::new(error_codes::IO, "io error", e.to_string())
    }
}

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, RecordError>;
