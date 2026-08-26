use super::model::*;
use super::*;
use std::collections::BTreeMap;

fn eligible(binding: &SourceBinding, kind: &str) -> bool {
    match kind {
        "transcript" => binding.audio,
        "visual" | "scene" => binding.video,
        "beat" => binding.audio,
        "metadata" => true,
        _ => false,
    }
}

fn binding_is_current(current: &SourceBinding, indexed: &SourceBinding, kind: &str) -> bool {
    current.asset_hash == indexed.asset_hash
        && authority_hash(current, kind).is_some()
        && authority_hash(current, kind) == authority_hash(indexed, kind)
}

pub(super) fn index_kind_current(
    index: &MediaEvidenceIndex,
    current: &SourceBinding,
    kind: &str,
) -> bool {
    let binding_current = if kind == "marker" {
        index
            .bindings
            .iter()
            .find(|binding| binding.asset_id == current.asset_id)
            .is_some_and(|indexed| indexed.asset_hash == current.asset_hash)
    } else {
        index
            .bindings
            .iter()
            .find(|binding| binding.asset_id == current.asset_id)
            .is_some_and(|indexed| binding_is_current(current, indexed, kind))
    };
    index
        .coverage
        .get(&current.asset_id)
        .is_some_and(|kinds| kinds.iter().any(|selected| selected == kind))
        && binding_current
}

pub(super) fn status_value(
    snapshot: &ProjectSnapshot,
    selected: Option<&[String]>,
) -> Result<Value, CutError> {
    let current = current_bindings(snapshot, selected)?;
    let index = load_index(&snapshot.dir);
    let indexed_by_asset: BTreeMap<&str, &SourceBinding> = index
        .as_ref()
        .map(|index| binding_map(&index.bindings))
        .unwrap_or_default();
    let project_current = index
        .as_ref()
        .is_some_and(|index| index.project_revision == snapshot.revision);
    let mut coverage: BTreeMap<&str, Value> = BTreeMap::new();
    let mut assets = Vec::new();
    let mut any_stale = index.is_some() && !project_current;
    let mut complete = index.is_some();
    let marker_total = {
        let mut project = snapshot.project.clone();
        project.ensure_sequence_bank();
        project.sync_active_sequence();
        project
            .sequences
            .iter()
            .map(|sequence| sequence.markers.len())
            .sum::<usize>()
    };

    for kind in ALL_KINDS {
        if kind == "marker" {
            let total = marker_total;
            let ready = index
                .as_ref()
                .filter(|_| project_current)
                .map(|index| {
                    index
                        .entries
                        .iter()
                        .filter(|entry| entry.kind == "marker")
                        .count()
                })
                .unwrap_or(0);
            coverage.insert(
                kind,
                json!({"ready": ready, "total": total, "missing": total.saturating_sub(ready)}),
            );
            complete &= ready == total;
            continue;
        }
        let mut ready = 0usize;
        let mut stale = 0usize;
        let mut missing = 0usize;
        let mut total = 0usize;
        for binding in &current {
            if !eligible(binding, kind) {
                continue;
            }
            total += 1;
            let has_source = authority_hash(binding, kind).is_some();
            let current_in_index = index
                .as_ref()
                .is_some_and(|index| index_kind_current(index, binding, kind));
            if current_in_index {
                ready += 1;
            } else if has_source && indexed_by_asset.contains_key(binding.asset_id.as_str()) {
                stale += 1;
                any_stale = true;
            } else {
                missing += 1;
            }
        }
        coverage.insert(
            kind,
            json!({"ready": ready, "total": total, "stale": stale, "missing": missing}),
        );
        complete &= ready == total;
    }

    for binding in current {
        let mut kinds = serde_json::Map::new();
        for kind in ALL_KINDS.into_iter().filter(|kind| *kind != "marker") {
            if !eligible(&binding, kind) {
                continue;
            }
            let source = authority_hash(&binding, kind).is_some();
            let indexed = index
                .as_ref()
                .is_some_and(|index| index_kind_current(index, &binding, kind));
            let state = if indexed {
                "ready"
            } else if source && indexed_by_asset.contains_key(binding.asset_id.as_str()) {
                "stale"
            } else {
                "missing"
            };
            kinds.insert(
                kind.into(),
                json!({"source": source, "indexed": indexed, "state": state}),
            );
        }
        assets.push(json!({
            "asset_id": binding.asset_id,
            "available": binding.available,
            "kinds": kinds,
        }));
    }
    Ok(json!({
        "schema": "shellx-cut/media-intelligence-status/1",
        "index_id": index.as_ref().map(|index| index.index_id.as_str()),
        "project_revision": snapshot.revision,
        "complete": complete,
        "stale": any_stale,
        "entry_count": index.as_ref().map(|index| index.entries.len()).unwrap_or(0),
        "coverage": coverage,
        "assets": assets,
    }))
}
