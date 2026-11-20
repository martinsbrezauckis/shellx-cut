//! error.rs — actionable error type + the universal verb envelope (public verb contract).
//!
//! Role: every verb returns `VerbResult` — `{ok, result?, op_ids?, error?}`.
//! Errors MUST be actionable: code + message + clip/timecode context + cause
//! (errors contract: "errors that tell the agent what to do next").
//! Dependencies: serde, serde_json, thiserror. Primary callers: all crates;
//! the server serializes VerbResult verbatim as the REST/MCP response body.

use serde::{Deserialize, Serialize};

/// Stable machine-readable error codes. String-typed in JSON; this enum-like
/// module keeps the canon in one place so agents can match on them.
pub mod codes {
    pub const NOT_FOUND: &str = "not_found";
    pub const INVALID_ARGS: &str = "invalid_args";
    pub const CONFLICT: &str = "conflict";
    pub const NO_PROJECT: &str = "no_project";
    pub const IO: &str = "io";
    pub const FFMPEG: &str = "ffmpeg";
    pub const SIDECAR: &str = "sidecar";
    /// A required external tool (ffmpeg or ffprobe) is not installed where
    /// Cut looks. The step was refused before it changed anything; the
    /// suggested_action says how to get the tool (system.fetch_tool where a
    /// built-in download exists, otherwise the platform's install steps or
    /// system.set_ffmpeg).
    pub const TOOL_MISSING: &str = "tool_missing";
    /// The file exists but is not a video, audio or image file that Cut can
    /// read (a renamed text file, an empty file, a damaged file). Nothing was
    /// added to the project; the suggested_action says how to get a usable copy.
    pub const UNREADABLE_MEDIA: &str = "unreadable_media";
    /// The project's frame size is odd (an older project), and a video export
    /// at the project size needs even sides. Refused before anything runs;
    /// the suggested_action is the exact `project.format` call to fix it.
    pub const ODD_PROJECT_SIZE: &str = "odd_project_size";
    /// The request is valid but this platform has no built-in way to do it
    /// (e.g. no built-in FFmpeg download for this OS/architecture).
    pub const UNSUPPORTED_PLATFORM: &str = "unsupported_platform";
    /// ShellX Motion stopped a render at the caller's or user's request. This
    /// is terminal and must never be treated as a retryable render failure.
    pub const RENDER_CANCELLED: &str = "render_cancelled";
    /// ShellX Motion could not admit the work before its machine-wide queue
    /// deadline. The render never started and may be retried later.
    pub const JOB_QUEUE_TIMEOUT: &str = "job_queue_timeout";
    /// No Motion job exists for the supplied id in the active workspace scope.
    pub const JOB_UNKNOWN: &str = "job_unknown";
    /// The Motion job existed, but its terminal record has left retention.
    pub const JOB_EXPIRED: &str = "job_expired";
    /// The Motion job exists but belongs to another caller scope.
    pub const JOB_NOT_VISIBLE: &str = "job_not_visible";
    pub const JOB_FAILED: &str = "job_failed";
    pub const UNIMPLEMENTED: &str = "unimplemented";
    pub const NO_UI_CLIENT: &str = "no_ui_client";
    /// A safety guard refused an operation that is probably a mistake (e.g.
    /// transcript.remove_silences deleting >80% of the timeline — regression
    /// the totality guard: on fully-silent footage it removed 99.4%). The error's
    /// suggested_action names the explicit override arg to proceed anyway.
    pub const GUARDRAIL: &str = "guardrail";
    /// A render could not get the memory it needs: the system stopped ffmpeg
    /// (out-of-memory kill) or an allocation failed, or the render plan needs
    /// more memory than this computer has. The suggested_action lists the
    /// ways to make the render lighter.
    pub const OUT_OF_MEMORY: &str = "out_of_memory";
    /// The output drive has less free space than the render needs, or it
    /// filled up while writing. The suggested_action says how much to free.
    pub const NOT_ENOUGH_SPACE: &str = "not_enough_space";
    /// A render's ffmpeg stopped working: for the stall time it wrote no new
    /// output and used almost no processor time, so the engine ended it
    /// instead of letting the job read "running" for ever. The message names
    /// the part of the timeline (and the clip) it was making; the cause says
    /// whether it was held at its memory limit; the suggested_action says
    /// what to change before exporting again.
    pub const RENDER_STALLED: &str = "render_stalled";
}

/// The actionable error (public verb contract:
/// `error{code,message,clip_id?,at_ms?,cause,suggested_action?}`).
/// `cause` is required by the contract: the underlying reason in plain words
/// (e.g. the ffmpeg stderr tail, the missing path) so the agent can act.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message} (cause: {cause})")]
pub struct CutError {
    pub code: String,
    pub message: String,
    /// Clip the error pertains to, when known — lets the agent jump there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    /// Timeline position the error pertains to, when known (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
    /// Underlying cause in plain words. Required.
    pub cause: String,
    /// What the agent should do next, when a clear next step exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    /// The tool or condition this refusal is about (an id of
    /// schema/requirements.json). The server fills its name and where to get
    /// it from the registry before the answer leaves the engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Boxed so an error stays small inside the many enums that carry one.
    pub requirement: Option<Box<MissingRequirement>>,
}

