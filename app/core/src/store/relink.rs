//! Durable grouped, exact-hash offline-media relink operations.
//!
//! This deliberately changes only `Asset::path`.  It neither touches the
//! Library nor invalidates/regenerates derived media.  A single op carries the
//! complete group so reopening/replay is deterministic and there is no Ctrl-Z
//! promise for this metadata-recovery action.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const RELINK_GROUP_SCHEMA: &str = "shellx-cut/media-relink-group/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelinkGroupChange {
    pub asset_id: String,
    pub expected_hash: String,
    pub old_path: String,
    pub chosen_path: String,
}

#[derive(Debug, Clone)]
pub struct RelinkGroupCommit {
    pub changes: Vec<RelinkGroupChange>,
    pub op: OpRecord,
}

/// The bulk-relink contract accepts only a full, lowercase SHA-256 identity.
/// Sampled hashes (`sha256s:`) cannot prove a moved file is the same content.
pub fn is_exact_sha256(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl ProjectStore {
    /// Commit one all-or-nothing project-local relink metadata operation.
    ///
    /// The caller has already hashed and selected candidates.  Rechecking the
    /// preimage (asset id, path, exact stored hash) here is the final guard
    /// before the one durable append, so stale previews cannot partly apply.
    pub fn record_relink_group(
        &mut self,
        changes: Vec<RelinkGroupChange>,
        plan_hash: &str,
        project_identity: Value,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<RelinkGroupCommit, CutError> {
        if changes.is_empty() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "bulk relink requires at least one accepted asset",
                "preview exact-hash candidates and select one or more eligible assets",
            ));
        }
        if !is_exact_sha256(plan_hash) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "bulk relink plan hash is not a full SHA-256 value",
                "refresh the preview and use its exact plan_hash",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut next = self.project.clone();
        for change in &changes {
            if !seen.insert(change.asset_id.as_str()) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("bulk relink repeats asset '{}'", change.asset_id),
                    "each asset may be accepted at most once",
                ));
            }
            if !is_exact_sha256(&change.expected_hash) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("asset '{}' has no full SHA-256 identity", change.asset_id),
                    "sampled and missing hashes are diagnostic-only; re-import to establish an exact hash",
                ));
            }
            let asset = next.assets.get_mut(&change.asset_id).ok_or_else(|| {
                CutError::new(
                    codes::NOT_FOUND,
                    format!("no asset '{}'", change.asset_id),
                    "refresh the preview before applying a relink",
                )
            })?;
            if asset.hash != change.expected_hash || asset.path != change.old_path {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!(
                        "asset '{}' changed since bulk-relink preview",
                        change.asset_id
                    ),
                    "refresh the preview; no selected assets were changed",
                ));
            }
            asset.path = change.chosen_path.clone();
        }
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            verb: "media.relink_apply".into(),
            args: json!({
                "plan_hash": plan_hash,
                "accepted_assets": changes.iter().map(|change| change.asset_id.as_str()).collect::<Vec<_>>(),
            }),
            rationale,
            effects: vec![edit::fx(
                None,
                json!({
                    "schema": RELINK_GROUP_SCHEMA,
                    "project_identity": project_identity,
                    "plan_hash": plan_hash,
                    "relinks": changes.iter().map(|change| json!({
                        "asset_id": change.asset_id,
                        "expected_hash": change.expected_hash,
                        "old_path": change.old_path,
                        "chosen_path": change.chosen_path,
                        "disposition": "relinked",
                    })).collect::<Vec<_>>(),
                }),
            )],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(next, &rec)?;
        Ok(RelinkGroupCommit { changes, op: rec })
    }
}

pub(super) fn replay_group(project: &mut Project, op: &OpRecord) -> Result<(), CutError> {
    let detail = op
        .effects
        .iter()
        .map(|effect| &effect.detail)
        .find(|detail| detail.get("schema").and_then(Value::as_str) == Some(RELINK_GROUP_SCHEMA))
        .ok_or_else(|| replay_corrupt(op, "bulk relink effect payload is missing"))?;
    let changes: Vec<RelinkGroupChange> = serde_json::from_value(
        detail
            .get("relinks")
            .cloned()
            .ok_or_else(|| replay_corrupt(op, "bulk relink effect is missing relinks"))?,
    )
    .map_err(|error| replay_corrupt(op, format!("bulk relink effect is invalid: {error}")))?;
    if changes.is_empty() {
        return Err(replay_corrupt(op, "bulk relink effect has no relinks"));
    }
    let mut seen = BTreeSet::new();
    for change in &changes {
        if !seen.insert(change.asset_id.as_str()) || !is_exact_sha256(&change.expected_hash) {
            return Err(replay_corrupt(
                op,
                "bulk relink effect has duplicate or non-exact assets",
            ));
        }
        let asset = project.assets.get_mut(&change.asset_id).ok_or_else(|| {
            replay_corrupt(
                op,
                format!("bulk relink asset '{}' is missing", change.asset_id),
            )
        })?;
        // An old project cache can be reconstructed only if the immutable
        // journal's preimage is coherent.  Do not silently repoint wrong media.
        if asset.hash != change.expected_hash || asset.path != change.old_path {
            return Err(replay_corrupt(
                op,
                format!(
                    "bulk relink preimage for '{}' does not match replay state",
                    change.asset_id
                ),
            ));
        }
        asset.path = change.chosen_path.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn grouped_relink_is_one_durable_op_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let (asset_id, _) = store
            .record_import(
                Some("a1".into()),
                Asset {
                    path: "/gone/one.mov".into(),
                    hash: exact(),
                    probe: None,
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                Actor::system(),
                None,
            )
            .unwrap();
        assert_eq!(asset_id, "a1");
        let prior = store.log.read_all().unwrap().len();
        let committed = store
            .record_relink_group(
                vec![RelinkGroupChange {
                    asset_id: "a1".into(),
                    expected_hash: exact(),
                    old_path: "/gone/one.mov".into(),
                    chosen_path: "/restored/one.mov".into(),
                }],
                &format!("sha256:{}", "b".repeat(64)),
                json!({"schema": "shellx-cut/project-identity/1", "id": "test"}),
                Actor::system(),
                None,
            )
            .unwrap();
        assert_eq!(store.log.read_all().unwrap().len(), prior + 1);
        assert!(committed.op.inverse.is_none());
        assert_eq!(store.project.assets["a1"].path, "/restored/one.mov");
        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert_eq!(reopened.project.assets["a1"].path, "/restored/one.mov");
    }
}
