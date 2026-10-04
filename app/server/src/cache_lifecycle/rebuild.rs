//! Deterministic, bounded rebuild scheduling for owned editing caches.
//!
//! This module deliberately owns only the project-local base proxy and
//! filmstrip outputs. It does not introduce a generic cache queue: every target
//! is derived from a registered asset, its current source identity, and the
//! durable ownership ledger.

use super::ownership::{
    pending_rebuild_outputs_for_asset, rebuild_output_state, relative_output,
    reserve_rebuild_output, RebuildOutputState,
};
use super::*;
use crate::dispatch::run_blocking;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

mod admission;
mod estimate;
mod partial;
mod worker;
use admission::{retire_orphaned_reservations, revalidate_snapshot, snapshot_assets};
use estimate::{RebuildEstimate, SourceCheck};
use partial::{partial_reservation_result, reserve_planned_outputs};
use worker::execute_rebuild;

const CACHE_REBUILD_ASSET_LIMIT: usize = 64;
const CACHE_REBUILD_OUTPUT_LIMIT: usize = CACHE_REBUILD_ASSET_LIMIT * 2;
const PROXY_MAX_RUNNING: usize = 1;

#[derive(Debug, Deserialize, Default)]
struct RebuildArgs {
    #[serde(default)]
    asset_ids: Vec<String>,
    /// Run the same bounded source-identity admission pass without reserving
    /// outputs or creating a job. This is intentionally an estimate of known
    /// work units, not an invented duration prediction.
    #[serde(default)]
    estimate_only: bool,
}

