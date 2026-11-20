//! jobs.before_quit / jobs.interrupted / jobs.interrupted_dismiss: what quitting
//! would cancel, stopping it cleanly, and telling about work a quit without
//! asking (a crash, a kill, a power cut) cut off.

use super::*;

/// One running job as the person reads it in a question or a refusal.
fn listed(job: &crate::jobs::ActiveJobInfo) -> String {
    format!(
        "{} ({}, {:.0}% done{})",
        job.job_id,
        job.kind,
        job.progress * 100.0,
        job.message
            .as_deref()
            .map(|m| format!(", {m}"))
            .unwrap_or_default()
    )
}

/// One running job as a record for the quit question.
fn running_record(job: &crate::jobs::ActiveJobInfo) -> Value {
    json!({
        "job_id": job.job_id,
        "kind": job.kind,
        "progress": job.progress,
        "message": job.message,
    })
}

/// jobs.before_quit{cancel_running_jobs?} — the desktop app asks this before
/// it closes. Work someone asked for (the same rule as a project change,
/// `jobs::consent`) refuses with `jobs_running`, naming it; with
/// `cancel_running_jobs:true` every running job is cancelled the normal way
/// (its staging and lock files are removed) and the call returns once they
/// have stopped.
pub(super) async fn jobs_before_quit(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        #[serde(default)]
        cancel_running_jobs: bool,
    }
    let a: Args = parse_args(args)?;
    let active = state.jobs.active_jobs();
    let asked_for: Vec<&crate::jobs::ActiveJobInfo> =
        active.iter().filter(|job| job.is_delivery()).collect();
    if !a.cancel_running_jobs {
        if asked_for.is_empty() {
            return Ok(VerbResult::ok(
                json!({ "running": [], "cancelled_jobs": [] }),
            ));
        }
        let names = asked_for
            .iter()
            .map(|job| listed(job))
            .collect::<Vec<_>>()
            .join("; ");
        let error = CutError::new(
            "jobs_running",
            format!(
                "quitting ShellX Cut would cancel {} running job(s) that were asked for (exports, renders, reviews, analysis, downloads) and nothing has been stopped",
                asked_for.len()
            ),
            format!("running: {names}"),
        )
        .with_suggested_action(
            "wait for them to finish, or cancel them with jobs.cancel, or repeat the call with cancel_running_jobs:true to stop them cleanly before the app closes",
        );
        // The same jobs as records, one per job: a job's own message is free
        // text and may hold ';' or ',' itself, so a question built from the
        // cause line could show part of a sentence as a second job.
        let running: Vec<Value> = asked_for.iter().map(|job| running_record(job)).collect();
        return Ok(VerbResult {
            ok: false,
            result: Some(json!({ "running": running })),
            op_ids: None,
            warnings: None,
            project_revision: None,
            error: Some(error),
        });
    }
    // Cancel every job at once: each waits for its own workers to stop (the
    // export removes its staging and lock files), so the wait is the slowest
    // job's, not their sum.
    let waits: Vec<_> = active
        .iter()
        .cloned()
        .map(|job| {
            let state = state.clone();
            tokio::spawn(async move {
                let stopped = if job.kind == "render_queue" {
                    state.jobs.abort_render_queue(&job.job_id).await
                } else {
                    state.jobs.abort(&job.job_id).await
                };
                (job, stopped)
            })
        })
        .collect();
    let mut outcomes = Vec::with_capacity(waits.len());
    for wait in waits {
        match wait.await {
            Ok(outcome) => outcomes.push(outcome),
            Err(error) => tracing::warn!(%error, "a job cancel task ended early"),
        }
    }
    let mut cancelled = Vec::new();
    let mut still_stopping = Vec::new();
    for (job, outcome) in outcomes {
        match outcome {
            Ok(true) => cancelled.push(job),
            Ok(false) => {}
            Err(_) => still_stopping.push(listed(&job)),
        }
    }
    if !still_stopping.is_empty() {
        return Err(CutError::new(
            "job_cancel_pending",
            format!(
                "{} job(s) were cancelled but are still stopping",
                still_stopping.len()
            ),
            format!("still stopping: {}", still_stopping.join("; ")),
        )
        .with_suggested_action(
            "repeat the call to wait for them; if the app closes now, the next start says which work was interrupted and removes its leftover files",
        ));
    }
    Ok(VerbResult::ok(
        json!({ "running": [], "cancelled_jobs": cancelled }),
    ))
}

fn project_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".cutproj").to_string())
        .unwrap_or_else(|| dir.display().to_string())
}

/// jobs.interrupted{} — jobs someone asked for whose engine ended while they
/// ran. Needs no open project. A helper of that dead run that still runs is
/// stopped first, then the export's unfinished files are removed from every
/// folder it could write to (the project's `exports`, the app-wide export
/// folder); both are counted. Skipped while this engine runs work itself.
pub(super) async fn jobs_interrupted(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {}
    let _: Args = parse_args(args)?;
    // Stopping a surviving helper waits up to a couple of seconds.
    let may_tidy = state.jobs.active_jobs().is_empty();
    let notes = tokio::task::spawn_blocking(move || {
        crate::jobs::interrupted::interrupted_for_listing(may_tidy)
    })
    .await
    .map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not read interrupted work",
            error.to_string(),
        )
    })?;
    let items: Vec<Value> = notes
        .into_iter()
        .map(|(key, note)| {
            let project = project_name(&note.project_dir);
            json!({
                "key": key,
                "job_id": note.job_id,
                "kind": note.kind,
                "project_path": note.project_dir.display().to_string(),
                "project_name": project,
                "started_unix_ms": note.started_unix_ms,
                "leftover_files_removed": note.leftover_files_removed,
                "helpers_stopped": note.helpers_stopped,
                "export_folders": note.export_dirs.iter().map(|dir| dir.display().to_string()).collect::<Vec<_>>(),
                "message": format!(
                    "ShellX Cut stopped while {} ({}) was running in “{project}”, so it did not finish.",
                    note.kind, note.job_id
                ),
                "suggested_action": "start it again; dismiss this notice with jobs.interrupted_dismiss",
            })
        })
        .collect();
    Ok(VerbResult::ok(json!({ "interrupted": items })))
}

/// jobs.interrupted_dismiss{key} — forget one interrupted-job notice.
pub(super) async fn jobs_interrupted_dismiss(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        key: String,
    }
    let a: Args = parse_args(args)?;
    if !crate::jobs::interrupted::dismiss(&a.key) {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "there is no interrupted-job notice with this key",
            format!("no interrupted job for key {}", a.key),
        )
        .with_suggested_action("list current notices with jobs.interrupted"));
    }
    Ok(VerbResult::ok(json!({ "dismissed": true, "key": a.key })))
}

#[cfg(test)]
#[path = "jobs_quit_tests.rs"]
mod tests;
