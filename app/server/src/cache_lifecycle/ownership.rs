//! Exact cache-root and ownership-ledger validation.

use super::*;
use crate::output_paths::write_output_atomic;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path};

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
    pub(super) asset: String,
    pub(super) name: String,
    /// Legacy entries predate deterministic rebuild reservations and therefore
    /// deserialize as ready without provenance. They remain purgeable under the
    /// original lifecycle, but are never silently adopted as a fresh rebuild.
    #[serde(default)]
    pub(super) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) source_hash: Option<String>,
    /// A pending reservation with this flag false still has an older final
    /// output to retire. A completed worker only ever leaves a final file after
    /// it has flipped this durable marker to true.
    #[serde(default)]
    pub(super) output_retired: bool,
}

impl LedgerEntry {
    pub(super) fn is_ready(&self) -> bool {
        self.state.is_empty() || self.state == "ready"
    }

    fn is_pending(&self) -> bool {
        self.state == "pending"
    }

    fn matches(&self, kind: CacheKind, asset: &str, name: &str) -> bool {
        self.kind == kind.directory() && self.asset == asset && self.name == name
    }
}

/// The durable-cache outcome after a caller has committed an asset mutation.
///
/// A ledger-publish failure is deliberately distinct from pre-unlink failures:
/// the derived file is already gone in that case, while the stale ledger entry
/// remains visible and blocks later cache inventory until it is repaired.
#[derive(Debug)]
pub(crate) enum OwnedRemoval {
    Retired,
    LedgerRetiredMissing,
    UnlinkedLedgerPending(CutError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RebuildOutputState {
    /// A present, ready entry carries the exact current source identity.
    ReadyVerified,
    /// A present legacy entry has ownership but no source provenance.
    ReadyLegacy,
    /// A durable reservation for this source survived cancellation or restart.
    Pending,
    /// The output is absent, or an owned output belongs to another source.
    MissingOrStale,
    /// A recognized filename exists without an ownership ledger record.
    UnownedPresent,
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

fn valid_cache_name_for_asset(kind: CacheKind, asset: &str, name: &OsStr) -> bool {
    if !valid_asset_id(asset) || !valid_cache_name(kind, name) {
        return false;
    }
    let Some(name) = name.to_str() else {
        return false;
    };
    match kind {
        CacheKind::Proxies => name == format!("{asset}.mp4"),
        CacheKind::Thumbnails => {
            name == format!("{asset}.jpg") || name.starts_with(&format!("{asset}_w"))
        }
    }
}

fn flat_output_name(kind: CacheKind, asset: &str, relative: &str) -> Result<OsString, CutError> {
    let mut components = Path::new(relative).components();
    let name = match (components.next(), components.next(), components.next()) {
        (Some(Component::Normal(root)), Some(Component::Normal(name)), None)
            if root == OsStr::new(kind.directory()) =>
        {
            name.to_os_string()
        }
        _ => {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "the asset cache reference is not a flat recognized cache output",
            ))
        }
    };
    if !valid_cache_name_for_asset(kind, asset, &name) {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "the asset cache reference does not match its asset and cache kind",
        ));
    }
    Ok(name)
}

pub(super) fn key(kind: CacheKind, name: &OsStr) -> Option<String> {
    name.to_str()
        .map(|name| format!("{}{}", kind.key_prefix(), name))
}

fn matching_ledger_entry<'a>(
    ledger: &'a OwnershipLedger,
    entry_key: &str,
    kind: CacheKind,
    asset: &str,
    name: &str,
) -> Result<&'a LedgerEntry, CutError> {
    let entry = ledger.entries.get(entry_key).ok_or_else(|| {
        cache_error(
            "cache ownership cannot be verified",
            "the cache output has no ownership ledger entry",
        )
    })?;
    if !entry.matches(kind, asset, name) {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "the cache output ownership ledger entry does not match the asset",
        ));
    }
    Ok(entry)
}

pub(super) fn base_output_name(kind: CacheKind, asset: &str) -> Result<OsString, CutError> {
    if !valid_asset_id(asset) {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "the asset id cannot own a recognized cache output",
        ));
    }
    Ok(match kind {
        CacheKind::Proxies => format!("{asset}.mp4").into(),
        CacheKind::Thumbnails => format!("{asset}.jpg").into(),
    })
}

pub(super) fn relative_output(kind: CacheKind, asset: &str) -> Result<String, CutError> {
    let name = base_output_name(kind, asset)?;
    Ok(format!("{}/{}", kind.directory(), name.to_string_lossy()))
}

