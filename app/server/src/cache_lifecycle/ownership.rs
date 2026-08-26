//! Exact cache-root and ownership-ledger validation.

use super::*;
use crate::output_paths::write_output_atomic;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OwnershipLedger {
    schema: String,
    #[serde(default)]
    pub(super) entries: BTreeMap<String, LedgerEntry>,
}

impl Default for OwnershipLedger {
    fn default() -> Self {
        Self {
            schema: LEDGER_SCHEMA.into(),
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LedgerEntry {
    pub(super) kind: String,
    asset: String,
    pub(super) name: String,
}

pub(super) fn cache_root(project_dir: &Path, kind: CacheKind) -> Result<PathBuf, CutError> {
    let project_meta = std::fs::symlink_metadata(project_dir).map_err(|_| {
        cache_error(
            "cache ownership cannot be verified",
            "the open project directory is unavailable",
        )
    })?;
    if project_meta.file_type().is_symlink() || !project_meta.is_dir() {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "the open project directory is not a plain directory",
        ));
    }
    let root = project_dir.join(kind.directory());
    match std::fs::symlink_metadata(&root) {
        Ok(meta) if !meta.file_type().is_symlink() && meta.is_dir() => Ok(root),
        Ok(_) => Err(cache_error(
            "cache ownership cannot be verified",
            "a cache root is not a plain directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(root),
        Err(_) => Err(cache_error(
            "cache ownership cannot be verified",
            "a cache root cannot be inspected",
        )),
    }
}

fn ledger_path(project_dir: &Path) -> Result<PathBuf, CutError> {
    let _ = cache_root(project_dir, CacheKind::Proxies)?;
    let path = project_dir.join(LEDGER_NAME);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_symlink() && meta.is_file() => Ok(path),
        Ok(_) => Err(cache_error(
            "cache ownership cannot be verified",
            "the cache ownership ledger is not a plain file",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(_) => Err(cache_error(
            "cache ownership cannot be verified",
            "the cache ownership ledger cannot be inspected",
        )),
    }
}

pub(super) fn read_ledger(project_dir: &Path) -> Result<OwnershipLedger, CutError> {
    let path = ledger_path(project_dir)?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OwnershipLedger::default())
        }
        Err(_) => {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "the cache ownership ledger cannot be read",
            ))
        }
    };
    let ledger: OwnershipLedger = serde_json::from_slice(&bytes).map_err(|_| {
        cache_error(
            "cache ownership cannot be verified",
            "the cache ownership ledger is malformed",
        )
    })?;
    if ledger.schema != LEDGER_SCHEMA {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "the cache ownership ledger schema is unknown",
        ));
    }
    Ok(ledger)
}

pub(super) fn write_ledger(project_dir: &Path, ledger: &OwnershipLedger) -> Result<(), CutError> {
    let path = ledger_path(project_dir)?;
    let bytes = serde_json::to_vec_pretty(ledger).map_err(|_| {
        cache_error(
            "cache ownership cannot be recorded",
            "the ownership ledger could not be encoded",
        )
    })?;
    write_output_atomic(&path, bytes).map_err(|_| {
        cache_error(
            "cache ownership cannot be recorded",
            "the ownership ledger could not be published",
        )
    })
}

fn valid_asset_id(value: &str) -> bool {
    value.strip_prefix('a').is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_cache_name(kind: CacheKind, name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    match kind {
        CacheKind::Proxies => name.strip_suffix(".mp4").is_some_and(valid_asset_id),
        CacheKind::Thumbnails => {
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
    }
}

pub(super) fn key(kind: CacheKind, name: &OsStr) -> Option<String> {
    name.to_str()
        .map(|name| format!("{}{}", kind.key_prefix(), name))
}

pub(super) fn identity(
    kind: CacheKind,
    name: OsString,
    path: &Path,
) -> Result<FileIdentity, CutError> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| {
        cache_error(
            "cache changed while it was being verified",
            "a cache entry disappeared or cannot be inspected",
        )
    })?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "a cache entry is not a plain file",
        ));
    }
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .ok_or_else(|| {
            cache_error(
                "cache ownership cannot be verified",
                "a cache entry has no usable modification time",
            )
        })?;
    Ok(FileIdentity {
        kind,
        name,
        bytes: meta.len(),
        modified_ms,
    })
}

/// Record a generated output before asset metadata begins referring to it.
/// The caller holds `AppState::cache_lifecycle_lease` in shared mode.
pub(crate) fn record_generated(
    project_dir: &Path,
    kind: CacheKind,
    asset: &str,
    filename: &OsStr,
) -> Result<(), CutError> {
    if !valid_asset_id(asset) || !valid_cache_name(kind, filename) {
        return Err(cache_error(
            "cache ownership cannot be recorded",
            "the generated cache filename or asset id is outside the cache contract",
        ));
    }
    let root = cache_root(project_dir, kind)?;
    let path = root.join(filename);
    let _ = identity(kind, filename.to_os_string(), &path)?;
    let entry_key = key(kind, filename).ok_or_else(|| {
        cache_error(
            "cache ownership cannot be recorded",
            "the generated cache filename is not valid text",
        )
    })?;
    let mut ledger = read_ledger(project_dir)?;
    ledger.entries.insert(
        entry_key,
        LedgerEntry {
            kind: kind.directory().into(),
            asset: asset.into(),
            name: filename.to_string_lossy().into_owned(),
        },
    );
    write_ledger(project_dir, &ledger)
}
