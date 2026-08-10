//! Bounded, path-free inventory of rebuildable project editing caches.

use cut_core::ProjectStore;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

const CACHE_ENTRY_LIMIT: usize = 20_000;
const CACHE_RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Default)]
struct CategoryReport {
    bytes: u64,
    files: usize,
    reclaimable_bytes: u64,
    reclaimable_files: usize,
    aged_unreferenced_bytes: u64,
    aged_unreferenced_files: usize,
    cleanup_blocked: bool,
    scanned_entries: usize,
    skipped_entries: usize,
    truncated: bool,
    latest_modified_ms: Option<u64>,
}

impl CategoryReport {
    fn observe_file(&mut self, path: &Path, reclaimable: bool, now_ms: Option<u64>) {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {
                self.files = self.files.saturating_add(1);
                self.bytes = self.bytes.saturating_add(metadata.len());
                if reclaimable {
                    self.reclaimable_files = self.reclaimable_files.saturating_add(1);
                    self.reclaimable_bytes = self.reclaimable_bytes.saturating_add(metadata.len());
                }
                match metadata.modified() {
                    Ok(modified) => {
                        let millis = modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64);
                        self.latest_modified_ms = self.latest_modified_ms.max(millis);
                        if reclaimable
                            && (millis.is_none()
                                || millis
                                    .zip(now_ms)
                                    .is_some_and(|(modified_ms, now)| modified_ms > now))
                        {
                            self.cleanup_blocked = true;
                        }
                        if reclaimable
                            && millis.is_some_and(|modified_ms| {
                                now_ms.is_some_and(|now| {
                                    now.saturating_sub(modified_ms) >= CACHE_RETENTION_MS
                                        && modified_ms <= now
                                })
                            })
                        {
                            self.aged_unreferenced_files =
                                self.aged_unreferenced_files.saturating_add(1);
                            self.aged_unreferenced_bytes =
                                self.aged_unreferenced_bytes.saturating_add(metadata.len());
                        }
                    }
                    Err(_) => self.skipped_entries = self.skipped_entries.saturating_add(1),
                }
            }
            _ => self.skipped_entries = self.skipped_entries.saturating_add(1),
        }
    }

    fn status(&self) -> &'static str {
        if self.truncated || self.skipped_entries > 0 {
            "partial"
        } else {
            "ready"
        }
    }

    fn value(&self, kind: &str) -> Value {
        let mut value = json!({
            "kind": kind,
            "status": self.status(),
            "bytes": self.bytes,
            "files": self.files,
            "reclaimable_bytes": self.reclaimable_bytes,
            "reclaimable_files": self.reclaimable_files,
            "scanned_entries": self.scanned_entries,
            "skipped_entries": self.skipped_entries,
            "truncated": self.truncated,
            "entry_limit": CACHE_ENTRY_LIMIT,
        });
        if let Some(modified) = self.latest_modified_ms {
            value["latest_modified_ms"] = json!(modified);
        }
        value
    }
}

fn scan_category(
    root: &Path,
    kind: &str,
    referenced: &HashSet<OsString>,
    now_ms: Option<u64>,
) -> CategoryReport {
    let mut report = CategoryReport::default();
    let root_metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(_) => {
            report.skipped_entries = 1;
            return report;
        }
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        report.skipped_entries = 1;
        return report;
    }

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            report.skipped_entries = 1;
            return report;
        }
    };
    for entry in entries {
        if report.scanned_entries >= CACHE_ENTRY_LIMIT {
            report.truncated = true;
            break;
        }
        report.scanned_entries = report.scanned_entries.saturating_add(1);
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report.skipped_entries = report.skipped_entries.saturating_add(1);
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                report.skipped_entries = report.skipped_entries.saturating_add(1);
                continue;
            }
        };
        if file_type.is_file()
            && !file_type.is_symlink()
            && is_generated_cache_name(kind, &entry.file_name())
        {
            report.observe_file(
                &entry.path(),
                !referenced.contains(&entry.file_name()),
                now_ms,
            );
        } else {
            report.skipped_entries = report.skipped_entries.saturating_add(1);
        }
    }
    report
}

