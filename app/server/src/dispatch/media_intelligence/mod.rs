//! Cited, rebuildable media-evidence index and read-only search contracts.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

mod build;
mod inspect;
mod markers;
mod merge;
mod model;
mod navigation;
mod search;
mod status;
mod visual;

use build::{build_index, publish_index};
use inspect::{inspect_media_value, inspect_range_value, InspectMediaArgs, InspectRangeArgs};
use merge::merge_index;
use model::{current_bindings, load_index, ProjectSnapshot, ALL_KINDS};
use search::{search_value, SearchArgs};
use status::status_value;

#[cfg(test)]
mod tests;

#[derive(Debug, serde::Deserialize)]
struct ScopeArgs {
    asset_ids: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct RebuildArgs {
    asset_ids: Option<Vec<String>>,
    kinds: Option<Vec<String>>,
}

async fn project_snapshot(state: &AppState) -> Result<ProjectSnapshot, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    Ok(ProjectSnapshot {
        dir: store.dir.clone(),
        revision: store.log.current_revision()?,
        project: store.project.clone(),
    })
}

fn validated_kinds(kinds: Option<Vec<String>>) -> Result<BTreeSet<String>, CutError> {
    let kinds: BTreeSet<String> = kinds
        .unwrap_or_else(|| ALL_KINDS.iter().map(|kind| kind.to_string()).collect())
        .into_iter()
        .collect();
    if kinds.is_empty() || kinds.iter().any(|kind| !ALL_KINDS.contains(&kind.as_str())) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "unknown or empty media intelligence kind set",
            "use transcript, visual, scene, beat, marker, or metadata",
        ));
    }
    Ok(kinds)
}

pub(super) async fn media_intelligence_status(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: ScopeArgs = parse_args(args)?;
    let snapshot = project_snapshot(state).await?;
    Ok(VerbResult::ok(status_value(
        &snapshot,
        args.asset_ids.as_deref(),
    )?))
}

pub(super) async fn media_intelligence_rebuild(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: RebuildArgs = parse_args(args)?;
    let kinds = validated_kinds(args.kinds)?;
    let snapshot = project_snapshot(state).await?;
    let selected = args
        .asset_ids
        .map(|ids| ids.into_iter().collect::<BTreeSet<_>>());
    let selected_vec = selected
        .as_ref()
        .map(|ids| ids.iter().cloned().collect::<Vec<_>>());
    let all_bindings = current_bindings(&snapshot, None)?;
    let bindings = current_bindings(&snapshot, selected_vec.as_deref())?;
    let existing = load_index(&snapshot.dir);
    let job = state.jobs.create("media_intelligence");
    let job_id = job.job_id.clone();
    let jobs = state.jobs.clone();
    let state = state.clone();
    jobs.spawn_limited(&job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        state.jobs.progress(
            &job.job_id,
            0.1,
            Some("deriving cited media evidence (background)".into()),
        );
        let result = run_blocking_cancellable("media intelligence rebuild", {
            let snapshot = snapshot.clone();
            let kinds = kinds.clone();
            let selected = selected.clone();
            move |cancellation| {
                let rebuilt = build_index(&snapshot, bindings, kinds.clone(), cancellation)?;
                merge_index(
                    &snapshot,
                    all_bindings,
                    existing,
                    rebuilt,
                    selected.as_ref(),
                    &kinds,
                )
            }
        })
        .await;
        if let Some(reason) = crate::jobs::current_job_cancellation().reason() {
            return state.jobs.cancel_from_worker(&job.job_id, reason);
        }
        let index = match result {
            Ok(index) => index,
            Err(error) => return state.jobs.fail(&job.job_id, error),
        };
        state.jobs.progress(
            &job.job_id,
            0.85,
            Some("validating project identity before publication".into()),
        );
        let current = match project_snapshot(&state).await {
            Ok(current) => current,
            Err(error) => return state.jobs.fail(&job.job_id, error),
        };
        if current.dir != snapshot.dir || current.revision != snapshot.revision {
            return state.jobs.fail(
                &job.job_id,
                CutError::new(
                    error_codes::CONFLICT,
                    "project changed while media evidence was being derived",
                    "no stale index was published; rebuild against the current project revision",
                ),
            );
        }
        if let Err(error) = publish_index(&snapshot, &index) {
            return state.jobs.fail(&job.job_id, error);
        }
        let mut coverage = BTreeMap::<String, usize>::new();
        for entry in &index.entries {
            *coverage.entry(entry.kind.clone()).or_default() += 1;
        }
        state.jobs.finish(
            &job.job_id,
            json!({
                "schema": index.schema,
                "index_id": index.index_id,
                "entry_count": index.entries.len(),
                "assets": index.bindings.len(),
                "coverage": coverage,
            }),
        );
    });
    Ok(VerbResult::ok(json!({"job_id": job_id})))
}

pub(super) async fn media_intelligence_search(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: SearchArgs = parse_args(args)?;
    Ok(VerbResult::ok(
        search_value(project_snapshot(state).await?, args).await?,
    ))
}

pub(super) async fn inspect_media(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: InspectMediaArgs = parse_args(args)?;
    Ok(VerbResult::ok(inspect_media_value(
        &project_snapshot(state).await?,
        args.asset_ids,
    )?))
}

pub(super) async fn inspect_range(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: InspectRangeArgs = parse_args(args)?;
    Ok(VerbResult::ok(inspect_range_value(
        &project_snapshot(state).await?,
        args.index_id.as_deref(),
        &args.evidence_ids,
    )?))
}

pub(super) async fn validate_evidence_attachments(
    state: &AppState,
    index_id: Option<&str>,
    evidence_ids: &[String],
) -> Result<Option<String>, CutError> {
    match (evidence_ids.is_empty(), index_id) {
        (true, None) => Ok(None),
        (false, Some(index_id)) => inspect_range_value(
            &project_snapshot(state).await?,
            Some(index_id),
            evidence_ids,
        )
        .map(|_| Some(index_id.to_string()))
        .map_err(|error| {
            CutError::new(
                &error.code,
                "invalid cited-moment attachments",
                error.message,
            )
        }),
        _ => Err(CutError::new(
            error_codes::INVALID_ARGS,
            "incomplete cited-moment attachments",
            "supply both evidence_ids and their exact evidence_index_id, or omit both",
        )),
    }
}
