//! B6 portable package planning and publication.
//!
//! This packages a read-only source snapshot into a new Cut-native `.cutproj`
//! for the Projects preview-first flow and direct API. It never falls back to
//! the legacy one-asset relink path or exposes a partially written destination.

use super::project_workspace::path_free_project_identity;
use super::*;
use cut_core::store::{current_relink_receipt, is_exact_sha256, PORTABLE_SNAPSHOT_SCHEMA};
use cut_core::{Clip, Project};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const PACKAGE_PLAN_SCHEMA: &str = "shellx-cut/portable-package-plan/1";
const PACKAGE_MANIFEST_SCHEMA: &str = "shellx-cut/portable-package/1";
const B5_RECEIPT_SCHEMA: &str = "shellx-cut/media-relink-receipt/1";
const PACKAGE_JOB_KIND: &str = "portable_package";
const PACKAGE_STAGE_ATTEMPTS: u32 = 16;
// A metadata-only package should complete promptly. Media packages receive a
// conservative extra second per MiB, with the same thirty-minute ceiling used
// for owned recorder exports. A timed-out worker remains confined to its
// private stage: every publication path checks cancellation before it can
// acquire project state or expose the destination.
const PACKAGE_MIN_TIMEOUT: Duration = Duration::from_secs(15);
const PACKAGE_BYTES_PER_SECOND: u64 = 1024 * 1024;
const PACKAGE_MAX_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(serde::Deserialize)]
struct PlanArgs {
    destination: String,
    name: String,
    #[serde(default)]
    b5_receipt: Option<Value>,
}

#[derive(serde::Deserialize)]
struct CreateArgs {
    destination: String,
    name: String,
    plan_hash: String,
    #[serde(default)]
    b5_receipt: Option<Value>,
}

#[derive(Clone)]
struct PackageSourceSnapshot {
    project: Project,
    project_dir: PathBuf,
    project_identity: Value,
    project_revision: String,
    ops: Vec<OpRecord>,
    current_b5_receipt: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

#[derive(Clone)]
struct PackageSourceFile {
    asset_id: String,
    source: PathBuf,
    stamp: FileStamp,
    bytes: u64,
    sha256: String,
    package_path: String,
}

#[derive(Clone)]
struct PreparedPackage {
    plan: PackagePlan,
    source: PackageSourceSnapshot,
    destination: PathBuf,
    target: PathBuf,
    name: String,
    files: Vec<PackageSourceFile>,
}

#[derive(Debug, Clone, Serialize)]
struct PackagePlan {
    schema: &'static str,
    project_identity: Value,
    project_revision: String,
    destination: String,
    name: String,
    target: String,
    target_status: &'static str,
    b5_receipt_sha256: Option<String>,
    assets: Vec<PackagePlanAsset>,
    total_source_bytes: u64,
    total_package_bytes: u64,
    source_file_count: usize,
    unique_media_count: usize,
    policy: PackagePlanPolicy,
}

#[derive(Debug, Clone, Serialize)]
struct PackagePlanAsset {
    asset: String,
    bytes: u64,
    sha256: String,
    package_path: String,
}

#[derive(Debug, Clone, Serialize)]
struct PackagePlanPolicy {
    only_referenced_media: bool,
    derived_cache: &'static str,
    offline_media: &'static str,
    collision: &'static str,
    publication: &'static str,
}

#[derive(Serialize)]
struct PackageManifest {
    schema: &'static str,
    source: PackageManifestSource,
    policy: PackageManifestPolicy,
    project: PackageManifestProject,
    members: Vec<PackageManifestMember>,
    totals: PackageManifestTotals,
}

#[derive(Serialize)]
struct PackageManifestSource {
    project_identity: Value,
    project_revision: String,
    b5_receipt_sha256: Option<String>,
}

#[derive(Serialize)]
struct PackageManifestPolicy {
    only_referenced_media: bool,
    derived_cache: &'static str,
    offline_media: &'static str,
    collision: &'static str,
}

#[derive(Serialize)]
struct PackageManifestProject {
    project_json: &'static str,
    ops_jsonl: &'static str,
    baseline_schema: &'static str,
}

#[derive(Serialize)]
struct PackageManifestMember {
    path: String,
    role: &'static str,
    bytes: u64,
    sha256: String,
    asset_ids: Vec<String>,
}

#[derive(Serialize)]
struct PackageManifestTotals {
    file_count: usize,
    media_file_count: usize,
    bytes: u64,
}

fn package_timeout(total_source_bytes: u64) -> Duration {
    let transfer_seconds =
        total_source_bytes.saturating_add(PACKAGE_BYTES_PER_SECOND - 1) / PACKAGE_BYTES_PER_SECOND;
    PACKAGE_MIN_TIMEOUT
        .saturating_add(Duration::from_secs(transfer_seconds))
        .min(PACKAGE_MAX_TIMEOUT)
}

/// On timeout, signal the blocking worker and terminalize the job immediately.
/// Waiting for a filesystem call to return would make the Cut runtime itself
/// unavailable. The detached worker can only finish private-stage work; the
/// cancellation gate before publication prevents it from exposing a package
/// or racing a later project transition.
async fn await_bounded_package_work<T: Send + 'static>(
    timeout: Duration,
    work: impl Future<Output = Result<T, CutError>>,
) -> Result<T, CutError> {
    let cancellation = crate::jobs::current_job_cancellation();
    tokio::pin!(work);
    match tokio::time::timeout(timeout, &mut work).await {
        Ok(result) => result,
        Err(_) => {
            cancellation.request_cancel();
            Err(CutError::new(
                error_codes::JOB_FAILED,
                "portable package timed out",
                format!(
                    "project.package_create exceeded its {} second deadline",
                    timeout.as_secs()
                ),
            )
            .with_suggested_action(
                "retry the package; if it persists, check the destination volume and available disk space",
            ))
        }
    }
}