/// Inspect only the exact base output that a deterministic rebuild may own.
/// This never claims a file: a present filename without a matching ledger
/// record remains unowned and blocks rebuild admission.
pub(super) fn rebuild_output_state(
    project_dir: &Path,
    kind: CacheKind,
    asset: &str,
    source_hash: &str,
) -> Result<RebuildOutputState, CutError> {
    let name = base_output_name(kind, asset)?;
    let name_text = name.to_string_lossy();
    let root = cache_root(project_dir, kind)?;
    let path = root.join(&name);
    let exists = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(cache_error(
                    "cache ownership cannot be verified",
                    "a rebuildable cache output is not a plain file",
                ));
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "a rebuildable cache output cannot be inspected",
            ))
        }
    };
    let ledger = read_ledger(project_dir)?;
    let entry_key = key(kind, &name).expect("validated cache names are text");
    let Some(entry) = ledger.entries.get(&entry_key) else {
        return Ok(if exists {
            RebuildOutputState::UnownedPresent
        } else {
            RebuildOutputState::MissingOrStale
        });
    };
    if !entry.matches(kind, asset, &name_text) {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "a rebuildable cache output ownership record does not match its asset",
        ));
    }
    if entry.is_pending() && entry.source_hash.as_deref() == Some(source_hash) {
        return Ok(RebuildOutputState::Pending);
    }
    if entry.is_ready() && entry.source_hash.as_deref() == Some(source_hash) && exists {
        return Ok(RebuildOutputState::ReadyVerified);
    }
    if entry.is_ready() && entry.source_hash.is_none() && exists {
        return Ok(RebuildOutputState::ReadyLegacy);
    }
    Ok(RebuildOutputState::MissingOrStale)
}

/// Publish a restart-safe reservation before a rebuild can create its final
/// filename. If an old ledger-owned output exists, mark it pending first, then
/// retire that exact file before a replacement is admitted. A crash at any
/// point leaves a pending ledger entry, never an unowned cache output.
pub(super) fn reserve_rebuild_output(
    project_dir: &Path,
    kind: CacheKind,
    asset: &str,
    source_hash: &str,
) -> Result<(), CutError> {
    let name = base_output_name(kind, asset)?;
    let name_text = name.to_string_lossy().into_owned();
    let entry_key = key(kind, &name).expect("validated cache names are text");
    let root = cache_root(project_dir, kind)?;
    let path = root.join(&name);
    let mut ledger = read_ledger(project_dir)?;
    let current = ledger.entries.get(&entry_key).cloned();
    if let Some(entry) = &current {
        if !entry.matches(kind, asset, &name_text) {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "a rebuild reservation would replace another asset's cache output",
            ));
        }
    } else if std::fs::symlink_metadata(&path).is_ok() {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "a rebuildable cache filename exists without an ownership record",
        ));
    }

    let already_current_pending = current.as_ref().is_some_and(|entry| {
        entry.is_pending()
            && entry.source_hash.as_deref() == Some(source_hash)
            && entry.output_retired
    });
    if already_current_pending {
        return Ok(());
    }

    ledger.entries.insert(
        entry_key.clone(),
        LedgerEntry {
            kind: kind.directory().into(),
            asset: asset.into(),
            name: name_text.clone(),
            state: "pending".into(),
            source_hash: Some(source_hash.into()),
            output_retired: false,
        },
    );
    write_ledger(project_dir, &ledger)?;

    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(cache_error(
                    "cache ownership cannot be verified",
                    "a rebuildable cache output is not a plain file",
                ));
            }
            std::fs::remove_file(&path).map_err(|_| {
                cache_error(
                    "cache output could not be retired for rebuild",
                    "the exact ledger-owned cache file could not be removed",
                )
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "a rebuildable cache output cannot be inspected",
            ))
        }
    }
    let entry = ledger
        .entries
        .get_mut(&entry_key)
        .expect("reservation was just inserted");
    entry.output_retired = true;
    write_ledger(project_dir, &ledger)
}

/// Promote the exact pending reservation after the worker has published a
/// complete plain output and has rechecked the unchanged source identity.
pub(super) fn complete_rebuild_output(
    project_dir: &Path,
    kind: CacheKind,
    asset: &str,
    source_hash: &str,
) -> Result<(), CutError> {
    let name = base_output_name(kind, asset)?;
    let name_text = name.to_string_lossy();
    let entry_key = key(kind, &name).expect("validated cache names are text");
    let root = cache_root(project_dir, kind)?;
    let _ = identity(kind, name.clone(), &root.join(&name))?;
    let mut ledger = read_ledger(project_dir)?;
    let entry = ledger.entries.get_mut(&entry_key).ok_or_else(|| {
        cache_error(
            "cache rebuild reservation is no longer current",
            "the pending ownership record disappeared before the output could be published",
        )
    })?;
    if !entry.matches(kind, asset, &name_text)
        || !entry.is_pending()
        || entry.source_hash.as_deref() != Some(source_hash)
        || !entry.output_retired
    {
        return Err(cache_error(
            "cache rebuild reservation is no longer current",
            "the pending ownership record changed before the output could be published",
        ));
    }
    entry.state = "ready".into();
    write_ledger(project_dir, &ledger)
}

