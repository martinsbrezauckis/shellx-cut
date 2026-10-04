//! Path-free accounting for a reservation pass that stopped after an error.
//! The lifecycle writer lease remains held while the exact ledger is read back.

use super::{OutputAction, RebuildAssetPlan};
use crate::cache_lifecycle::ownership::{base_output_name, key, read_ledger};
use crate::cache_lifecycle::CacheKind;
use cut_core::CutError;
use serde_json::{json, Value};
use std::path::Path;

pub(super) struct ReservationTarget {
    asset_id: String,
    kind: CacheKind,
    source_hash: String,
    reserved_before_error: bool,
}

pub(super) struct ReservationFailure {
    pub(super) error: CutError,
    pub(super) targets: Vec<ReservationTarget>,
}

pub(super) fn reserve_planned_outputs<F>(
    project_dir: &Path,
    plans: &[RebuildAssetPlan],
    mut reserve: F,
) -> Result<(), ReservationFailure>
where
    F: FnMut(&Path, CacheKind, &str, &str) -> Result<(), CutError>,
{
    let mut targets = Vec::new();
    for plan in plans {
        for (kind, action) in &plan.outputs {
            if *action != OutputAction::Generate {
                continue;
            }
            let target = ReservationTarget {
                asset_id: plan.asset.asset_id.clone(),
                kind: *kind,
                source_hash: plan.asset.hash.clone(),
                reserved_before_error: false,
            };
            if let Err(error) = reserve(project_dir, *kind, &target.asset_id, &target.source_hash) {
                targets.push(target);
                return Err(ReservationFailure { error, targets });
            }
            targets.push(ReservationTarget {
                reserved_before_error: true,
                ..target
            });
        }
    }
    Ok(())
}

pub(super) fn partial_reservation_result(
    project_dir: &Path,
    targets: &[ReservationTarget],
) -> Value {
    let retry = "restore or relink a changed source, or remove the asset; then retry cache rebuild";
    let attempted_outputs = targets
        .iter()
        .map(|target| {
            json!({
                "asset_id": target.asset_id,
                "kind": target.kind.directory(),
                "reserved_before_error": target.reserved_before_error,
            })
        })
        .collect::<Vec<_>>();
    let unknown = || {
        json!({
            "schema": "shellx-cut/cache-rebuild-partial/1",
            "status": "failed_partial_admission",
            "ownership_verification": "unavailable",
            "attempted_outputs": attempted_outputs,
            "retry": retry,
        })
    };
    let Ok(ledger) = read_ledger(project_dir) else {
        return unknown();
    };
    let mut pending = Vec::new();
    let mut pending_retired = 0u64;
    let mut pending_unretired = 0u64;
    for target in targets {
        let Ok(name) = base_output_name(target.kind, &target.asset_id) else {
            return unknown();
        };
        let Some(entry_key) = key(target.kind, &name) else {
            return unknown();
        };
        let Some(entry) = ledger.entries.get(&entry_key) else {
            if target.reserved_before_error {
                return unknown();
            }
            continue;
        };
        if entry.kind != target.kind.directory()
            || entry.asset != target.asset_id
            || entry.name != name.to_string_lossy().as_ref()
        {
            return unknown();
        }
        if entry.state != "pending" {
            if target.reserved_before_error {
                return unknown();
            }
            continue;
        }
        if entry.source_hash.as_deref() != Some(target.source_hash.as_str()) {
            return unknown();
        }
        if entry.output_retired {
            pending_retired += 1;
        } else {
            pending_unretired += 1;
        }
        pending.push(json!({
            "asset_id": target.asset_id,
            "kind": target.kind.directory(),
            "output_retired": entry.output_retired,
            "reserved_before_error": target.reserved_before_error,
        }));
    }
    json!({
        "schema": "shellx-cut/cache-rebuild-partial/1",
        "status": "failed_partial_admission",
        "ownership_verification": "verified",
        "pending_outputs": pending,
        "counts": {
            "pending_retired": pending_retired,
            "pending_unretired": pending_unretired,
        },
        "retry": retry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_ownership_never_reports_a_verified_partial_count() {
        let root = tempfile::tempdir().unwrap();
        let project =
            cut_core::ProjectStore::create(&root.path().join("cache.cutproj"), "cache", None)
                .unwrap();
        let project_dir = &project.dir;
        crate::cache_lifecycle::ownership::reserve_rebuild_output(
            project_dir,
            CacheKind::Proxies,
            "a1",
            "sha256:source",
        )
        .unwrap();
        let ledger_path = project_dir.join(".shellx-cut-cache-ownership.json");
        let saved_path = project_dir.join("ownership-ledger-saved-for-test.json");
        std::fs::rename(&ledger_path, &saved_path).unwrap();
        std::fs::create_dir(&ledger_path).unwrap();
        let result = partial_reservation_result(
            project_dir,
            &[ReservationTarget {
                asset_id: "a1".into(),
                kind: CacheKind::Proxies,
                source_hash: "sha256:source".into(),
                reserved_before_error: true,
            }],
        );
        assert_eq!(result["ownership_verification"], "unavailable");
        assert!(result.get("counts").is_none());
        assert!(result.get("pending_outputs").is_none());
        assert_eq!(result["attempted_outputs"][0]["asset_id"], "a1");
        assert_eq!(
            result["attempted_outputs"][0]["reserved_before_error"],
            true
        );
        std::fs::remove_dir(&ledger_path).unwrap();
        std::fs::rename(&saved_path, &ledger_path).unwrap();
        let ledger = read_ledger(project_dir).unwrap();
        let entry = ledger.entries.get("proxies/a1.mp4").unwrap();
        assert_eq!(entry.state, "pending");
        assert!(entry.output_retired);
    }
}
