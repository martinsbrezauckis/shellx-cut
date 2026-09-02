//! Exact recovery of deterministic rebuild reservations.
//!
//! Callers hold the cache lifecycle writer lease. Pending entries never grant
//! broad cleanup authority: each recovery target is re-derived from the ledger
//! entry's own asset/kind/base-name identity before it can be retired.

use super::*;
use std::collections::BTreeSet;
use std::ffi::OsString;

pub(crate) fn pending_rebuild_outputs_for_asset(
    project_dir: &Path,
    asset: &str,
) -> Result<Vec<(CacheKind, String)>, CutError> {
    let ledger = read_ledger(project_dir)?;
    let mut outputs = Vec::new();
    for (entry_key, entry) in &ledger.entries {
        if entry.asset != asset || !entry.is_pending() {
            continue;
        }
        let (kind, relative) = checked_pending_output(asset, entry_key, entry)?;
        if outputs.iter().any(|(existing, _)| *existing == kind) {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "an asset has more than one pending base output of the same kind",
            ));
        }
        outputs.push((kind, relative));
    }
    Ok(outputs)
}

/// A removed asset cannot later resume its reservation. Retire only valid,
/// exact pending outputs; a ledger-write failure remains a visible pending entry
/// and this same reconciliation can safely be retried by cache rebuild.
pub(in crate::cache_lifecycle) fn retire_orphaned_pending_outputs(
    project_dir: &Path,
    live_assets: &BTreeSet<String>,
) -> Result<(), CutError> {
    let ledger = read_ledger(project_dir)?;
    let orphaned = ledger
        .entries
        .values()
        .filter(|entry| entry.is_pending() && !live_assets.contains(&entry.asset))
        .map(|entry| entry.asset.clone())
        .collect::<BTreeSet<_>>();
    for asset in orphaned {
        for (kind, relative) in pending_rebuild_outputs_for_asset(project_dir, &asset)? {
            match remove_owned_output(project_dir, kind, &asset, &relative)? {
                OwnedRemoval::Retired | OwnedRemoval::LedgerRetiredMissing => {}
                OwnedRemoval::UnlinkedLedgerPending(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn checked_pending_output(
    asset: &str,
    entry_key: &str,
    entry: &LedgerEntry,
) -> Result<(CacheKind, String), CutError> {
    let kind = match entry.kind.as_str() {
        "proxies" => CacheKind::Proxies,
        "filmstrip" => CacheKind::Thumbnails,
        _ => {
            return Err(cache_error(
                "cache ownership cannot be verified",
                "a pending cache reservation has an unknown root",
            ))
        }
    };
    let name = OsString::from(&entry.name);
    let expected = base_output_name(kind, asset)?;
    if name != expected
        || !entry.matches(kind, asset, &entry.name)
        || entry.source_hash.is_none()
        || key(kind, &name).as_deref() != Some(entry_key)
    {
        return Err(cache_error(
            "cache ownership cannot be verified",
            "a pending cache reservation is not an exact rebuild base output",
        ));
    }
    Ok((kind, format!("{}/{}", kind.directory(), entry.name)))
}