pub(super) async fn project_package_plan(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: PlanArgs = parse_args(args)?;
    let prepared = prepare(state, args.destination, args.name, args.b5_receipt).await?;
    let plan_hash = hash_json(&prepared.plan)?;
    Ok(VerbResult::ok(json!({
        "plan": prepared.plan,
        "plan_hash": plan_hash,
    })))
}

pub(super) async fn project_package_create(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: CreateArgs = parse_args(args)?;
    if !is_exact_sha256(&args.plan_hash) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "portable package plan_hash is not a full SHA-256 value",
            "run project.package_plan and use its unchanged plan_hash",
        ));
    }
    let prepared = prepare(state, args.destination, args.name, args.b5_receipt).await?;
    let actual_plan_hash = hash_json(&prepared.plan)?;
    if actual_plan_hash != args.plan_hash {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package plan is stale",
            "referenced media, source revision, destination, or B5 evidence changed",
        )
        .with_suggested_action("run project.package_plan again; no package was created"));
    }
    if prepared.target.exists() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "portable package destination already exists",
            prepared.target.display().to_string(),
        )
        .with_suggested_action(
            "choose a new package name; this slice never overwrites a package",
        ));
    }

    let job = state.jobs.create(PACKAGE_JOB_KIND);
    let job_id = job.job_id.clone();
    let task_id = job_id.clone();
    let response_source_revision = prepared.source.project_revision.clone();
    let response_target = prepared.target.clone();
    let package_timeout = package_timeout(prepared.plan.total_source_bytes);
    let task_state = state.clone();
    state.jobs.spawn_limited(&job_id, PACKAGE_JOB_KIND, 1, async move {
        task_state
            .jobs
            .progress(&task_id, 0.02, Some("validating portable package destination".into()));
        let job_state = task_state.clone();
        let publication_state = task_state.clone();
        let progress_job_id = task_id.clone();
        let package_source_revision = prepared.source.project_revision.clone();
        let worker = run_blocking_cancellable("project.package_create", move |cancel| {
            publish_package(prepared, &cancel, |progress, message| {
                job_state
                    .jobs
                    .progress(&progress_job_id, progress, Some(message));
            }, |stage, target| {
                source_revision_then_publish(
                    &publication_state,
                    &package_source_revision,
                    &cancel,
                    stage,
                    target,
                )
            })
        });
        let result = match await_bounded_package_work(
            package_timeout,
            worker,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => return task_state.jobs.fail(&task_id, error),
        };

        let payload = json!({
            "status": if result.warnings.is_empty() { "published" } else { "published_with_warnings" },
            "destination": result.destination,
            "manifest_sha256": result.manifest_sha256,
            "source_revision": result.source_revision,
            "file_count": result.file_count,
            "bytes": result.bytes,
            "warnings": result.warnings,
        });
        if payload["warnings"].as_array().is_some_and(|warnings| warnings.is_empty()) {
            task_state.jobs.finish(&task_id, payload);
        } else {
            task_state.jobs.finish_with_warnings(&task_id, payload);
        }
    });
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "plan_hash": actual_plan_hash,
        "source_revision": response_source_revision,
        "target": response_target,
        "status": "queued",
    })))
}

include!("project_package/plan.rs");
include!("project_package/b5_evidence.rs");
include!("project_package/source_io.rs");
include!("project_package/publish.rs");
include!("project_package/manifest.rs");
include!("project_package/stage.rs");
include!("project_package/tests.rs");
