//! Deterministic, bounded rebuild scheduling for owned editing caches.
//!
//! This module deliberately owns only the project-local base proxy and
//! filmstrip outputs. It does not introduce a generic cache queue: every target
//! is derived from a registered asset, its current source identity, and the
//! durable ownership ledger.

use super::ownership::{
    abandon_rebuild_output, complete_rebuild_output, rebuild_output_state, relative_output,
    reserve_rebuild_output, RebuildOutputState,
};
use super::*;
use crate::dispatch::run_blocking;
use crate::jobs::{begin_current_process_worker, current_job_cancellation};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CACHE_REBUILD_ASSET_LIMIT: usize = 64;
const CACHE_REBUILD_OUTPUT_LIMIT: usize = CACHE_REBUILD_ASSET_LIMIT * 2;
const PROXY_MAX_RUNNING: usize = 1;

#[derive(Debug, Deserialize, Default)]
struct RebuildArgs {
    #[serde(default)]
    asset_ids: Vec<String>,
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

    if let Some(active) = state.cache_rebuild_active.lock().await.clone() {
        return Ok(VerbResult::ok(json!({
            "schema": "shellx-cut/cache-rebuild/1",
            "status": "already_queued",
            "job_id": active.job_id,
            "deduplicated": true,
            "scheduled_assets": active.assets.len(),
            "scheduled_outputs": active.scheduled_outputs,
            "counts": active.counts,
        })));
    }

    let (project_dir, assets) = snapshot_assets(state, &requested).await?;
    let source_checks = run_blocking("cache.rebuild source identity", {
        let assets = assets.clone();
        move || {
            Ok(assets
                .iter()
                .map(|asset| {
                    let actual = cut_core::hash_file(&asset.source).ok();
                    (
                        asset.asset_id.clone(),
                        (actual == Some(asset.hash.clone()), actual.is_some()),
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
        })));
    }
    revalidate_snapshot(state, &project_dir, &assets).await?;

    let mut counts = ScheduleCounts::default();
    let mut plans = Vec::new();
    for asset in assets {
        let Some((matches, readable)) = source_checks.get(&asset.asset_id).copied() else {
            counts.source_unavailable = counts.source_unavailable.saturating_add(1);
            continue;
        };
        if !readable {
            counts.source_unavailable = counts.source_unavailable.saturating_add(1);
            continue;
        }
        if !matches {
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
        plans.push(RebuildAssetPlan { asset, outputs });
    }

    let output_count = plans.iter().map(|plan| plan.outputs.len()).sum::<usize>();
    if output_count > CACHE_REBUILD_OUTPUT_LIMIT {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "cache rebuild would exceed its bounded output inventory",
            format!("schedule at most {CACHE_REBUILD_OUTPUT_LIMIT} rebuild outputs at once"),
        ));
    }
    for plan in &plans {
        for (kind, action) in &plan.outputs {
            if *action == OutputAction::Generate {
                reserve_rebuild_output(
                    &project_dir,
                    *kind,
                    &plan.asset.asset_id,
                    &plan.asset.hash,
                )?;
            }
        }
    }

    if plans.is_empty() {
        return Ok(VerbResult::ok(json!({
            "schema": "shellx-cut/cache-rebuild/1",
            "status": "not_needed",
            "scheduled_assets": 0,
            "scheduled_outputs": 0,
            "counts": counts.public(),
        })));
    }

    let job = state.jobs.create("cache_rebuild");
    let job_id = job.job_id.clone();
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
    })))
}

