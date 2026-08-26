//! Bounded exact-hash offline-media relink preview and grouped apply.

mod plan;
mod scan;
#[cfg(test)]
mod tests;

use super::*;
use plan::{accepted_changes, build_plan, preview_result};
use sha2::{Digest, Sha256};

const PREVIEW_SCHEMA: &str = "shellx-cut/media-relink-preview/1";
const RECEIPT_SCHEMA: &str = "shellx-cut/media-relink-receipt/1";
const PROJECT_ID_SCHEMA: &str = "shellx-cut/project-identity/1";
const MAX_SCAN_FILES: usize = 512;
const MAX_SCAN_DEPTH: usize = 16;
const MAX_SCAN_BYTES: u64 = 32 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
struct OfflineAssetSnapshot {
    asset_id: String,
    expected_hash: String,
    old_path: String,
    source_path: PathBuf,
    display_name: String,
    kind: Option<String>,
    duration_ms: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RelinkSnapshot {
    project_identity: Value,
    project_revision: String,
    root: PathBuf,
    assets: Vec<OfflineAssetSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct Candidate {
    path: String,
    display_name: String,
    hash: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct PlanAsset {
    asset_id: String,
    expected_hash: String,
    old_path: String,
    display_name: String,
    disposition: String,
    chosen_path: Option<String>,
    chosen_hash: Option<String>,
    diagnostics: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RelinkPlan {
    project_identity: Value,
    project_revision: String,
    root: String,
    scan_files: usize,
    scan_directories: usize,
    assets: Vec<PlanAsset>,
}

#[derive(Debug, Clone)]
struct PreparedPlan {
    plan: RelinkPlan,
    plan_hash: String,
}

#[derive(serde::Deserialize)]
struct PreviewArgs {
    root: String,
}

#[derive(serde::Deserialize)]
struct ApplyArgs {
    root: String,
    plan_hash: String,
    accept: Vec<String>,
}

pub(super) async fn media_relink_preview(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: PreviewArgs = parse_args(args)?;
    let snapshot = snapshot(state, PathBuf::from(args.root)).await?;
    let prepared = run_blocking("media.relink_preview", move || build_plan(snapshot)).await?;
    Ok(VerbResult::ok(preview_result(&prepared)))
}

pub(super) async fn media_relink_apply(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let args: ApplyArgs = parse_args(args)?;
    let expected_revision = required_request_revision(&actor)?;
    let snapshot = snapshot(state, PathBuf::from(args.root)).await?;
    if snapshot.project_revision != expected_revision {
        return stale_preview(&expected_revision, &snapshot.project_revision);
    }
    let prepared = run_blocking("media.relink_apply revalidate", move || {
        build_plan(snapshot)
    })
    .await?;
    if prepared.plan_hash != args.plan_hash {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "bulk relink preview no longer matches the recovery folder",
            "candidate identity, file set, or selected folder changed after preview",
        )
        .with_suggested_action("run media.relink_preview again; no assets were changed"));
    }
    let accepted = accepted_changes(&prepared.plan, &args.accept)?;
    let (op, post_revision, warnings) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        let actual = store.log.current_revision()?.unwrap_or_default();
        if actual != expected_revision {
            return stale_preview(&expected_revision, &actual);
        }
        let committed = guard_call("media.relink_apply", || {
            store.record_relink_group(
                accepted.clone(),
                &args.plan_hash,
                prepared.plan.project_identity.clone(),
                actor,
                rationale,
            )
        })?;
        let post_revision = store.log.current_revision()?.unwrap_or_default();
        let warnings = store.take_commit_warnings(std::slice::from_ref(&committed.op.op_id));
        (committed.op, post_revision, warnings)
    };
    state.events.publish(Event::OpApplied { op: op.clone() });
    let rows = accepted
        .iter()
        .map(|change| {
            json!({
                "asset": change.asset_id,
                "expected_hash": change.expected_hash,
                "chosen": {"path": change.chosen_path, "sha256": change.expected_hash},
                "disposition": "relinked",
            })
        })
        .collect::<Vec<_>>();
    Ok(VerbResult::ok_with_ops(
        json!({
            "schema": RECEIPT_SCHEMA,
            "immutable": true,
            "project_identity": prepared.plan.project_identity,
            "pre_revision": expected_revision,
            "post_revision": post_revision,
            "plan_hash": args.plan_hash,
            "grouped_op_id": op.op_id,
            "assets": rows,
            "scope": {
                "library_transaction": false,
                "import_or_proxy_job": false,
                "undo": "not_promised",
            },
        }),
        vec![op.op_id],
    )
    .with_warnings(warnings))
}

fn required_request_revision(actor: &Actor) -> Result<String, CutError> {
    let request = actor.request.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "bulk relink requires request_id and expected_revision",
            "the immutable grouped receipt needs a retry identity and revision guard",
        )
        .with_suggested_action("preview again, then submit request_id plus expected_revision")
    })?;
    request.expected_revision.clone().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "bulk relink requires expected_revision",
            "a request_id without a revision guard could apply a stale preview",
        )
    })
}

fn stale_preview(expected: &str, actual: &str) -> Result<VerbResult, CutError> {
    Err(CutError::new(
        error_codes::CONFLICT,
        format!("bulk relink expected project revision '{expected}' but found '{actual}'"),
        "the preview is stale; no selected assets were changed",
    )
    .with_suggested_action("refresh media.relink_preview and submit a new request_id"))
}

async fn snapshot(state: &AppState, root: PathBuf) -> Result<RelinkSnapshot, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let revision = store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "open project has no durable revision",
            "reopen the project before recovery",
        )
    })?;
    let mut assets = store
        .project
        .assets
        .iter()
        .map(|(asset_id, asset)| {
            let source_path = source_path(&store.dir, asset);
            let probe = asset.probe.as_ref();
            OfflineAssetSnapshot {
                asset_id: asset_id.clone(),
                expected_hash: asset.hash.clone(),
                old_path: asset.path.clone(),
                source_path,
                display_name: Path::new(&asset.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("media")
                    .to_owned(),
                kind: probe
                    .and_then(|value| value.get("kind"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                duration_ms: probe
                    .and_then(|value| value.get("duration_ms"))
                    .and_then(Value::as_u64),
            }
        })
        .collect::<Vec<_>>();
    assets.sort_by(|left, right| left.asset_id.cmp(&right.asset_id));
    Ok(RelinkSnapshot {
        project_identity: project_identity(store),
        project_revision: revision,
        root,
        assets,
    })
}

fn source_path(project_dir: &Path, asset: &cut_core::Asset) -> PathBuf {
    let path = PathBuf::from(&asset.path);
    if path.is_relative() {
        project_dir.join(path)
    } else {
        path
    }
}

fn project_identity(store: &ProjectStore) -> Value {
    let canonical = store
        .dir
        .canonicalize()
        .unwrap_or_else(|_| store.dir.clone());
    let origin_path_sha256 = format!(
        "sha256:{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    );
    json!({
        "schema": PROJECT_ID_SCHEMA,
        "origin_path_sha256": origin_path_sha256,
        "project_name": store.project.name,
    })
}
