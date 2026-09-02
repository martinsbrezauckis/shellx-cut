//! Deterministic output production for an already-admitted rebuild plan.
//!
//! Reservations are durable before this worker begins. This module owns source
//! rechecks, generated-output promotion, and metadata backfill, while the
//! scheduler owns only bounded admission and work-unit accounting.

use super::super::ownership::{abandon_rebuild_output, complete_rebuild_output, relative_output};
use super::super::{cache_error, CacheKind, CACHE_RETENTION_MS};
use super::admission::asset_matches_snapshot;
use super::{OutputAction, RebuildAsset, RebuildAssetPlan, PROXY_MAX_RUNNING};
use crate::dispatch::{owned_job_process_control, run_blocking, run_blocking_cancellable};
use crate::jobs::current_job_cancellation;
use crate::state::AppState;
use cut_core::CutError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) async fn execute_rebuild(
    state: AppState,
    job_id: String,
    project_dir: PathBuf,
    plans: Vec<RebuildAssetPlan>,
) {
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
    // Every ffmpeg child below shares this cancellation-aware owner. A cancel or
    // project switch therefore stops and reaps the active child before this job
    // reports a terminal outcome or releases its cache lease.
    let process_control = owned_job_process_control(cancellation.clone());
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
            rebuild_output(
                state,
                job_id,
                project_dir,
                &plan.asset,
                *kind,
                &process_control,
            )
            .await?;
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
    process_control: &cut_media::ffmpeg::OwnedProcessControl,
) -> Result<(), RebuildWorkerError> {
    match kind {
        CacheKind::Proxies => {
            let source = asset.source.clone();
            let proxies = project_dir.join("proxies");
            let asset_id = asset.asset_id.clone();
            let total = asset.duration_ms.unwrap_or_default();
            let progress_state = state.clone();
            let progress_job = job_id.to_owned();
            let control = process_control.clone();
            run_blocking_cancellable("cache.rebuild proxy", move |_| {
                cut_media::ffmpeg::with_render_process_control(&control, || {
                    let progress = move |fraction: f32| {
                        progress_state.jobs.progress(
                            &progress_job,
                            0.10 + fraction.clamp(0.0, 1.0) * 0.70,
                            Some("rebuilding editing proxy".into()),
                        );
                    };
                    cut_media::make_proxy_with_progress(
                        &source, &proxies, &asset_id, total, &progress,
                    )
                })
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
            let control = process_control.clone();
            run_blocking_cancellable("cache.rebuild filmstrip", move |_| {
                cut_media::ffmpeg::with_render_process_control(&control, || {
                    cut_media::filmstrip::make_filmstrip(&proxy, &filmstrip, &asset_id, duration)
                })
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
            let control = process_control.clone();
            run_blocking_cancellable("cache.rebuild image thumbnail", move |_| {
                cut_media::ffmpeg::with_render_process_control(&control, || {
                    cut_media::filmstrip::make_image_thumb(&source, &filmstrip, &asset_id)
                })
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

async fn source_matches(asset: &RebuildAsset) -> Result<bool, RebuildWorkerError> {
    let source = asset.source.clone();
    let expected = asset.hash.clone();
    Ok(run_blocking("cache.rebuild source identity", move || {
        Ok(cut_core::hash_file(&source).ok().as_deref() == Some(expected.as_str()))
    })
    .await?)
}