async fn snapshot_assets(
    state: &AppState,
    requested: &BTreeSet<String>,
) -> Result<(PathBuf, Vec<RebuildAsset>), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no project open",
            "open a project before rebuilding its editing cache",
        )
    })?;
    let mut assets = store
        .project
        .assets
        .iter()
        .filter(|(asset_id, _)| requested.is_empty() || requested.contains(*asset_id))
        .map(|(asset_id, asset)| RebuildAsset {
            asset_id: asset_id.clone(),
            path: asset.path.clone(),
            source: source_path(&store.dir, &asset.path),
            hash: asset.hash.clone(),
            probe: asset.probe.clone().unwrap_or(Value::Null),
            kind: asset
                .probe
                .as_ref()
                .and_then(|probe| probe.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            duration_ms: asset
                .probe
                .as_ref()
                .and_then(|probe| probe.get("duration_ms"))
                .and_then(Value::as_u64),
            proxy: asset.proxy.clone(),
            filmstrip: asset.filmstrip.clone(),
        })
        .collect::<Vec<_>>();
    if !requested.is_empty() && assets.len() != requested.len() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "a requested cache rebuild asset is not in the open project",
            "call project.health or project.state and retry with current asset ids",
        ));
    }
    if assets.len() > CACHE_REBUILD_ASSET_LIMIT {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "cache rebuild needs an explicit bounded asset selection",
            format!(
                "the open project has more than {CACHE_REBUILD_ASSET_LIMIT} assets; request a bounded asset_ids list"
            ),
        ));
    }
    assets.sort_by(|left, right| compare_asset_ids(&left.asset_id, &right.asset_id));
    Ok((store.dir.clone(), assets))
}

async fn revalidate_snapshot(
    state: &AppState,
    project_dir: &Path,
    snapshot: &[RebuildAsset],
) -> Result<(), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no project open",
            "the project closed while cache rebuild admission was running",
        )
    })?;
    if store.dir != project_dir
        || snapshot.iter().any(|candidate| {
            store
                .project
                .assets
                .get(&candidate.asset_id)
                .is_none_or(|asset| !asset_matches_snapshot(asset, candidate))
        })
    {
        return Err(cache_error(
            "cache rebuild admission is no longer current",
            "asset source metadata changed while rebuild scheduling was verifying it",
        ));
    }
    Ok(())
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

async fn execute_rebuild(
    state: AppState,
    job_id: String,
    project_dir: PathBuf,
    plans: Vec<RebuildAssetPlan>,
) {
    let _process_guard = begin_current_process_worker();
    let outcome = state
        .jobs
        .with_limit(
            "proxy",
            PROXY_MAX_RUNNING,
            execute_rebuild_inner(&state, &job_id, &project_dir, &plans),
        )
        .await;
    clear_active(&state, &job_id).await;
    match outcome {
        Ok(result) => state.jobs.finish(&job_id, result),
        Err(RebuildWorkerError::Cancelled(reason)) => {
            state.jobs.cancel_from_worker(&job_id, reason)
        }
        Err(RebuildWorkerError::Failed(error)) => state.jobs.fail(&job_id, error),
    }
}

enum RebuildWorkerError {
    Cancelled(crate::jobs::JobCancellationReason),
    Failed(CutError),
}

impl From<CutError> for RebuildWorkerError {
    fn from(error: CutError) -> Self {
        Self::Failed(error)
    }
}

