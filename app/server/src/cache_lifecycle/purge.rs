//! Preview admission, atomic plan consumption, and exact deletion job.

use super::inventory::scan;
use super::ownership::{cache_root, identity, key, read_ledger, write_ledger};
use super::*;
use crate::jobs::{begin_current_process_worker, current_job_cancellation};
use crate::state::AppState;
use cut_core::{error_codes, CutError, ProjectStore};

mod admission;
mod reconciliation;
#[cfg(test)]
mod reconciliation_tests;
#[cfg(test)]
mod tests;
pub(crate) use admission::{preview, start_purge};
#[cfg(test)]
pub(super) use reconciliation::derived_after;
use reconciliation::post_measurement;
pub(super) use reconciliation::{
    cancel_with_reconciliation, fail_after_removed, fail_with_reconciliation,
    reconciliation_result, ReconciliationStatus,
};

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

pub(super) async fn execute_purge(state: AppState, job_id: String, plan: CachePurgePlan) {
    let _process_guard = begin_current_process_worker();
    let cancellation = current_job_cancellation();
    if let Some(reason) = cancellation.reason() {
        // The plan already contains a strict before measurement. Before the
        // worker has the lifecycle lease we may retain a fresh strict zero-
        // removal reconciliation, but never derive one from a stale plan.
        cancel_with_reconciliation(&state, &job_id, reason, &plan, 0, 0, false).await;
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
        if let Some(reason) = cancellation.reason() {
            // A project transition can take the store between acquiring the
            // lease and validation. Preserve its cancellation terminal state.
            cancel_with_reconciliation(&state, &job_id, reason, &plan, 0, 0, false).await;
        } else {
            state.jobs.fail(&job_id, error);
        }
        return;
    }
    if let Some(reason) = cancellation.reason() {
        cancel_with_reconciliation(&state, &job_id, reason, &plan, 0, 0, true).await;
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
            cancel_with_reconciliation(
                &state,
                &job_id,
                reason,
                &plan,
                deleted_files,
                deleted_bytes,
                true,
            )
            .await;
            return;
        }
        let root = match cache_root(&plan.project_dir, target.kind) {
            Ok(root) => root,
            Err(error) => {
                fail_with_reconciliation(
                    &state,
                    &job_id,
                    &plan,
                    deleted_files,
                    deleted_bytes,
                    error,
                )
                .await;
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
            fail_with_reconciliation(
                &state,
                &job_id,
                &plan,
                deleted_files,
                deleted_bytes,
                cache_error(
                    "cache purge plan is no longer valid",
                    "a cache root changed after the preview",
                ),
            )
            .await;
            return;
        }
        let path = root.join(&target.name);
        match identity(target.kind, target.name.clone(), &path) {
            Ok(current) if current == *target => {}
            Ok(_) => {
                fail_with_reconciliation(
                    &state,
                    &job_id,
                    &plan,
                    deleted_files,
                    deleted_bytes,
                    cache_error(
                        "cache purge plan is no longer valid",
                        "a cache entry changed after the preview",
                    ),
                )
                .await;
                return;
            }
            Err(error) => {
                fail_with_reconciliation(
                    &state,
                    &job_id,
                    &plan,
                    deleted_files,
                    deleted_bytes,
                    error,
                )
                .await;
                return;
            }
        }
        if std::fs::remove_file(&path).is_err() {
            fail_with_reconciliation(
                &state,
                &job_id,
                &plan,
                deleted_files,
                deleted_bytes,
                cache_error(
                    "cache purge could not remove a cache entry",
                    "the planned cache file could not be removed",
                ),
            )
            .await;
            return;
        }
        let Some(entry_key) = key(target.kind, &target.name) else {
            fail_after_removed(
                &state,
                &job_id,
                &plan,
                deleted_files.saturating_add(1),
                deleted_bytes.saturating_add(target.bytes),
                cache_error(
                    "cache purge plan is no longer valid",
                    "a cache filename is not valid text after it was removed",
                ),
            );
            return;
        };
        if ledger.entries.remove(&entry_key).is_none() {
            fail_after_removed(
                &state,
                &job_id,
                &plan,
                deleted_files.saturating_add(1),
                deleted_bytes.saturating_add(target.bytes),
                cache_error(
                    "cache purge plan is no longer valid",
                    "an ownership record disappeared after the cache file was removed",
                ),
            );
            return;
        }
        if let Err(error) = write_ledger(&plan.project_dir, &ledger) {
            fail_after_removed(
                &state,
                &job_id,
                &plan,
                deleted_files.saturating_add(1),
                deleted_bytes.saturating_add(target.bytes),
                error,
            );
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
    // Cancellation can race the final deletion and the after-scan. Check it
    // once more so a project transition cannot flatten an otherwise exact
    // partial/full delta into a generic no-project failure.
    if let Some(reason) = cancellation.reason() {
        cancel_with_reconciliation(
            &state,
            &job_id,
            reason,
            &plan,
            deleted_files,
            deleted_bytes,
            true,
        )
        .await;
        return;
    }
    let after = match post_measurement(&state, &plan).await {
        Ok(after) => after,
        Err(error) => {
            if let Some(reason) = cancellation.reason() {
                cancel_with_reconciliation(
                    &state,
                    &job_id,
                    reason,
                    &plan,
                    deleted_files,
                    deleted_bytes,
                    true,
                )
                .await;
            } else {
                fail_with_reconciliation(
                    &state,
                    &job_id,
                    &plan,
                    deleted_files,
                    deleted_bytes,
                    error,
                )
                .await;
            }
            return;
        }
    };
    state.jobs.finish(
        &job_id,
        reconciliation_result(
            &plan,
            deleted_files,
            deleted_bytes,
            after,
            ReconciliationStatus::Completed,
            true,
            false,
        ),
    );
}
