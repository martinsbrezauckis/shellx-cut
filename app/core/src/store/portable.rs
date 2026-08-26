//! Replayable baseline records for a Cut-native portable project package.
//!
//! A portable package deliberately starts a fresh operation journal instead of
//! copying a source project's historical absolute paths.  The baseline is an
//! internal core record, not a public edit verb: it materializes the reduced
//! project snapshot that a package publisher has already validated.

use super::*;
use std::path::{Component, Path};

pub const PORTABLE_SNAPSHOT_SCHEMA: &str = "shellx-cut/portable-project-snapshot/1";

impl ProjectStore {
    /// Append the one package-local baseline after `project.create`.
    ///
    /// Callers must construct `snapshot` from referenced media only.  This
    /// method enforces the invariants that make it safe to replay on another
    /// host: package-relative media paths, full content identities, and no
    /// source-machine derived-cache pointers.
    pub fn record_portable_snapshot(
        &mut self,
        snapshot: Project,
        manifest_path: &str,
        actor: Actor,
    ) -> Result<OpRecord, CutError> {
        validate_snapshot(&snapshot, manifest_path).map_err(|message| {
            CutError::new(
                codes::INVALID_ARGS,
                "portable project snapshot is invalid",
                message,
            )
        })?;
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            // The package's journal records the public package action only in
            // its destination project.  The source action itself never appends
            // an op, while replay recognizes the private baseline effect.
            verb: "project.package_create".into(),
            args: json!({"manifest": manifest_path}),
            rationale: None,
            effects: vec![edit::fx(
                None,
                json!({"schema": PORTABLE_SNAPSHOT_SCHEMA, "project": snapshot}),
            )],
            inverse: None,
            status: OpStatus::Applied,
        };
        self.commit_staged(snapshot, &rec)?;
        Ok(rec)
    }
}

pub(super) fn replay_snapshot(project: &mut Project, op: &OpRecord) -> Result<(), CutError> {
    let detail = op
        .effects
        .iter()
        .map(|effect| &effect.detail)
        .find(|detail| {
            detail.get("schema").and_then(Value::as_str) == Some(PORTABLE_SNAPSHOT_SCHEMA)
        })
        .ok_or_else(|| replay_corrupt(op, "portable snapshot effect payload is missing"))?;
    let snapshot = detail
        .get("project")
        .cloned()
        .ok_or_else(|| replay_corrupt(op, "portable snapshot project is missing"))
        .and_then(|value| {
            serde_json::from_value::<Project>(value).map_err(|error| {
                replay_corrupt(op, format!("portable snapshot is invalid: {error}"))
            })
        })?;
    validate_snapshot(&snapshot, "package.manifest.json")
        .map_err(|message| replay_corrupt(op, format!("portable snapshot is unsafe: {message}")))?;
    *project = snapshot;
    Ok(())
}

fn validate_snapshot(snapshot: &Project, manifest_path: &str) -> Result<(), String> {
    if snapshot.name.trim().is_empty() {
        return Err("project name is empty".into());
    }
    if manifest_path != "package.manifest.json" {
        return Err("portable baseline must name package.manifest.json".into());
    }
    for (asset_id, asset) in &snapshot.assets {
        if !is_exact_sha256(&asset.hash) {
            return Err(format!(
                "asset '{asset_id}' does not have a full SHA-256 identity"
            ));
        }
        if !is_portable_media_path(&asset.path) {
            return Err(format!(
                "asset '{asset_id}' path is not package-relative media"
            ));
        }
        if asset.probe.is_some()
            || asset.transcript.is_some()
            || asset.perception.is_some()
            || asset.proxy.is_some()
            || asset.filmstrip.is_some()
        {
            return Err(format!(
                "asset '{asset_id}' retains a derived cache pointer"
            ));
        }
    }
    Ok(())
}

fn is_portable_media_path(value: &str) -> bool {
    let path = Path::new(value);
    let components = path.components().collect::<Vec<_>>();
    matches!(
        components.as_slice(),
        [Component::Normal(media), Component::Normal(sha256), Component::Normal(file)]
            if *media == "media" && *sha256 == "sha256" && !file.is_empty()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn portable_baseline_replays_without_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(dir.path(), "portable", None).unwrap();
        let mut snapshot = store.project.clone();
        snapshot.assets.insert(
            "a1".into(),
            Asset {
                path: format!("media/sha256/{}.mov", "a".repeat(64)),
                hash: exact(),
                probe: None,
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        store
            .record_portable_snapshot(snapshot, "package.manifest.json", Actor::system())
            .unwrap();
        std::fs::remove_file(store.dir.join("project.json")).unwrap();
        let reopened = ProjectStore::open(&store.dir).unwrap();
        assert_eq!(
            reopened.project.assets["a1"].path,
            format!("media/sha256/{}.mov", "a".repeat(64))
        );
    }

    #[test]
    fn portable_baseline_refuses_an_absolute_path_or_cache_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(dir.path(), "portable", None).unwrap();
        let mut snapshot = store.project.clone();
        snapshot.assets.insert(
            "a1".into(),
            Asset {
                path: "/host/source.mov".into(),
                hash: exact(),
                probe: Some(json!({"duration_ms": 1})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        let error = store
            .record_portable_snapshot(snapshot, "package.manifest.json", Actor::system())
            .unwrap_err();
        assert_eq!(error.code, codes::INVALID_ARGS);
    }
}
