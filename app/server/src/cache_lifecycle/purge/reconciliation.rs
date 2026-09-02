//! Path-free accounting for a confirmed cache purge.
//!
//! This owns the distinction between a strict after-scan and the narrowly
//! permitted project-switch delta, keeping the destructive worker focused on
//! admission and per-file ledger transitions.

use super::super::inventory::scan;
use super::super::ownership::read_ledger;
use super::super::*;
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub(crate) enum ReconciliationStatus {
    Completed,
    Cancelled,
    Failed,
}

impl ReconciliationStatus {
    fn public(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// The exclusive lifecycle lease stays held from validation through this scan,
/// so `after` is the exact strict inventory immediately after this purge's
/// confirmed removals. It never needs filenames or arbitrary project paths.
pub(crate) async fn post_measurement(
    state: &AppState,
    plan: &CachePurgePlan,
) -> Result<CacheMeasurement, CutError> {
    let project = state.project.read().await;
    let store = project.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no project open",
            "the project closed before cache cleanup could be remeasured",
        )
    })?;
    if store.dir != plan.project_dir {
        return Err(cache_error(
            "cache purge plan is no longer valid",
            "the open project changed before cache cleanup could be remeasured",
        ));
    }
    let ledger = read_ledger(&plan.project_dir)?;
    let (_snapshot, _roots, _targets, counts) = scan(store, &ledger, now_ms()?)?;
    Ok(total_cache_measurement(&counts))
}

pub(crate) async fn cancel_with_reconciliation(
    state: &AppState,
    job_id: &str,
    reason: crate::jobs::JobCancellationReason,
    plan: &CachePurgePlan,
    deleted_files: u64,
    deleted_bytes: u64,
    // Derived measurements require the lifecycle writer lease. A cancellation
    // can arrive before the worker obtains it, in which case a vanished
    // ProjectStore must remain a cancellation without a made-up delta.
    derived_delta_allowed: bool,
) {
    match post_measurement(state, plan).await {
        Ok(after) => state.jobs.cancel_from_worker_with_result(
            job_id,
            reason,
            reconciliation_result(
                plan,
                deleted_files,
                deleted_bytes,
                after,
                ReconciliationStatus::Cancelled,
                true,
                false,
            ),
        ),
        // Project transition removes the ProjectStore before waiting for this
        // cooperative worker. The exclusive lease and strict preflight still
        // make the delta exact for managed cache changes, so retain cancellation
        // rather than flattening it into a generic post-measurement failure.
        Err(error)
            if reason == crate::jobs::JobCancellationReason::ProjectSwitch
                && error.code == error_codes::NOT_FOUND =>
        {
            match derived_delta_allowed.then(|| derived_after(plan, deleted_files, deleted_bytes)) {
                Some(Ok(after)) => state.jobs.cancel_from_worker_with_result(
                    job_id,
                    reason,
                    reconciliation_result(
                        plan,
                        deleted_files,
                        deleted_bytes,
                        after,
                        ReconciliationStatus::Cancelled,
                        false,
                        false,
                    ),
                ),
                _ => state.jobs.cancel_from_worker(job_id, reason),
            }
        }
        // A cancellation request remains a cancellation even if a strict
        // after-scan cannot be read for another reason. Do not mislabel it as
        // a cache execution failure or claim an unverified reconciliation.
        Err(_) => state.jobs.cancel_from_worker(job_id, reason),
    }
}

pub(crate) fn fail_after_removed(
    state: &AppState,
    job_id: &str,
    plan: &CachePurgePlan,
    deleted_files: u64,
    deleted_bytes: u64,
    error: CutError,
) {
    fail_with_delta(
        state,
        job_id,
        plan,
        deleted_files,
        deleted_bytes,
        error,
        true,
    );
}

fn fail_with_delta(
    state: &AppState,
    job_id: &str,
    plan: &CachePurgePlan,
    deleted_files: u64,
    deleted_bytes: u64,
    error: CutError,
    ledger_recovery_required: bool,
) {
    match derived_after(plan, deleted_files, deleted_bytes) {
        Ok(after) => state.jobs.fail_with_result(
            job_id,
            error,
            reconciliation_result(
                plan,
                deleted_files,
                deleted_bytes,
                after,
                ReconciliationStatus::Failed,
                false,
                ledger_recovery_required,
            ),
        ),
        Err(_) => state.jobs.fail(job_id, error),
    }
}

/// Keep the terminal job truthful when a later planned entry fails after this
/// worker has already removed earlier entries. The caller retains the exclusive
/// cache lease, so a successful after-scan is the exact current inventory.
pub(crate) async fn fail_with_reconciliation(
    state: &AppState,
    job_id: &str,
    plan: &CachePurgePlan,
    deleted_files: u64,
    deleted_bytes: u64,
    error: CutError,
) {
    match post_measurement(state, plan).await {
        Ok(after) => state.jobs.fail_with_result(
            job_id,
            error,
            reconciliation_result(
                plan,
                deleted_files,
                deleted_bytes,
                after,
                ReconciliationStatus::Failed,
                true,
                false,
            ),
        ),
        // The strict scan itself can fail after exact earlier removals (for
        // example an external root fault). Preserve the known lifecycle delta
        // rather than discarding all terminal accounting. It is explicitly not
        // a strict scan and does not imply a ledger write failed.
        Err(_) => fail_with_delta(
            state,
            job_id,
            plan,
            deleted_files,
            deleted_bytes,
            error,
            false,
        ),
    }
}

pub(crate) fn derived_after(
    plan: &CachePurgePlan,
    deleted_files: u64,
    deleted_bytes: u64,
) -> Result<CacheMeasurement, CutError> {
    Ok(CacheMeasurement {
        files: plan
            .before
            .files
            .checked_sub(deleted_files)
            .ok_or_else(|| {
                cache_error(
                    "cache purge accounting is no longer valid",
                    "confirmed removals exceed the strict preview inventory",
                )
            })?,
        bytes: plan
            .before
            .bytes
            .checked_sub(deleted_bytes)
            .ok_or_else(|| {
                cache_error(
                    "cache purge accounting is no longer valid",
                    "confirmed removed bytes exceed the strict preview inventory",
                )
            })?,
    })
}

pub(crate) fn reconciliation_result(
    plan: &CachePurgePlan,
    deleted_files: u64,
    deleted_bytes: u64,
    after: CacheMeasurement,
    status: ReconciliationStatus,
    after_measured: bool,
    ledger_recovery_required: bool,
) -> Value {
    let planned = CacheMeasurement {
        files: plan.targets.len() as u64,
        bytes: plan
            .targets
            .iter()
            .fold(0u64, |total, target| total.saturating_add(target.bytes)),
    };
    let removed = CacheMeasurement {
        files: deleted_files,
        bytes: deleted_bytes,
    };
    let balanced = plan.before.files == removed.files.saturating_add(after.files)
        && plan.before.bytes == removed.bytes.saturating_add(after.bytes);
    json!({
        "schema": "shellx-cut/cache-purge-reconciliation/1",
        "status": status.public(),
        "reconciliation": {
            "before": plan.before.public(),
            "planned": planned.public(),
            "removed": removed.public(),
            "after": after.public(),
            "balanced": balanced,
            "partial_progress": !matches!(status, ReconciliationStatus::Completed) && removed.files > 0,
            "after_basis": if after_measured { "strict_scan" } else { "exclusive_lease_delta" },
            "ledger_recovery_required": ledger_recovery_required,
        },
        "retention_minimum_age_ms": CACHE_RETENTION_MS,
        "note": "only ledger-owned, unreferenced, aged proxy and filmstrip files were eligible"
    })
}