fn exact_cache_name(value: &str, directory: &str) -> Option<OsString> {
    let mut components = Path::new(value).components();
    match (components.next(), components.next(), components.next()) {
        (Some(Component::Normal(root)), Some(Component::Normal(name)), None)
            if root == OsStr::new(directory) =>
        {
            Some(name.to_os_string())
        }
        _ => None,
    }
}

fn valid_asset_id(value: &str) -> bool {
    value.strip_prefix('a').is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_generated_cache_name(kind: &str, name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    match kind {
        "proxies" => name.strip_suffix(".mp4").is_some_and(valid_asset_id),
        "thumbnails" => {
            let Some(stem) = name.strip_suffix(".jpg") else {
                return false;
            };
            if valid_asset_id(stem) {
                return true;
            }
            let Some((asset, window)) = stem.split_once("_w") else {
                return false;
            };
            let Some((range, dimensions)) = window.split_once('_') else {
                return false;
            };
            let Some((start, end)) = range.split_once('-') else {
                return false;
            };
            let Some((count, height)) = dimensions.split_once('x') else {
                return false;
            };
            valid_asset_id(asset) && digits(start) && digits(end) && digits(count) && digits(height)
        }
        _ => false,
    }
}

pub(super) fn editing_cache_report(store: &ProjectStore) -> Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64);
    let proxy_references = store
        .project
        .assets
        .values()
        .filter_map(|asset| asset.proxy.as_deref())
        .filter_map(|value| exact_cache_name(value, "proxies"))
        .collect::<HashSet<_>>();
    let thumbnail_references = store
        .project
        .assets
        .values()
        .filter_map(|asset| asset.filmstrip.as_deref())
        .filter_map(|value| exact_cache_name(value, "filmstrip"))
        .collect::<HashSet<_>>();
    let roots: [(&str, PathBuf, &HashSet<OsString>); 2] = [
        ("proxies", store.proxies_dir(), &proxy_references),
        (
            "thumbnails",
            store.dir.join("filmstrip"),
            &thumbnail_references,
        ),
    ];
    let categories = roots
        .iter()
        .map(|(kind, root, referenced)| (kind, scan_category(root, kind, referenced, now_ms)))
        .collect::<Vec<_>>();
    let bytes = categories.iter().fold(0u64, |total, (_, report)| {
        total.saturating_add(report.bytes)
    });
    let files = categories.iter().fold(0usize, |total, (_, report)| {
        total.saturating_add(report.files)
    });
    let reclaimable_bytes = categories.iter().fold(0u64, |total, (_, report)| {
        total.saturating_add(report.reclaimable_bytes)
    });
    let reclaimable_files = categories.iter().fold(0usize, |total, (_, report)| {
        total.saturating_add(report.reclaimable_files)
    });
    let aged_unreferenced_bytes = categories.iter().fold(0u64, |total, (_, report)| {
        total.saturating_add(report.aged_unreferenced_bytes)
    });
    let aged_unreferenced_files = categories.iter().fold(0usize, |total, (_, report)| {
        total.saturating_add(report.aged_unreferenced_files)
    });
    let latest_modified_ms = categories
        .iter()
        .filter_map(|(_, report)| report.latest_modified_ms)
        .max();
    let partial = categories
        .iter()
        .any(|(_, report)| report.status() == "partial");
    let cleanup_blocked = categories.iter().any(|(_, report)| report.cleanup_blocked);
    let mut value = json!({
        "status": if partial { "partial" } else { "ready" },
        "bytes": bytes,
        "files": files,
        "reclaimable_bytes": reclaimable_bytes,
        "reclaimable_files": reclaimable_files,
        "cleanup_preview": {
            "status": if partial || cleanup_blocked || now_ms.is_none() { "blocked" } else { "ready" },
            "minimum_age_ms": CACHE_RETENTION_MS,
            "aged_unreferenced_bytes": aged_unreferenced_bytes,
            "aged_unreferenced_files": aged_unreferenced_files,
            "recent_unreferenced_bytes": reclaimable_bytes.saturating_sub(aged_unreferenced_bytes),
            "recent_unreferenced_files": reclaimable_files.saturating_sub(aged_unreferenced_files),
        },
        "categories": categories
            .iter()
            .map(|(kind, report)| report.value(kind))
            .collect::<Vec<_>>(),
    });
    if let Some(modified) = latest_modified_ms {
        value["latest_modified_ms"] = json!(modified);
    }
    value
}

#[cfg(test)]
mod tests;