#[derive(Debug, Clone)]
struct RebuildAsset {
    asset_id: String,
    path: String,
    source: PathBuf,
    hash: String,
    probe: Value,
    kind: String,
    duration_ms: Option<u64>,
    proxy: Option<String>,
    filmstrip: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputAction {
    Generate,
    Backfill,
}

#[derive(Debug, Clone)]
struct RebuildAssetPlan {
    asset: RebuildAsset,
    outputs: Vec<(CacheKind, OutputAction)>,
    /// The current source byte count measured only after its imported hash
    /// matched during this admission pass.
    source_bytes: u64,
}

#[derive(Default)]
struct ScheduleCounts {
    fresh_assets: u64,
    source_changed: u64,
    source_unavailable: u64,
    unowned_outputs: u64,
    legacy_outputs: u64,
    unsupported_assets: u64,
}

impl ScheduleCounts {
    fn public(&self) -> Value {
        json!({
            "fresh_assets": self.fresh_assets,
            "source_changed": self.source_changed,
            "source_unavailable": self.source_unavailable,
            "unowned_outputs": self.unowned_outputs,
            "legacy_outputs": self.legacy_outputs,
            "unsupported_assets": self.unsupported_assets,
        })
    }
}

pub(crate) async fn start_rebuild(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    start_rebuild_with_reserver(state, args, reserve_rebuild_output).await
}

async fn start_rebuild_with_reserver<F>(
    state: &AppState,
    args: Value,
    reserve: F,
) -> Result<VerbResult, CutError>
where
    F: FnMut(&std::path::Path, CacheKind, &str, &str) -> Result<(), CutError>,
{
    let args: RebuildArgs = serde_json::from_value(args).map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "invalid cache rebuild request",
            "send no asset_ids to backfill every eligible asset, or a bounded unique asset_ids list",
        )
    })?;
    if args.asset_ids.len() > CACHE_REBUILD_ASSET_LIMIT {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "cache rebuild request exceeds the bounded asset limit",
            format!("request at most {CACHE_REBUILD_ASSET_LIMIT} asset ids"),
        ));
    }
    let requested = args.asset_ids.iter().cloned().collect::<BTreeSet<_>>();
    if requested.len() != args.asset_ids.len() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "cache rebuild asset ids must be unique",
            "remove duplicate asset ids before scheduling a deterministic rebuild",
        ));
    }

    // A rebuild owns an absolute project directory after it is queued. Hold the
    // transition through snapshot, final admission, and task registration so a
    // project replacement cannot detach jobs between those steps.
    let _transition = state.project_transition.lock().await;
    if let Some(active) = state.cache_rebuild_active.lock().await.clone() {
        return Ok(VerbResult::ok(json!({
            "schema": "shellx-cut/cache-rebuild/1",
            "status": "already_queued",
            "job_id": active.job_id,
            "deduplicated": true,
            "scheduled_assets": active.assets.len(),
            "scheduled_outputs": active.scheduled_outputs,
            "counts": active.counts,
            "estimate": active.estimate,
        })));
    }

    let (project_dir, assets) = snapshot_assets(state, &requested).await?;
    let source_checks = run_blocking("cache.rebuild source identity", {
        let assets = assets.clone();
        move || {
            Ok(assets
                .iter()
                .map(|asset| {
                    (
                        asset.asset_id.clone(),
                        SourceCheck::verify(&asset.source, &asset.hash),
                    )
                })
                .collect::<Vec<_>>())
        }
    })
    .await?;
    let source_checks = source_checks
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();

    // Scheduling refuses to wait behind a producer. A caller can retry after
    // the existing cache worker settles; this preserves the same admission
    // boundary as project.cache_preview.
    let _cache_lease = state
        .cache_lifecycle_lease
        .try_write()
        .map_err(|_| cache_busy_error())?;
    if let Some(active) = state.cache_rebuild_active.lock().await.clone() {
        return Ok(VerbResult::ok(json!({
            "schema": "shellx-cut/cache-rebuild/1",
            "status": "already_queued",
            "job_id": active.job_id,
            "deduplicated": true,
            "scheduled_assets": active.assets.len(),
            "scheduled_outputs": active.scheduled_outputs,
            "counts": active.counts,
            "estimate": active.estimate,
        })));
    }
    revalidate_snapshot(state, &project_dir, &assets).await?;
    retire_orphaned_reservations(state, &project_dir).await?;

    let mut counts = ScheduleCounts::default();
    let mut plans = Vec::new();
    for asset in assets {
        let Some(check) = source_checks.get(&asset.asset_id).copied() else {
            pending_source_recovery(&project_dir, &asset.asset_id, "unavailable")?;
            counts.source_unavailable = counts.source_unavailable.saturating_add(1);
            continue;
        };
        if !check.readable {
            pending_source_recovery(&project_dir, &asset.asset_id, "unavailable")?;
            counts.source_unavailable = counts.source_unavailable.saturating_add(1);
            continue;
        }
        if !check.matches {
            pending_source_recovery(&project_dir, &asset.asset_id, "changed")?;
            counts.source_changed = counts.source_changed.saturating_add(1);
            continue;
        }
        let Some(required) = required_outputs(&asset) else {
            counts.unsupported_assets = counts.unsupported_assets.saturating_add(1);
            continue;
        };
        let mut outputs = Vec::new();
        let mut blocked = false;
        for kind in required {
            let relative = relative_output(kind, &asset.asset_id)?;
            let recorded = match kind {
                CacheKind::Proxies => asset.proxy.as_deref(),
                CacheKind::Thumbnails => asset.filmstrip.as_deref(),
            };
            match rebuild_output_state(&project_dir, kind, &asset.asset_id, &asset.hash)? {
                RebuildOutputState::ReadyVerified if recorded == Some(relative.as_str()) => {}
                RebuildOutputState::ReadyVerified => outputs.push((kind, OutputAction::Backfill)),
                RebuildOutputState::Pending | RebuildOutputState::MissingOrStale => {
                    outputs.push((kind, OutputAction::Generate))
                }
                RebuildOutputState::ReadyLegacy => {
                    counts.legacy_outputs = counts.legacy_outputs.saturating_add(1);
                    blocked = true;
                }
                RebuildOutputState::UnownedPresent => {
                    counts.unowned_outputs = counts.unowned_outputs.saturating_add(1);
                    blocked = true;
                }
            }
        }
        if blocked {
            continue;
        }
        if outputs.is_empty() {
            counts.fresh_assets = counts.fresh_assets.saturating_add(1);
            continue;
        }
        plans.push(RebuildAssetPlan {
            asset,
            outputs,
            // `readable` proves the measured bytes are present.
            source_bytes: check.bytes.unwrap_or_default(),
        });
    }

    let output_count = plans.iter().map(|plan| plan.outputs.len()).sum::<usize>();
    if output_count > CACHE_REBUILD_OUTPUT_LIMIT {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "cache rebuild would exceed its bounded output inventory",
            format!("schedule at most {CACHE_REBUILD_OUTPUT_LIMIT} rebuild outputs at once"),
        ));
    }
    let estimate = RebuildEstimate::from_plans(&plans).public();
    if args.estimate_only {
        return Ok(VerbResult::ok(json!({
            "schema": "shellx-cut/cache-rebuild/1",
            "status": "estimated",
            "scheduled_assets": plans.len(),
            "scheduled_outputs": output_count,
            "counts": counts.public(),
            "estimate": estimate,
        })));
    }
    if plans.is_empty() {
        return Ok(VerbResult::ok(json!({
            "schema": "shellx-cut/cache-rebuild/1",
            "status": "not_needed",
            "scheduled_assets": 0,
            "scheduled_outputs": 0,
            "counts": counts.public(),
            "estimate": estimate,
        })));
    }

    // Reserve/replacement can retire an old cache output, so admit a durable
    // job before the first such mutation. A failed job-store write returns
    // before any ledger or output changes.
    let job = state.jobs.create_durable("cache_rebuild")?;
    let job_id = job.job_id.clone();
    if let Err(failure) = reserve_planned_outputs(&project_dir, &plans, reserve) {
        let mut error = failure.error.with_suggested_action(
            "check the failed cache rebuild job for pending outputs; restore or relink a changed source, or remove the asset, then retry cache rebuild",
        );
        let result = partial_reservation_result(&project_dir, &failure.targets);
        error.message = format!(
            "{}; rebuild stopped—check Jobs for pending cache outputs, then retry",
            error.message
        );
        state.jobs.fail_with_result(&job_id, error.clone(), result);
        return Err(error);
    }
    let active_assets = plans
        .iter()
        .map(|plan| plan.asset.asset_id.clone())
        .collect::<Vec<_>>();
    let active_count = active_assets.len();
    *state.cache_rebuild_active.lock().await = Some(CacheRebuildActive {
        job_id: job_id.clone(),
        assets: active_assets,
        scheduled_outputs: output_count,
        counts: counts.public(),
        estimate: estimate.clone(),
    });
    let worker_state = state.clone();
    let worker_job_id = job_id.clone();
    state.jobs.spawn(&job_id, async move {
        execute_rebuild(worker_state, worker_job_id, project_dir, plans).await;
    });

    Ok(VerbResult::ok(json!({
        "schema": "shellx-cut/cache-rebuild/1",
        "status": "queued",
        "job_id": job_id,
        "scheduled_assets": active_count,
        "scheduled_outputs": output_count,
        "counts": counts.public(),
        "estimate": estimate,
    })))
}

fn pending_source_recovery(
    project_dir: &std::path::Path,
    asset: &str,
    state: &str,
) -> Result<(), CutError> {
    if pending_rebuild_outputs_for_asset(project_dir, asset)?.is_empty() {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "cache rebuild requires pending source recovery",
        format!(
            "a pending rebuild reservation cannot be retired while its source is {state}; restore the exact source, relink it, or remove the asset before retrying"
        ),
    )
    .with_suggested_action(
        "restore or relink the source, or remove the asset; then retry cache rebuild to reconcile its pending reservation",
    ))
}

fn required_outputs(asset: &RebuildAsset) -> Option<Vec<CacheKind>> {
    match asset.kind.as_str() {
        "video" if asset.duration_ms.is_some_and(|duration| duration > 0) => {
            Some(vec![CacheKind::Proxies, CacheKind::Thumbnails])
        }
        "image" => Some(vec![CacheKind::Thumbnails]),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
