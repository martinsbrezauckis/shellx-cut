//! Disposable, journal-verified materialized replay snapshots.
//!
//! Snapshots are never journal truth. Each records the exact chained identity
//! hash of its log prefix and a checksum of its materialized project; a failed
//! read or validation simply falls back to replaying `ops.jsonl`.

use super::*;
use crate::ops::JournalView;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

const SNAPSHOT_SCHEMA: &str = "shellx-cut/history-snapshot/1";
pub(super) const SNAPSHOT_INTERVAL: usize = 4_096;
const MAX_SNAPSHOTS: usize = 8;
const SNAPSHOT_DIR: &str = ".history-snapshots";

#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    schema: String,
    prefix_len: usize,
    prefix_hash: String,
    project_hash: String,
    project: Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReplayStats {
    pub(super) snapshot_prefix: usize,
    pub(super) replayed_ops: usize,
    pub(super) rejected_snapshot: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct TestReplayMetrics {
    pub(super) rebuilds: usize,
    pub(super) replayed_ops: usize,
}

#[cfg(test)]
static TEST_REBUILDS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static TEST_REPLAYED_OPS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(super) fn reset_test_replay_metrics() {
    TEST_REBUILDS.store(0, Ordering::Relaxed);
    TEST_REPLAYED_OPS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(super) fn test_replay_metrics() -> TestReplayMetrics {
    TestReplayMetrics {
        rebuilds: TEST_REBUILDS.load(Ordering::Relaxed),
        replayed_ops: TEST_REPLAYED_OPS.load(Ordering::Relaxed),
    }
}

/// Rebuild one journal prefix from the nearest verified snapshot plus its
/// suffix. The caller supplies the whole log so legacy restore records retain
/// access to their original prefix during suffix replay.
pub(super) fn rebuild(
    dir: &Path,
    journal: &JournalView,
    prefix_len: usize,
) -> Result<(Project, ReplayStats), CutError> {
    let ops = journal.records();
    if prefix_len > ops.len() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "requested replay prefix exceeds the operation journal",
            format!(
                "requested {prefix_len} records from a {}-record journal",
                ops.len()
            ),
        ));
    }
    let (snapshot, rejected_snapshot) = nearest_verified_snapshot(dir, prefix_len, journal)?;
    let (mut project, snapshot_prefix) = match snapshot {
        Some(snapshot) => (snapshot.project, snapshot.prefix_len),
        None => (Project::new("", ProjectSettings::default()), 0),
    };
    for index in snapshot_prefix..prefix_len {
        apply_record(&mut project, &ops[index], &ops[..index])?;
        project.sync_active_sequence();
    }
    let stats = ReplayStats {
        snapshot_prefix,
        replayed_ops: prefix_len - snapshot_prefix,
        rejected_snapshot,
    };
    #[cfg(test)]
    {
        TEST_REBUILDS.fetch_add(1, Ordering::Relaxed);
        TEST_REPLAYED_OPS.fetch_add(stats.replayed_ops, Ordering::Relaxed);
    }
    Ok((project, stats))
}

/// Write a snapshot only at a fixed journal interval. Failure is deliberately
/// ignored by callers: the append is already durable and full replay is safe.
// Keep the modulo form for the workspace's Rust 1.74 compatibility; the
// clippy-preferred integer helper arrived after that baseline.
#[allow(clippy::manual_is_multiple_of)]
pub(super) fn write_if_due(dir: &Path, log: &OpLog, project: &Project) {
    let Ok(journal) = log.replay_view() else {
        return;
    };
    let journal_len = journal.records().len();
    if journal_len == 0 || journal_len % SNAPSHOT_INTERVAL != 0 {
        return;
    }
    let _ = write(dir, &journal, project);
}

/// Refresh the current-prefix snapshot. Used after a rejected cache so the
/// next open can use a verified materialization immediately.
pub(super) fn write(dir: &Path, journal: &JournalView, project: &Project) -> Result<(), CutError> {
    let prefix_len = journal.records().len();
    if prefix_len == 0 {
        return Ok(());
    }
    let prefix_hash = journal
        .prefix_hash(prefix_len)
        .expect("journal view always contains its full prefix identity");
    let snapshot = Snapshot {
        schema: SNAPSHOT_SCHEMA.into(),
        prefix_len,
        prefix_hash,
        project_hash: project_hash(project)?,
        project: project.clone(),
    };
    let root = snapshot_dir(dir);
    std::fs::create_dir_all(&root)?;
    let path = snapshot_path(&root, snapshot.prefix_len);
    atomic_write(&path, &serde_json::to_vec_pretty(&snapshot)?)?;
    prune_old_snapshots(&root);
    Ok(())
}

fn nearest_verified_snapshot(
    dir: &Path,
    prefix_len: usize,
    journal: &JournalView,
) -> Result<(Option<Snapshot>, bool), CutError> {
    let mut rejected = false;
    let mut nearest: Option<Snapshot> = None;
    for path in snapshot_paths(&snapshot_dir(dir)) {
        let Ok(bytes) = std::fs::read(&path) else {
            rejected = true;
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let Ok(snapshot) = serde_json::from_slice::<Snapshot>(&bytes) else {
            rejected = true;
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let Ok(actual_project_hash) = project_hash(&snapshot.project) else {
            rejected = true;
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if snapshot.schema != SNAPSHOT_SCHEMA
            || snapshot.prefix_len == 0
            || snapshot.project_hash != actual_project_hash
        {
            rejected = true;
            let _ = std::fs::remove_file(&path);
            continue;
        }
        // A newer snapshot cannot serve this older undo target, but is not
        // corrupt: keep it for a later, longer-prefix replay.
        if snapshot.prefix_len > prefix_len {
            continue;
        }
        if snapshot.prefix_hash
            != journal
                .prefix_hash(snapshot.prefix_len)
                .expect("verified prefix length is within the journal view")
        {
            rejected = true;
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let is_nearer = match nearest.as_ref() {
            Some(current) => current.prefix_len < snapshot.prefix_len,
            None => true,
        };
        if snapshot.prefix_len <= prefix_len && is_nearer {
            nearest = Some(snapshot);
        }
    }
    Ok((nearest, rejected))
}

fn project_hash(project: &Project) -> Result<String, CutError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(project)?)
    ))
}

fn snapshot_dir(dir: &Path) -> PathBuf {
    dir.join(SNAPSHOT_DIR)
}

fn snapshot_path(root: &Path, prefix_len: usize) -> PathBuf {
    root.join(format!("snapshot-{prefix_len:012}.json"))
}

fn snapshot_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.starts_with("snapshot-") && name.ends_with(".json")).then(|| entry.path())
        })
        .collect();
    paths.sort();
    paths
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CutError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(".snapshot-{}.{}.tmp", std::process::id(), nonce));
    let result = (|| -> Result<(), CutError> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        if let Ok(parent) = std::fs::File::open(parent) {
            let _ = parent.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn prune_old_snapshots(root: &Path) {
    let paths = snapshot_paths(root);
    for path in paths.into_iter().rev().skip(MAX_SNAPSHOTS) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests;
