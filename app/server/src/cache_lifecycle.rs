//! Durable ownership and fail-closed cleanup for rebuildable editing caches.
//!
//! The lifecycle is deliberately limited to the flat project-local proxy and
//! filmstrip roots. It never walks, follows, or names source media, exports,
//! captures, receipts, or arbitrary project files.

mod inventory;
mod ownership;
mod purge;
mod rebuild;
#[cfg(test)]
mod tests;

pub(crate) use ownership::{
    pending_rebuild_outputs_for_asset, record_generated, remove_owned_output, OwnedRemoval,
};
pub(crate) use purge::{preview, start_purge};
pub(crate) use rebuild::start_rebuild;

use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) const CACHE_RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;
const CACHE_ENTRY_LIMIT: usize = 20_000;
const LEDGER_NAME: &str = ".shellx-cut-cache-ownership.json";
const LEDGER_SCHEMA: &str = "shellx-cut/cache-ownership/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CacheKind {
    Proxies,
    Thumbnails,
}

/// In-memory admission record for the one deterministic cache rebuild owned by
/// this server run. The durable ledger is the restart boundary; this only keeps
/// duplicate requests and conflicting asset mutations from racing a live worker.
#[derive(Debug, Clone)]
pub(crate) struct CacheRebuildActive {
    pub(crate) job_id: String,
    pub(crate) assets: Vec<String>,
    pub(crate) scheduled_outputs: usize,
    /// The same path-free admission summary returned to the first scheduler.
    /// Keeping it only while the job is active lets a repeated human request
    /// explain the already-queued work without scanning or inventing counts.
    pub(crate) counts: serde_json::Value,
    /// Exact work-unit estimate from the same source-identity admission pass
    /// that created this worker. A duplicate request must repeat that result,
    /// rather than recomputing a potentially different cost while work runs.
    pub(crate) estimate: serde_json::Value,
}

/// Path-free cache measurement used to reconcile a confirmed purge. This is
/// intentionally only a count and byte total: cache filenames and paths stay
/// inside the private plan/ledger boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheMeasurement {
    pub(crate) files: u64,
    pub(crate) bytes: u64,
}

impl CacheMeasurement {
    pub(crate) fn public(self) -> serde_json::Value {
        serde_json::json!({"files": self.files, "bytes": self.bytes})
    }
}

impl CacheKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Proxies => "proxies",
            Self::Thumbnails => "filmstrip",
        }
    }

    fn key_prefix(self) -> &'static str {
        match self {
            Self::Proxies => "proxies/",
            Self::Thumbnails => "filmstrip/",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileIdentity {
    kind: CacheKind,
    name: OsString,
    bytes: u64,
    modified_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootIdentity {
    kind: CacheKind,
    canonical: PathBuf,
}

/// Stored only in process memory and identified publicly by `plan_id`.
/// Individual paths and filenames never enter a verb result.
#[derive(Debug, Clone)]
pub(crate) struct CachePurgePlan {
    plan_id: String,
    project_dir: PathBuf,
    project_revision: Option<String>,
    created_ms: u64,
    roots: Vec<RootIdentity>,
    snapshot: Vec<FileIdentity>,
    targets: Vec<FileIdentity>,
    /// Exact strict inventory measurement at preview time. The purge worker
    /// holds the exclusive lease through deletion and post-measurement so it
    /// can report a balanced before/removed/after reconciliation.
    before: CacheMeasurement,
}

#[derive(Default)]
pub(crate) struct CategoryCount {
    files: u64,
    bytes: u64,
    purgeable_files: u64,
    purgeable_bytes: u64,
}

pub(crate) fn total_cache_measurement(counts: &[CategoryCount; 2]) -> CacheMeasurement {
    CacheMeasurement {
        files: counts
            .iter()
            .fold(0u64, |total, category| total.saturating_add(category.files)),
        bytes: counts
            .iter()
            .fold(0u64, |total, category| total.saturating_add(category.bytes)),
    }
}

fn cache_error(message: impl Into<String>, cause: impl Into<String>) -> CutError {
    CutError::new(error_codes::CONFLICT, message, cause)
}

pub(crate) fn cache_busy_error() -> CutError {
    cache_error(
        "editing cache is busy",
        "a proxy or filmstrip producer is publishing cache output; wait for it to finish, then preview again",
    )
    .with_suggested_action(
        "wait for active media jobs to settle, then call project.cache_preview again",
    )
}

fn now_ms() -> Result<u64, CutError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .map_err(|_| {
            cache_error(
                "cache clock is unavailable",
                "system time is before the Unix epoch",
            )
        })
}
