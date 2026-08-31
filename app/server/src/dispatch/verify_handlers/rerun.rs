//! `verify.rerun` — repeat output-only checks on one receipt-bound artifact.
//!
//! A rerun never re-encodes or rewrites the source RenderReceipt. Its separate
//! receipt documents a fresh instrument pass over the exact output bytes.

use super::*;
use std::path::PathBuf;

#[path = "rerun_execution.rs"]
mod rerun_execution;
#[path = "rerun_output.rs"]
mod rerun_output;
#[path = "rerun_preparation.rs"]
mod rerun_preparation;
#[path = "rerun_receipt.rs"]
mod rerun_receipt;
use rerun_execution::spawn_verify_rerun_job;
#[cfg(test)]
use rerun_output::fenced_output_for_receipt;
#[cfg(test)]
use rerun_preparation::retry_descriptor;
use rerun_preparation::{
    prepare_verify_rerun, retry_conflict, retry_descriptor_async, retry_revalidation_error,
    verify_retry_descriptor_matches,
};
use rerun_receipt::{
    assert_receipt_hash, duration_matches_receipt, profile_from_receipt,
    selected_render_receipt_with_path, verification_receipt_name, write_verification_receipt,
};
#[cfg(test)]
use rerun_receipt::{full_sha256, selected_render_receipt};

#[derive(serde::Deserialize)]
struct Args {
    render_id: String,
}

/// Fully revalidated, output-only verification work. This is deliberately
/// distinct from historical verb arguments: it binds a named immutable render
/// receipt and its exact output bytes, then only creates a fresh verification
/// receipt for the attempt that owns it.
#[derive(Clone)]
struct PreparedVerifyRerun {
    project_dir: PathBuf,
    receipts: PathBuf,
    receipt_path: PathBuf,
    receipt: cut_core::RenderReceipt,
    output: PathBuf,
    profile: cut_perception::FootageProfile,
    project_revision: String,
}

/// Re-run only checks that inspect the named render's already-produced bytes.
/// Timeline/source checks remain the original RenderReceipt's responsibility.
pub(crate) async fn verify_rerun(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: Args = parse_args(args)?;
    let prepared = prepare_verify_rerun(state, &args.render_id).await?;
    let descriptor = retry_descriptor_async(&prepared).await?;
    let job = state.jobs.create_with_retry(
        "verify-rerun",
        Some(crate::jobs::JobRetry::verify_rerun(descriptor)),
    );
    let job_id = job.job_id.clone();
    let render_id = prepared.receipt.render_id.clone();
    let output_hash = prepared.receipt.output_hash.clone();
    spawn_verify_rerun_job(state, &job_id, prepared);
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "render_id": render_id,
        "output_hash": output_hash,
    })))
}

/// Re-admit a failed output-only verification after proving that the current
/// active project, source receipt, and rendered bytes still match its durable
/// typed descriptor. This never calls `verify.rerun` with historical args.
pub(crate) async fn retry_verify_rerun(
    state: &AppState,
    source_job_id: &str,
) -> Result<VerbResult, CutError> {
    let record = state.jobs.get(source_job_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no job '{source_job_id}'"),
            "select a job returned by jobs.list",
        )
    })?;
    if record.kind != "verify-rerun" {
        return Err(retry_conflict(
            "only verify.rerun jobs have this retry implementation",
        ));
    }
    let descriptor = match record.retry.and_then(|retry| retry.descriptor) {
        Some(crate::jobs::JobRetryDescriptor::VerifyRerun(descriptor)) => descriptor,
        _ => {
            return Err(retry_conflict(
                "this verification job predates retry support or has no durable descriptor",
            ))
        }
    };
    let prepared = prepare_verify_rerun(state, &descriptor.render_id)
        .await
        .map_err(retry_revalidation_error)?;
    let current = retry_descriptor_async(&prepared)
        .await
        .map_err(retry_revalidation_error)?;
    verify_retry_descriptor_matches(&descriptor, &current)?;

    let child = state.jobs.admit_retry(source_job_id)?;
    let job_id = child.job_id.clone();
    let retry = child.retry.as_ref().expect("retry child has projection");
    let attempt = retry.attempt;
    let root_job_id = retry.root_job_id.clone();
    let render_id = prepared.receipt.render_id.clone();
    let output_hash = prepared.receipt.output_hash.clone();
    spawn_verify_rerun_job(state, &job_id, prepared);
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "retry_of": source_job_id,
        "root_job_id": root_job_id,
        "attempt": attempt,
        "render_id": render_id,
        "output_hash": output_hash,
        "status": "queued",
    })))
}

#[cfg(test)]
#[path = "rerun_tests.rs"]
mod tests;
