use super::*;

// ---------------------------------------------------------------------------
// jobs.* handlers (the background-job contract)
// ---------------------------------------------------------------------------

/// jobs.status{job_id} — job record lookup.
pub(super) async fn jobs_status(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        job_id: String,
    }
    let a: Args = parse_args(args)?;
    match state.jobs.get(&a.job_id) {
        Some(rec) => Ok(VerbResult::ok(public_job_value(rec)?)),
        None => Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no job '{}'", a.job_id),
            "job ids come from media.import/transcribe/perception/render.final/verify.judge results",
        )
        .with_suggested_action("call jobs.list to see all jobs of this run")),
    }
}

/// jobs.list{} — every job record of this server run, newest first.
pub(super) async fn jobs_list(state: &AppState) -> Result<VerbResult, CutError> {
    let mut jobs = state.jobs.list();
    jobs.sort_by(|a, b| b.created_ts.cmp(&a.created_ts));
    let jobs = jobs
        .into_iter()
        .map(public_job_value)
        .collect::<Result<Vec<_>, CutError>>()?;
    Ok(VerbResult::ok(json!({
        "jobs": jobs,
        "persistence_notices": state.jobs.persistence_notices(),
    })))
}

/// `retry.descriptor` is an engine-only persisted recipe. API clients receive
/// eligibility and lineage, never project input paths or content fingerprints.
fn public_job_value(record: crate::jobs::JobRecord) -> Result<Value, CutError> {
    let mut value = serde_json::to_value(record)?;
    if let Some(retry) = value.get_mut("retry").and_then(Value::as_object_mut) {
        retry.remove("descriptor");
    }
    Ok(value)
}

/// jobs.cancel{job_id} — abort an active background task from this server run.
pub(super) async fn jobs_cancel(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        job_id: String,
    }
    let a: Args = parse_args(args)?;
    if state.jobs.get(&a.job_id).is_none() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no job '{}'", a.job_id),
            "job ids come from job-returning verbs and can be listed with jobs.list",
        ));
    }
    if !state.jobs.abort(&a.job_id).await? {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!("job '{}' is not active", a.job_id),
            "only queued/running tasks created in this server run can be cancelled",
        )
        .with_suggested_action("call jobs.status to inspect the current state"));
    }
    Ok(VerbResult::ok(json!({
        "job_id": a.job_id,
        "cancelled": true,
    })))
}

/// jobs.retry{job_id} — replay exactly one engine-owned safe descriptor.
pub(super) async fn jobs_retry(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        job_id: String,
    }
    let a: Args = parse_args(args)?;
    let record = state.jobs.get(&a.job_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no job '{}'", a.job_id),
            "select a job returned by jobs.list",
        )
    })?;
    match record.kind.as_str() {
        "screen_record_export" => crate::screen_record::retry_screen_record_export(state, &a.job_id).await,
        "verify-rerun" => crate::dispatch::verify_handlers::retry_verify_rerun(state, &a.job_id).await,
        _ => Err(CutError::new(
            error_codes::CONFLICT,
            format!("job '{}' has no retry implementation", a.job_id),
            "only screen-record exports and receipt-bound output verification have typed retry owners",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::public_job_value;
    use crate::events::EventBus;
    use crate::jobs::{
        JobManager, JobRetry, ScreenRecordExportRetryDescriptor, ScreenRecordExportRetryFormat,
    };

    #[test]
    fn status_projection_keeps_retry_recipe_engine_private() {
        let jobs = JobManager::new(EventBus::new());
        let record = jobs.create_with_retry(
            "screen_record_export",
            Some(JobRetry::screen_record_export(
                ScreenRecordExportRetryDescriptor {
                    project_revision: "op_000001".into(),
                    source: "cache/screen_record/capture/source.mp4".into(),
                    plan: "plans/edit.json".into(),
                    format: ScreenRecordExportRetryFormat::Mp4,
                    inputs: Vec::new(),
                    system_audio_offset_ms: 0,
                },
            )),
        );
        let value = public_job_value(record).unwrap();
        let retry = value["retry"].as_object().unwrap();
        assert!(retry.contains_key("eligible"));
        assert!(!retry.contains_key("descriptor"));
    }
}
