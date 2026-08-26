use super::adapter::{BoundMotionJob, JobProjection};
use super::status_fields::*;
use super::ConsumerError;
use serde_json::{Map, Value};

pub(super) fn project_job(
    expected: &BoundMotionJob,
    value: Value,
) -> Result<JobProjection, ConsumerError> {
    let job = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("Motion job response must be an object"))?;
    validate_shape(job)?;
    if text(job, "schema", 96)? != "shellx-motion/job-status@1"
        || text(job, "jobId", 128)? != expected.job_id
        || text(job, "callerId", 256)? != expected.caller_id
        || text(job, "lane", 64)? != "connector"
        || text(job, "operation", 128)? != expected.prepared.capability_id
    {
        return Err(ConsumerError::refusal(
            "Motion job status does not bind the requested connector identity",
        ));
    }
    timestamp(job, "createdAtMs")?;
    warnings(job)?;
    let lifecycle = text(job, "lifecycle", 16)?;
    let state = text(job, "state", 16)?;
    match lifecycle.as_str() {
        "pending" if state == "pending" => {
            null(job, "outcome")?;
            absent(
                job,
                &[
                    "startedAtMs",
                    "endedAtMs",
                    "durationMs",
                    "queueWaitMs",
                    "lineage",
                    "error",
                    "cancellation",
                    "skip",
                    "receiptPath",
                    "receiptId",
                    "producerEvidence",
                ],
            )?;
            Ok(JobProjection::Pending {
                job_id: expected.job_id.clone(),
                poll_after_ms: poll_after(job)?,
                cancel_requested: cancel_requested(job)?,
            })
        }
        "running" if state == "running" => {
            null(job, "outcome")?;
            timestamp(job, "startedAtMs")?;
            integer(job, "queueWaitMs", 0)?;
            absent(
                job,
                &[
                    "endedAtMs",
                    "durationMs",
                    "lineage",
                    "error",
                    "cancellation",
                    "skip",
                    "receiptPath",
                    "receiptId",
                    "producerEvidence",
                ],
            )?;
            Ok(JobProjection::Running {
                job_id: expected.job_id.clone(),
                poll_after_ms: poll_after(job)?,
                cancel_requested: cancel_requested(job)?,
            })
        }
        "ended" => terminal(expected, job, &state),
        _ => Err(ConsumerError::refusal(
            "Motion job lifecycle/state is unsupported; queued is never a state",
        )),
    }
}
fn terminal(
    expected: &BoundMotionJob,
    job: &Map<String, Value>,
    state: &str,
) -> Result<JobProjection, ConsumerError> {
    absent(job, &["pollAfterMs", "pid"])?;
    null(job, "cancelRequested")?;
    timestamp(job, "endedAtMs")?;
    integer(job, "durationMs", 0)?;
    integer(job, "queueWaitMs", 0)?;
    lineage(job)?;
    let outcome = text(job, "outcome", 16)?;
    if outcome != state {
        return Err(ConsumerError::refusal(
            "Motion terminal job state is not its outcome",
        ));
    }
    if let Some(path) = job.get("receiptPath") {
        private_receipt_path(path)?;
    }
    let receipt_id = optional_token(job, "receiptId", 128)?;
    match outcome.as_str() {
        "succeeded" => {
            absent(job, &["error", "cancellation", "skip"])?;
            Ok(JobProjection::Succeeded {
                job_id: expected.job_id.clone(),
                receipt_id,
                receipt_available: job.contains_key("receiptPath"),
            })
        }
        "failed" => {
            absent(job, &["cancellation", "skip"])?;
            Ok(JobProjection::Failed {
                job_id: expected.job_id.clone(),
                error: failure(job.get("error").unwrap_or(&Value::Null))?,
            })
        }
        "cancelled" => {
            absent(job, &["error", "skip"])?;
            cancellation(job.get("cancellation").unwrap_or(&Value::Null))?;
            Ok(JobProjection::Cancelled {
                job_id: expected.job_id.clone(),
            })
        }
        "skipped" => {
            absent(job, &["error", "cancellation", "startedAtMs"])?;
            skip(job.get("skip").unwrap_or(&Value::Null))?;
            Ok(JobProjection::Skipped {
                job_id: expected.job_id.clone(),
            })
        }
        _ => Err(ConsumerError::refusal("Motion job outcome is unsupported")),
    }
}