async fn execute_rebuild_inner(
    state: &AppState,
    job_id: &str,
    project_dir: &Path,
    plans: &[RebuildAssetPlan],
) -> Result<Value, RebuildWorkerError> {
    let cancellation = current_job_cancellation();
    if let Some(reason) = cancellation.reason() {
        return Err(RebuildWorkerError::Cancelled(reason));
    }
    // This exclusive lease prevents manual cache producers, purge, and another
    // scheduler from interleaving final filenames with the reservation protocol.
    let _cache_lease = state.cache_lifecycle_lease.write().await;
    let total = plans.len().max(1) as f32;
    let mut rebuilt_outputs = 0u64;
    let mut backfilled_outputs = 0u64;

    for (index, plan) in plans.iter().enumerate() {
        if let Some(reason) = cancellation.reason() {
            return Err(RebuildWorkerError::Cancelled(reason));
        }
        state.jobs.progress(
            job_id,
            0.05 + (index as f32 / total) * 0.85,
            Some("verifying source identity for deterministic cache rebuild".into()),
        );
        if !source_matches(&plan.asset).await? {
            abandon_generated_outputs(project_dir, plan)?;
            return Err(RebuildWorkerError::Failed(cache_error(
                "cache rebuild refused a changed source",
                "the source identity no longer matches the imported asset; relink the asset before rebuilding its cache",
            )));
        }

        for (kind, action) in &plan.outputs {
            if let Some(reason) = cancellation.reason() {
                return Err(RebuildWorkerError::Cancelled(reason));
            }
            if *action == OutputAction::Backfill {
                backfilled_outputs = backfilled_outputs.saturating_add(1);
                continue;
            }
            rebuild_output(state, job_id, project_dir, &plan.asset, *kind).await?;
            rebuilt_outputs = rebuilt_outputs.saturating_add(1);
        }

        if let Some(reason) = cancellation.reason() {
            return Err(RebuildWorkerError::Cancelled(reason));
        }
        if !source_matches(&plan.asset).await? {
            abandon_generated_outputs(project_dir, plan)?;
            return Err(RebuildWorkerError::Failed(cache_error(
                "cache rebuild discarded output from a changed source",
                "the source changed during rebuilding, so no derived cache was published",
            )));
        }
        for (kind, action) in &plan.outputs {
            if *action == OutputAction::Generate {
                complete_rebuild_output(
                    project_dir,
                    *kind,
                    &plan.asset.asset_id,
                    &plan.asset.hash,
                )?;
            }
        }
        apply_backfill(state, project_dir, plan).await?;
    }

    Ok(json!({
        "rebuilt_outputs": rebuilt_outputs,
        "backfilled_outputs": backfilled_outputs,
        "retention_minimum_age_ms": CACHE_RETENTION_MS,
        "note": "only current-source, ledger-reserved proxy and base filmstrip outputs were rebuilt",
    }))
}

async fn rebuild_output(
    state: &AppState,
    job_id: &str,
    project_dir: &Path,
    asset: &RebuildAsset,
    kind: CacheKind,
) -> Result<(), RebuildWorkerError> {
    match kind {
        CacheKind::Proxies => {
            let source = asset.source.clone();
            let proxies = project_dir.join("proxies");
            let asset_id = asset.asset_id.clone();
            let total = asset.duration_ms.unwrap_or_default();
            let progress_state = state.clone();
            let progress_job = job_id.to_owned();
            run_blocking("cache.rebuild proxy", move || {
                let progress = move |fraction: f32| {
                    progress_state.jobs.progress(
                        &progress_job,
                        0.10 + fraction.clamp(0.0, 1.0) * 0.70,
                        Some("rebuilding editing proxy".into()),
                    );
                };
                cut_media::make_proxy_with_progress(&source, &proxies, &asset_id, total, &progress)
            })
            .await?;
        }
        CacheKind::Thumbnails if asset.kind == "video" => {
            let proxy = project_dir
                .join("proxies")
                .join(format!("{}.mp4", asset.asset_id));
            let filmstrip = project_dir.join("filmstrip");
            let asset_id = asset.asset_id.clone();
            let duration = asset.duration_ms.ok_or_else(|| {
                cache_error(
                    "cache rebuild cannot derive a filmstrip",
                    "the video asset has no positive recorded duration",
                )
            })?;
            state
                .jobs
                .progress(job_id, 0.86, Some("rebuilding timeline filmstrip".into()));
            run_blocking("cache.rebuild filmstrip", move || {
                cut_media::filmstrip::make_filmstrip(&proxy, &filmstrip, &asset_id, duration)
            })
            .await?;
        }
        CacheKind::Thumbnails if asset.kind == "image" => {
            let source = asset.source.clone();
            let filmstrip = project_dir.join("filmstrip");
            let asset_id = asset.asset_id.clone();
            state.jobs.progress(
                job_id,
                0.60,
                Some("rebuilding still-image thumbnail".into()),
            );
            run_blocking("cache.rebuild image thumbnail", move || {
                cut_media::filmstrip::make_image_thumb(&source, &filmstrip, &asset_id)
            })
            .await?;
        }
        _ => {
            return Err(RebuildWorkerError::Failed(cache_error(
                "cache rebuild target is not supported",
                "only video proxies and video/image base filmstrips are rebuildable",
            )));
        }
    }
    Ok(())
}

