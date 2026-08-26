//! Typed terminal failure projection for a capture that cannot publish a project.
//!
//! A failed pre-first-frame portal setup has no checkpoint, source, or
//! `project.json` to recover. The worker therefore publishes this small,
//! contained record before it releases its capture reservation. `screen_record.stop`
//! can return the original typed error instead of waiting for a project that is
//! intentionally absent.

use std::path::Path;

use cut_core::{error_codes, CutError};
use record_core::RecordError;
use serde::{Deserialize, Serialize};

const TERMINAL_FILE: &str = "capture.terminal.json";
const TERMINAL_SCHEMA: &str = "shellx-cut/screen-record-terminal/1";
const MAX_TERMINAL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureTerminal {
    schema: String,
    state: TerminalState,
    error: RecordError,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TerminalState {
    Failed,
}

pub(crate) fn publish_failure(
    project_dir: &Path,
    capture_id: &str,
    error: &RecordError,
) -> Result<(), CutError> {
    let terminal = CaptureTerminal {
        schema: TERMINAL_SCHEMA.into(),
        state: TerminalState::Failed,
        error: error.clone(),
    };
    validate(&terminal)?;
    let bytes = serde_json::to_vec_pretty(&terminal).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not serialize capture terminal failure",
            error.to_string(),
        )
    })?;
    record_recovery::CaptureRoot::for_project(project_dir)
        .and_then(|root| root.publish_new_capture_file(capture_id, TERMINAL_FILE, &bytes))
        .map(|_| ())
        .map_err(|error| projection_error("publish capture terminal failure", error))
}

pub(crate) fn read_failure(
    project_dir: &Path,
    capture_id: &str,
) -> Result<Option<RecordError>, CutError> {
    let Some(root) = record_recovery::CaptureRoot::open_existing(project_dir)
        .map_err(|error| projection_error("inspect capture terminal failure", error))?
    else {
        return Ok(None);
    };
    let path = root
        .capture_file(capture_id, TERMINAL_FILE)
        .map_err(|error| projection_error("resolve capture terminal failure", error))?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CutError::new(
                error_codes::IO,
                "could not inspect capture terminal failure",
                error.to_string(),
            ))
        }
        Ok(metadata) => metadata,
    };
    if !record_recovery::is_plain_regular_file(&path)
        .map_err(|error| projection_error("validate capture terminal failure", error))?
    {
        return Err(CutError::new(
            error_codes::IO,
            "capture terminal failure is not a local regular file",
            "terminal capture evidence cannot be a link, reparse point, or non-regular file",
        ));
    }
    if metadata.len() > MAX_TERMINAL_BYTES {
        return Err(CutError::new(
            error_codes::IO,
            "capture terminal failure is too large",
            format!(
                "{} has {} bytes (limit: {MAX_TERMINAL_BYTES})",
                path.display(),
                metadata.len()
            ),
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not read capture terminal failure",
            error.to_string(),
        )
    })?;
    let terminal: CaptureTerminal = serde_json::from_slice(&bytes).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "capture terminal failure is malformed",
            error.to_string(),
        )
    })?;
    validate(&terminal)?;
    Ok(Some(terminal.error))
}

fn validate(terminal: &CaptureTerminal) -> Result<(), CutError> {
    if terminal.schema != TERMINAL_SCHEMA || terminal.state != TerminalState::Failed {
        return Err(CutError::new(
            error_codes::IO,
            "capture terminal failure has an unsupported state",
            "terminal capture evidence must use the current failed schema",
        ));
    }
    if terminal.error.code.trim().is_empty()
        || terminal.error.message.trim().is_empty()
        || terminal.error.cause.trim().is_empty()
    {
        return Err(CutError::new(
            error_codes::IO,
            "capture terminal failure is incomplete",
            "terminal capture evidence requires typed code, message, and cause",
        ));
    }
    Ok(())
}

fn projection_error(stage: &str, error: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {stage}: {error}"),
        "the terminal capture failure must remain inside the local capture directory",
    )
}
