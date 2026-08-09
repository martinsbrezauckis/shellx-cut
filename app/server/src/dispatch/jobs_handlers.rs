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
        Some(rec) => Ok(VerbResult::ok(serde_json::to_value(rec)?)),
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
    Ok(VerbResult::ok(json!({
        "jobs": jobs,
        "persistence_notices": state.jobs.persistence_notices(),
    })))
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
