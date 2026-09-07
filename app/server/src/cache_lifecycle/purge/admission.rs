//! Project-bound cache purge preview and job admission.
//!
//! The transition lock spans plan publication and worker registration. This
//! keeps the opaque in-memory plan and durable job table in the same project
//! lifetime, leaving deletion and reconciliation to the worker owner.

use super::super::inventory::build_preview;
use super::super::{cache_busy_error, cache_error};
use super::execute_purge;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

#[derive(Deserialize)]
struct PurgeArgs {
    plan_id: String,
    confirm: bool,
}

pub(crate) async fn preview(state: &AppState) -> Result<VerbResult, CutError> {
    // An opaque plan belongs to one project lifetime even though it is kept in
    // memory. Never publish an old project's plan after replacement commits.
    let _transition = state.project_transition.lock().await;
    let _lease = state
        .cache_lifecycle_lease
        .try_write()
        .map_err(|_| cache_busy_error())?;
    let plan_id = format!(
        "cache_plan_{}",
        state
            .cache_purge_plan_seq
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    );
    let preview = {
        let project = state.project.read().await;
        let store = project.as_ref().ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "no project open",
                "open a project before previewing its editing cache",
            )
        })?;
        build_preview(store, plan_id)?
    };
    *state.cache_purge_plan.lock().await = preview.plan;
    Ok(VerbResult::ok(preview.public))
}

pub(crate) async fn start_purge(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: PurgeArgs = serde_json::from_value(args).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "invalid cache purge confirmation",
            "send the plan_id returned by project.cache_preview and confirm: true",
        )
    })?;
    if !args.confirm {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "cache purge requires confirmation",
            "send confirm: true only after reviewing the current preview",
        ));
    }
    // Register the worker before project replacement may start. Otherwise a
    // switch can drain the old table between plan consumption and spawn.
    let _transition = state.project_transition.lock().await;
    let (plan, job) = {
        let mut stored = state.cache_purge_plan.lock().await;
        match stored.as_ref() {
            Some(plan) if plan.plan_id == args.plan_id => {}
            Some(_) => {
                return Err(cache_error(
                    "cache purge plan is not current",
                    "preview the cache again and confirm the returned plan",
                ));
            }
            None => {
                return Err(cache_error(
                    "cache purge plan is not current",
                    "preview the cache again before confirming deletion",
                ))
            }
        };
        // The plan stays intact until its queued record is durable. A storage
        // failure therefore cannot consume a confirmed deletion request.
        let job = state.jobs.create_durable("cache_purge")?;
        let plan = stored
            .take()
            .expect("validated cache purge plan remains held by this transition");
        (plan, job)
    };
    let job_id = job.job_id.clone();
    let job_state = state.clone();
    let jid = job_id.clone();
    state.jobs.spawn(
        &job_id,
        async move { execute_purge(job_state, jid, plan).await },
    );
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "status": "queued"}),
    ))
}
