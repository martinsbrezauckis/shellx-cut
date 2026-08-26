//! Bounded inventory and path-free purge preview construction.

use super::ownership::{cache_root, identity, key, read_ledger, OwnershipLedger};
use super::*;
use cut_core::ProjectStore;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path};

pub(super) struct Preview {
    pub(super) public: Value,
    pub(super) plan: Option<CachePurgePlan>,
}

fn referenced_names(store: &ProjectStore, kind: CacheKind) -> HashSet<OsString> {
    let directory = kind.directory();
    store
        .project
        .assets
        .values()
        .filter_map(|asset| match kind {
            CacheKind::Proxies => asset.proxy.as_deref(),
            CacheKind::Thumbnails => asset.filmstrip.as_deref(),
        })
        .filter_map(|value| {
            let mut components = Path::new(value).components();
            match (components.next(), components.next(), components.next()) {
                (Some(Component::Normal(root)), Some(Component::Normal(name)), None)
                    if root == OsStr::new(directory) =>
                {
                    Some(name.to_os_string())
                }
                _ => None,
            }
        })
        .collect()
}

pub(super) fn scan(
    store: &ProjectStore,
    ledger: &OwnershipLedger,
    now: u64,
) -> Result<
    (
        Vec<FileIdentity>,
        Vec<RootIdentity>,
        Vec<FileIdentity>,
        [CategoryCount; 2],
    ),
    CutError,
> {
    let project_dir = &store.dir;
    let mut snapshot = Vec::new();
    let mut roots = Vec::new();
    let mut targets = Vec::new();
    let mut counts = [CategoryCount::default(), CategoryCount::default()];
    for (index, kind) in [CacheKind::Proxies, CacheKind::Thumbnails]
        .into_iter()
        .enumerate()
    {
        let root = cache_root(project_dir, kind)?;
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                roots.push(RootIdentity {
                    kind,
                    canonical: root,
                });
                continue;
            }
            Err(_) => {
                return Err(cache_error(
                    "cache ownership cannot be verified",
                    "a cache root cannot be inspected",
                ))
            }
            Ok(_) => {}
        }
        let canonical = std::fs::canonicalize(&root).map_err(|_| {
            cache_error(
                "cache ownership cannot be verified",
                "a cache root cannot be resolved",
            )
        })?;
        roots.push(RootIdentity { kind, canonical });
        let referenced = referenced_names(store, kind);
        let entries = std::fs::read_dir(&root).map_err(|_| {
            cache_error(
                "cache ownership cannot be verified",
                "a cache root cannot be listed",
            )
        })?;
        let mut total = 0usize;
        for entry in entries {
            total = total.saturating_add(1);
            if total > CACHE_ENTRY_LIMIT {
                return Err(cache_error(
                    "cache ownership cannot be verified",
                    "a cache root exceeds the bounded cleanup inventory",
                ));
            }
            let entry = entry.map_err(|_| {
                cache_error(
                    "cache ownership cannot be verified",
                    "a cache entry cannot be listed",
                )
            })?;
            let name = entry.file_name();
            let file = identity(kind, name.clone(), &entry.path())?;
            let entry_key = key(kind, &name).ok_or_else(|| {
                cache_error(
                    "cache ownership cannot be verified",
                    "a cache filename is not valid text",
                )
            })?;
            let owned = ledger.entries.get(&entry_key).is_some_and(|record| {
                record.kind == kind.directory() && record.name == name.to_string_lossy()
            });
            if !owned {
                return Err(cache_error(
                    "cache ownership cannot be verified",
                    "a cache root contains an unowned, legacy, or unexpected entry",
                ));
            }
            counts[index].files = counts[index].files.saturating_add(1);
            counts[index].bytes = counts[index].bytes.saturating_add(file.bytes);
            if !referenced.contains(&name)
                && now.saturating_sub(file.modified_ms) >= CACHE_RETENTION_MS
                && file.modified_ms <= now
            {
                counts[index].purgeable_files = counts[index].purgeable_files.saturating_add(1);
                counts[index].purgeable_bytes =
                    counts[index].purgeable_bytes.saturating_add(file.bytes);
                targets.push(file.clone());
            }
            snapshot.push(file);
        }
    }
    let snapshot_keys = snapshot
        .iter()
        .filter_map(|file| key(file.kind, &file.name))
        .collect::<HashSet<_>>();
    if ledger
        .entries
        .keys()
        .any(|entry| !snapshot_keys.contains(entry))
    {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "the cache ownership ledger has entries without matching cache files",
        ));
    }
    Ok((snapshot, roots, targets, counts))
}

pub(super) fn build_preview(store: &ProjectStore, plan_id: String) -> Result<Preview, CutError> {
    let now = now_ms()?;
    let ledger = read_ledger(&store.dir)?;
    let (snapshot, roots, targets, counts) = scan(store, &ledger, now)?;
    let (project_revision, _) = store.log.current_revision_and_count()?;
    let total_files = counts
        .iter()
        .fold(0u64, |total, category| total.saturating_add(category.files));
    let total_bytes = counts
        .iter()
        .fold(0u64, |total, category| total.saturating_add(category.bytes));
    let purgeable_files = counts.iter().fold(0u64, |total, category| {
        total.saturating_add(category.purgeable_files)
    });
    let purgeable_bytes = counts.iter().fold(0u64, |total, category| {
        total.saturating_add(category.purgeable_bytes)
    });
    let public = json!({
        "schema": "shellx-cut/cache-purge-preview/1",
        "status": "ready",
        "plan_id": plan_id,
        "minimum_age_ms": CACHE_RETENTION_MS,
        "inventory": {"files": total_files, "bytes": total_bytes},
        "purgeable": {"files": purgeable_files, "bytes": purgeable_bytes},
        "categories": [
            {"kind": "proxies", "files": counts[0].files, "bytes": counts[0].bytes, "purgeable_files": counts[0].purgeable_files, "purgeable_bytes": counts[0].purgeable_bytes},
            {"kind": "thumbnails", "files": counts[1].files, "bytes": counts[1].bytes, "purgeable_files": counts[1].purgeable_files, "purgeable_bytes": counts[1].purgeable_bytes}
        ],
        "blocked_reasons": []
    });
    Ok(Preview {
        public,
        plan: Some(CachePurgePlan {
            plan_id,
            project_dir: store.dir.clone(),
            project_revision,
            created_ms: now,
            roots,
            snapshot,
            targets,
        }),
    })
}
