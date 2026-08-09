//! Read-only ShellX Motion live-job adapter.
//!
//! Cut owns the caller scope and passes only fixed argv to Motion. Agents can
//! inspect work for the active project, but cannot supply another caller id or
//! request Motion's operator-only cross-caller scope.

use crate::dispatch::no_project;
use crate::motion_runtime::{build_motion_cli_command, run_motion_command_spec};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Deserialize;
use serde_json::{json, Map, Value};

const JOB_STATES: [&str; 6] = [
    "pending",
    "running",
    "succeeded",
    "failed",
    "cancelled",
    "skipped",
];

#[derive(Deserialize)]
struct GetArgs {
    job_id: String,
}

#[derive(Deserialize)]
struct ListArgs {
    limit: Option<u64>,
}

pub(crate) async fn get(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: GetArgs = serde_json::from_value(args).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.job.get arguments are invalid",
            error.to_string(),
        )
    })?;
    let project_dir = active_project_dir(state).await?;
    let spec = build_motion_cli_command(
        vec!["job".into(), "get".into(), args.job_id.clone()],
        &project_dir,
    );
    let envelope = run_motion_command_spec(spec, "read a Motion job").await?;
    let job = envelope.get("job").ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "ShellX Motion returned no job status",
            "the job.get success envelope had no job object",
        )
    })?;
    Ok(VerbResult::ok(json!({
        "schema": "shellx-cut/motion-job-query@1",
        "caller_scope": "active-project",
        "job": public_job(job)?,
    })))
}

pub(crate) async fn list(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: ListArgs = serde_json::from_value(args).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.job.list arguments are invalid",
            error.to_string(),
        )
    })?;
    let project_dir = active_project_dir(state).await?;
    let mut command = vec!["job".into(), "list".into()];
    if let Some(limit) = args.limit {
        command.extend(["--limit".into(), limit.to_string()]);
    }
    let spec = build_motion_cli_command(command, &project_dir);
    let envelope = run_motion_command_spec(spec, "list Motion jobs").await?;
    let jobs = envelope
        .get("jobs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "ShellX Motion returned no job list",
                "the job.list success envelope had no jobs array",
            )
        })?
        .iter()
        .map(public_job)
        .collect::<Result<Vec<_>, _>>()?;
    let in_flight_count = jobs
        .iter()
        .filter(|job| job.get("lifecycle").and_then(Value::as_str) != Some("ended"))
        .count();
    let mut state_counts = Map::new();
    for state in JOB_STATES {
        state_counts.insert(
            state.to_string(),
            json!(jobs
                .iter()
                .filter(|job| job.get("state").and_then(Value::as_str) == Some(state))
                .count()),
        );
    }
    Ok(VerbResult::ok(json!({
        "schema": "shellx-cut/motion-job-list@1",
        "caller_scope": "active-project",
        "job_count": jobs.len(),
        "in_flight_count": in_flight_count,
        "state_counts": state_counts,
        "jobs": jobs,
    })))
}

async fn active_project_dir(state: &AppState) -> Result<std::path::PathBuf, CutError> {
    let guard = state.project.read().await;
    Ok(guard.as_ref().ok_or_else(no_project)?.dir.clone())
}

/// Keep the authored status vocabulary while withholding process ids, caller
/// hashes, registry roots, and receipt paths from the public Cut response.
fn public_job(value: &Value) -> Result<Value, CutError> {
    let source = value
        .as_object()
        .ok_or_else(|| invalid_job("job was not an object"))?;
    let state = source
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| JOB_STATES.contains(state))
        .ok_or_else(|| invalid_job("job state was missing or unknown"))?;
    let lifecycle = source
        .get("lifecycle")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "pending" | "running" | "ended"))
        .ok_or_else(|| invalid_job("job lifecycle was missing or unknown"))?;
    let job_id = source
        .get("jobId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_job("jobId was missing"))?;

    let mut result = Map::new();
    result.insert("schema".into(), json!("shellx-motion/job-status@1"));
    result.insert("jobId".into(), json!(job_id));
    result.insert("lifecycle".into(), json!(lifecycle));
    result.insert("state".into(), json!(state));
    for key in [
        "outcome",
        "lane",
        "operation",
        "createdAtMs",
        "startedAtMs",
        "endedAtMs",
        "durationMs",
        "queueWaitMs",
        "error",
        "cancellation",
        "skip",
        "warnings",
        "pollAfterMs",
    ] {
        if let Some(value) = source.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    result.insert(
        "receiptAvailable".into(),
        json!(source.get("receiptPath").and_then(Value::as_str).is_some()),
    );
    Ok(Value::Object(result))
}

fn invalid_job(cause: &str) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        "ShellX Motion returned an invalid job status",
        cause,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_job_preserves_status_but_removes_private_runtime_fields() {
        let value = json!({
            "schema": "shellx-motion/job-status@1",
            "jobId": "cut:render-42",
            "callerId": "cut:secret-workspace-hash",
            "lifecycle": "running",
            "outcome": null,
            "state": "running",
            "lane": "ffmpeg",
            "operation": "render.final",
            "createdAtMs": 1,
            "startedAtMs": 2,
            "pid": 48122,
            "warnings": [],
            "pollAfterMs": 2000,
            "receiptPath": "/private/runtime/receipt.json"
        });
        let projected = public_job(&value).expect("valid job");
        assert_eq!(projected["state"], "running");
        assert_eq!(projected["pollAfterMs"], 2000);
        assert_eq!(projected["receiptAvailable"], true);
        assert!(projected.get("callerId").is_none());
        assert!(projected.get("pid").is_none());
        assert!(projected.get("receiptPath").is_none());
    }

    #[test]
    fn public_job_rejects_unknown_state_tokens() {
        let error = public_job(&json!({
            "jobId": "cut:render-42",
            "lifecycle": "running",
            "state": "queued"
        }))
        .unwrap_err();
        assert_eq!(error.code, error_codes::SIDECAR);
    }
}