/// The missing tool or condition named by a refusal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissingRequirement {
    /// Registry id, e.g. "ffmpeg", "captions_runtime", "open_project".
    pub id: String,
    /// Plain name, e.g. "Video processing (FFmpeg)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where to get it: the Settings entry, the button, the verb and what a
    /// person must do (`get` of the registry entry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub get: Option<serde_json::Value>,
}

impl CutError {
    /// Build an error with code + message + cause (the required trio).
    pub fn new(code: &str, message: impl Into<String>, cause: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            clip_id: None,
            at_ms: None,
            cause: cause.into(),
            suggested_action: None,
            requirement: None,
        }
    }

    /// Name the missing tool or condition (a schema/requirements.json id).
    pub fn with_requirement(mut self, id: &str) -> Self {
        self.requirement = Some(Box::new(MissingRequirement {
            id: id.to_string(),
            name: None,
            get: None,
        }));
        self
    }

    /// Attach a suggested next step (builder-style).
    pub fn with_suggested_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_action = Some(action.into());
        self
    }

    /// Attach clip context (builder-style).
    pub fn with_clip(mut self, clip_id: impl Into<String>) -> Self {
        self.clip_id = Some(clip_id.into());
        self
    }

    /// Attach timeline-position context (builder-style).
    pub fn with_at_ms(mut self, at_ms: u64) -> Self {
        self.at_ms = Some(at_ms);
        self
    }

    /// Shorthand for a capability declared by a compatibility surface but absent
    /// from this build.
    pub fn unimplemented(what: &str) -> Self {
        Self::new(
            codes::UNIMPLEMENTED,
            format!("{what} is not available in this build"),
            "the requested capability is unavailable in this build",
        )
    }
}

impl From<std::io::Error> for CutError {
    /// IO errors map to code "io" with the OS error as cause.
    fn from(e: std::io::Error) -> Self {
        CutError::new(codes::IO, "I/O operation failed", e.to_string())
    }
}

impl From<serde_json::Error> for CutError {
    /// JSON (de)serialization errors map to "invalid_args" — almost always
    /// malformed verb args or a corrupt project file; the message says which.
    fn from(e: serde_json::Error) -> Self {
        CutError::new(
            codes::INVALID_ARGS,
            "JSON (de)serialization failed",
            e.to_string(),
        )
    }
}

/// One non-fatal guardrail finding, carried IN-BAND on the envelope and never
/// logs-only. Example: preflight fps/resolution mismatch that was
/// auto-conformed (warn-and-proceed); integrity violations hard-error instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerbWarning {
    pub code: String,
    pub message: String,
    /// Finding-specific context keys (measured values, what was conformed…).
    #[serde(flatten)]
    pub detail: serde_json::Map<String, serde_json::Value>,
}

/// The universal verb envelope (public verb contract): every REST/MCP/CLI verb
/// returns exactly this shape. `op_ids` lists ops appended by the verb
/// (mutations); read-only verbs return `result` only; `warnings` carries
/// non-fatal guardrail findings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerbResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<VerbWarning>>,
    /// Latest durable project operation after a successful mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CutError>,
}

impl VerbResult {
    /// Success with a result payload (read-only verbs).
    pub fn ok(result: serde_json::Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            op_ids: None,
            warnings: None,
            project_revision: None,
            error: None,
        }
    }

    /// Success with payload + the op ids the verb appended (mutating verbs).
    pub fn ok_with_ops(result: serde_json::Value, op_ids: Vec<String>) -> Self {
        Self {
            ok: true,
            result: Some(result),
            op_ids: Some(op_ids),
            warnings: None,
            project_revision: None,
            error: None,
        }
    }

    /// Attach warnings to an existing envelope (builder-style).
    pub fn with_warnings(mut self, warnings: Vec<VerbWarning>) -> Self {
        if !warnings.is_empty() {
            self.warnings.get_or_insert_with(Vec::new).extend(warnings);
        }
        self
    }

    pub fn with_project_revision(mut self, project_revision: Option<String>) -> Self {
        self.project_revision = project_revision;
        self
    }

    /// Failure envelope.
    pub fn err(error: CutError) -> Self {
        Self {
            ok: false,
            result: None,
            op_ids: None,
            warnings: None,
            project_revision: None,
            error: Some(error),
        }
    }
}

impl From<Result<VerbResult, CutError>> for VerbResult {
    /// Flatten `Result<VerbResult, CutError>` — lets handlers use `?` freely.
    fn from(r: Result<VerbResult, CutError>) -> Self {
        match r {
            Ok(v) => v,
            Err(e) => VerbResult::err(e),
        }
    }
}