/// Remove a failed source-identity reservation and only its exact pending
/// output. Cancellation intentionally does not call this: its pending record
/// is the durable resume point after restart.
pub(super) fn abandon_rebuild_output(
    project_dir: &Path,
    kind: CacheKind,
    asset: &str,
    source_hash: &str,
) -> Result<(), CutError> {
    let name = base_output_name(kind, asset)?;
    let name_text = name.to_string_lossy();
    let entry_key = key(kind, &name).expect("validated cache names are text");
    let root = cache_root(project_dir, kind)?;
    let path = root.join(&name);
    let mut ledger = read_ledger(project_dir)?;
    let entry = ledger.entries.get(&entry_key).ok_or_else(|| {
        cache_error(
            "cache rebuild reservation is no longer current",
            "the pending ownership record disappeared before cleanup",
        )
    })?;
    if !entry.matches(kind, asset, &name_text)
        || !entry.is_pending()
        || entry.source_hash.as_deref() != Some(source_hash)
    {
        return Err(cache_error(
            "cache rebuild reservation is no longer current",
            "the pending ownership record changed before cleanup",
        ));
    }
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => {
            std::fs::remove_file(&path).map_err(|_| {
                cache_error(
                    "cache rebuild output could not be discarded",
                    "the exact pending cache file could not be removed",
                )
            })?;
        }
        Ok(_) => {
            return Err(cache_error(
                "cache rebuild output could not be discarded",
                "the pending cache path is no longer a plain file",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(cache_error(
                "cache rebuild output could not be discarded",
                "the pending cache path cannot be inspected",
            ))
        }
    }
    ledger.entries.remove(&entry_key);
    write_ledger(project_dir, &ledger)
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
    if !valid_cache_name_for_asset(kind, asset, filename) {
        return Err(cache_error(
            "cache ownership cannot be recorded",
            "the generated cache filename does not match its asset and cache kind",
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
    let existing = ledger.entries.get(&entry_key).cloned();
    if let Some(entry) = &existing {
        if !entry.matches(kind, asset, &filename.to_string_lossy()) {
            return Err(cache_error(
                "cache ownership cannot be recorded",
                "the generated cache filename already has a different ownership record",
            ));
        }
    }
    ledger.entries.insert(
        entry_key,
        LedgerEntry {
            kind: kind.directory().into(),
            asset: asset.into(),
            name: filename.to_string_lossy().into_owned(),
            state: "ready".into(),
            source_hash: existing.and_then(|entry| entry.source_hash),
            output_retired: true,
        },
    );
    write_ledger(project_dir, &ledger)
}

/// Remove one exact, ledger-owned proxy or filmstrip after a project mutation.
///
/// The caller holds `AppState::cache_lifecycle_lease` in exclusive mode and never
/// holds the project write guard. Validation happens before unlink so a stale or
/// crafted asset pointer cannot remove another asset's cache output. The ledger
/// record is retired only after that exact plain file was removed.
pub(crate) fn remove_owned_output(
    project_dir: &Path,
    kind: CacheKind,
    asset: &str,
    relative: &str,
) -> Result<OwnedRemoval, CutError> {
    let name = flat_output_name(kind, asset, relative)?;
    let name_text = name.to_str().ok_or_else(|| {
        cache_error(
            "cache ownership cannot be verified",
            "the asset cache filename is not valid text",
        )
    })?;
    let entry_key = key(kind, &name).ok_or_else(|| {
        cache_error(
            "cache ownership cannot be verified",
            "the asset cache filename is not valid text",
        )
    })?;
    let root = cache_root(project_dir, kind)?;
    let path = root.join(&name);
    let mut ledger = read_ledger(project_dir)?;
    matching_ledger_entry(&ledger, &entry_key, kind, asset, name_text)?;
    let exists = match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => true,
        Ok(_) => {
            return Err(cache_error(
                "cache output could not be removed",
                "the exact ledger-owned cache path is not a plain file",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(cache_error(
                "cache output could not be removed",
                "the exact ledger-owned cache file cannot be inspected",
            ))
        }
    };
    if exists {
        std::fs::remove_file(&path).map_err(|_| {
            cache_error(
                "cache output could not be removed",
                "the exact ledger-owned cache file could not be removed",
            )
        })?;
    } else if !ledger
        .entries
        .get(&entry_key)
        .is_some_and(LedgerEntry::is_pending)
    {
        return Err(cache_error(
            "cache output could not be removed",
            "a ready ownership record has no cache file",
        ));
    }
    // We already matched this record above; preserve that exactness at removal
    // instead of ever accepting an arbitrary same-key replacement.
    matching_ledger_entry(&ledger, &entry_key, kind, asset, name_text)?;
    ledger.entries.remove(&entry_key);
    match write_ledger(project_dir, &ledger) {
        Ok(()) if exists => Ok(OwnedRemoval::Retired),
        Ok(()) => Ok(OwnedRemoval::LedgerRetiredMissing),
        Err(error) => Ok(OwnedRemoval::UnlinkedLedgerPending(error)),
    }
}
