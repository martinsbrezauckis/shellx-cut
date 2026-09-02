//! Revision-bound cache rebuild admission.
//!
//! Snapshotting and revalidation stay apart from estimation and worker output:
//! a scheduler can prove its candidate asset set stayed current without
//! reserving output or touching derived cache files.

use super::super::cache_error;
use super::super::ownership::retire_orphaned_pending_outputs;
use super::{RebuildAsset, CACHE_REBUILD_ASSET_LIMIT};
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(super) async fn snapshot_assets(
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

pub(super) async fn revalidate_snapshot(
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

/// A pending reservation whose asset no longer exists cannot resume. The
/// scheduler owns the lifecycle writer lease when this runs, so exact pending
/// outputs may be retired without racing a producer. A failed ledger publish is
/// left visible and retryable rather than hidden.
pub(super) async fn retire_orphaned_reservations(
    state: &AppState,
    project_dir: &Path,
) -> Result<(), CutError> {
    let assets = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "no project open",
                "the project closed while cache rebuild admission was running",
            )
        })?;
        if store.dir != project_dir {
            return Err(cache_error(
                "cache rebuild admission is no longer current",
                "the open project changed while cache rebuild scheduling was verifying it",
            ));
        }
        store.project.assets.keys().cloned().collect()
    };
    retire_orphaned_pending_outputs(project_dir, &assets)
}

pub(super) fn asset_matches_snapshot(asset: &cut_core::Asset, snapshot: &RebuildAsset) -> bool {
    asset.path == snapshot.path
        && asset.hash == snapshot.hash
        && asset.probe.clone().unwrap_or(Value::Null) == snapshot.probe
}

fn source_path(project_dir: &Path, value: &str) -> PathBuf {
    let value = PathBuf::from(value);
    if value.is_relative() {
        project_dir.join(value)
    } else {
        value
    }
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
