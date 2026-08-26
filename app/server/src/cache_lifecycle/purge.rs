//! Preview admission, atomic plan consumption, and exact deletion job.

use super::inventory::{build_preview, scan};
use super::ownership::{cache_root, identity, key, read_ledger, write_ledger};
use super::*;
use crate::jobs::{begin_current_process_worker, current_job_cancellation};
use crate::state::AppState;
use cut_core::{error_codes, CutError, ProjectStore, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

pub(crate) async fn preview(state: &AppState) -> Result<VerbResult, CutError> {
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

#[derive(Deserialize)]
struct PurgeArgs {
    plan_id: String,
    confirm: bool,
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
    let plan = {
        let mut stored = state.cache_purge_plan.lock().await;
        match stored.take() {
            Some(plan) if plan.plan_id == args.plan_id => plan,
            Some(plan) => {
                *stored = Some(plan);
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
        }
    };
    let job = state.jobs.create("cache_purge");
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

fn revalidate_plan(store: &ProjectStore, plan: &CachePurgePlan) -> Result<(), CutError> {
    if now_ms()?.saturating_sub(plan.created_ms) > 5 * 60 * 1_000 {
        return Err(cache_error(
            "cache purge plan is no longer valid",
            "the preview is older than five minutes",
        ));
    }
    if store.dir != plan.project_dir {
        return Err(cache_error(
            "cache purge plan is no longer valid",
            "the open project changed",
        ));
    }
    let (revision, _) = store.log.current_revision_and_count()?;
    if revision != plan.project_revision {
        return Err(cache_error(
            "cache purge plan is no longer valid",
            "the project journal changed after the preview",
        ));
    }
    let ledger = read_ledger(&store.dir)?;
    let (snapshot, roots, _targets, _counts) = scan(store, &ledger, now_ms()?)?;
    if roots != plan.roots || snapshot != plan.snapshot {
        return Err(cache_error(
            "cache purge plan is no longer valid",
            "the cache roots changed after the preview",
        ));
    }
    Ok(())
}

async fn execute_purge(state: AppState, job_id: String, plan: CachePurgePlan) {
    let _process_guard = begin_current_process_worker();
    let cancellation = current_job_cancellation();
    if let Some(reason) = cancellation.reason() {
        state.jobs.cancel_from_worker(&job_id, reason);
        return;
    }
    let _lease = match state.cache_lifecycle_lease.try_write() {
        Ok(lease) => lease,
        Err(_) => {
            state.jobs.fail(&job_id, cache_busy_error());
            return;
        }
    };
    state
        .jobs
        .progress(&job_id, 0.05, Some("revalidating cache ownership".into()));
    let validation = {
        let project = state.project.read().await;
        let Some(store) = project.as_ref() else {
            state.jobs.fail(
                &job_id,
                CutError::new(
                    error_codes::NOT_FOUND,
                    "no project open",
                    "the project closed before cache cleanup started",
                ),
            );
            return;
        };
        revalidate_plan(store, &plan)
    };
    if let Err(error) = validation {
        state.jobs.fail(&job_id, error);
        return;
    }
    let mut ledger = match read_ledger(&plan.project_dir) {
        Ok(ledger) => ledger,
        Err(error) => {
            state.jobs.fail(&job_id, error);
            return;
        }
    };
    let total = plan.targets.len().max(1) as f32;
    let mut deleted_files = 0u64;
    let mut deleted_bytes = 0u64;
    for (index, target) in plan.targets.iter().enumerate() {
        if let Some(reason) = cancellation.reason() {
            state.jobs.cancel_from_worker(&job_id, reason);
            return;
        }
        let root = match cache_root(&plan.project_dir, target.kind) {
            Ok(root) => root,
            Err(error) => {
                state.jobs.fail(&job_id, error);
                return;
            }
        };
        let expected_root = plan.roots.iter().find(|root| root.kind == target.kind);
        let current_root = std::fs::canonicalize(&root).map_err(|_| {
            cache_error(
                "cache purge plan is no longer valid",
                "a cache root changed after the preview",
            )
        });
        if current_root.as_ref().ok() != expected_root.map(|expected| &expected.canonical) {
            state.jobs.fail(
                &job_id,
                cache_error(
                    "cache purge plan is no longer valid",
                    "a cache root changed after the preview",
                ),
            );
            return;
        }
        let path = root.join(&target.name);
        match identity(target.kind, target.name.clone(), &path) {
            Ok(current) if current == *target => {}
            Ok(_) => {
                state.jobs.fail(
                    &job_id,
                    cache_error(
                        "cache purge plan is no longer valid",
                        "a cache entry changed after the preview",
                    ),
                );
                return;
            }
            Err(error) => {
                state.jobs.fail(&job_id, error);
                return;
            }
        }
        if std::fs::remove_file(&path).is_err() {
            state.jobs.fail(
                &job_id,
                cache_error(
                    "cache purge could not remove a cache entry",
                    "the planned cache file could not be removed",
                ),
            );
            return;
        }
        let Some(entry_key) = key(target.kind, &target.name) else {
            state.jobs.fail(
                &job_id,
                cache_error(
                    "cache purge plan is no longer valid",
                    "a cache filename is not valid text",
                ),
            );
            return;
        };
        if ledger.entries.remove(&entry_key).is_none() {
            state.jobs.fail(
                &job_id,
                cache_error(
                    "cache purge plan is no longer valid",
                    "an ownership record disappeared after the preview",
                ),
            );
            return;
        }
        if let Err(error) = write_ledger(&plan.project_dir, &ledger) {
            state.jobs.fail(&job_id, error);
            return;
        }
        deleted_files = deleted_files.saturating_add(1);
        deleted_bytes = deleted_bytes.saturating_add(target.bytes);
        state.jobs.progress(
            &job_id,
            0.10 + (index.saturating_add(1) as f32 / total) * 0.85,
            Some("removing confirmed rebuildable cache files".into()),
        );
    }
    state.jobs.finish(
        &job_id,
        json!({
            "deleted_files": deleted_files,
            "deleted_bytes": deleted_bytes,
            "retention_minimum_age_ms": CACHE_RETENTION_MS,
            "note": "only ledger-owned, unreferenced, aged proxy and filmstrip files were eligible"
        }),
    );
}