fn abandon_generated_outputs(project_dir: &Path, plan: &RebuildAssetPlan) -> Result<(), CutError> {
    for (kind, action) in &plan.outputs {
        if *action == OutputAction::Generate {
            abandon_rebuild_output(project_dir, *kind, &plan.asset.asset_id, &plan.asset.hash)?;
        }
    }
    Ok(())
}

async fn apply_backfill(
    state: &AppState,
    project_dir: &Path,
    plan: &RebuildAssetPlan,
) -> Result<(), RebuildWorkerError> {
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(|| {
        cache_error(
            "cache rebuild cannot publish metadata",
            "the project closed before rebuilt cache metadata could be saved",
        )
    })?;
    if store.dir != project_dir {
        return Err(RebuildWorkerError::Failed(cache_error(
            "cache rebuild cannot publish metadata",
            "a different project became active before rebuilt cache metadata could be saved",
        )));
    }
    let asset = store
        .project
        .assets
        .get_mut(&plan.asset.asset_id)
        .ok_or_else(|| {
            cache_error(
                "cache rebuild cannot publish metadata",
                "the asset was removed before rebuilt cache metadata could be saved",
            )
        })?;
    if !asset_matches_snapshot(asset, &plan.asset) {
        return Err(RebuildWorkerError::Failed(cache_error(
            "cache rebuild cannot publish metadata",
            "the asset source metadata changed while rebuilding; no stale cache reference was saved",
        )));
    }
    for (kind, _) in &plan.outputs {
        let relative = relative_output(*kind, &plan.asset.asset_id)?;
        match kind {
            CacheKind::Proxies => asset.proxy = Some(relative),
            CacheKind::Thumbnails => asset.filmstrip = Some(relative),
        }
    }
    store.save()?;
    Ok(())
}

async fn clear_active(state: &AppState, job_id: &str) {
    let mut active = state.cache_rebuild_active.lock().await;
    if active.as_ref().is_some_and(|entry| entry.job_id == job_id) {
        *active = None;
    }
}

fn source_path(project_dir: &Path, value: &str) -> PathBuf {
    let value = PathBuf::from(value);
    if value.is_relative() {
        project_dir.join(value)
    } else {
        value
    }
}

fn asset_matches_snapshot(asset: &cut_core::Asset, snapshot: &RebuildAsset) -> bool {
    asset.path == snapshot.path
        && asset.hash == snapshot.hash
        && asset.probe.clone().unwrap_or(Value::Null) == snapshot.probe
}

fn compare_asset_ids(left: &str, right: &str) -> std::cmp::Ordering {
    let number = |value: &str| {
        value
            .strip_prefix('a')
            .and_then(|suffix| suffix.parse::<u64>().ok())
    };
    match (number(left), number(right)) {
        (Some(left_number), Some(right_number)) => {
            left_number.cmp(&right_number).then_with(|| left.cmp(right))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.cmp(right),
    }
}

async fn source_matches(asset: &RebuildAsset) -> Result<bool, RebuildWorkerError> {
    let source = asset.source.clone();
    let expected = asset.hash.clone();
    Ok(run_blocking("cache.rebuild source identity", move || {
        Ok(cut_core::hash_file(&source).ok().as_deref() == Some(expected.as_str()))
    })
    .await?)
}

#[cfg(test)]
mod tests;
