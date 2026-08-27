use super::model::*;
use super::status::index_kind_current;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn selected_asset(selected: Option<&BTreeSet<String>>, asset_id: &str) -> bool {
    selected.is_none_or(|ids| ids.contains(asset_id))
}

fn old_entry_is_current(
    entry: &EvidenceEntry,
    old: &MediaEvidenceIndex,
    current: &SourceBinding,
    revision: &Option<String>,
) -> bool {
    if matches!(entry.kind.as_str(), "marker" | "metadata") && old.project_revision != *revision {
        return false;
    }
    index_kind_current(old, current, &entry.kind)
        && match entry.kind.as_str() {
            "transcript" | "visual" | "scene" | "beat" => {
                authority_hash(current, &entry.kind) == Some(entry.provenance_sha256.as_str())
            }
            "marker" | "metadata" => true,
            _ => false,
        }
}

pub(super) fn merge_index(
    snapshot: &ProjectSnapshot,
    current_bindings: Vec<SourceBinding>,
    existing: Option<MediaEvidenceIndex>,
    rebuilt: MediaEvidenceIndex,
    selected: Option<&BTreeSet<String>>,
    replaced_kinds: &BTreeSet<String>,
) -> Result<MediaEvidenceIndex, CutError> {
    let existing = existing.filter(|index| index.project_id == rebuilt.project_id);
    let current = binding_map(&current_bindings);
    let mut entries = Vec::new();
    let mut coverage: BTreeMap<String, Vec<String>> = BTreeMap::new();

    if let Some(old) = &existing {
        for entry in &old.entries {
            let replaced =
                replaced_kinds.contains(&entry.kind) && selected_asset(selected, &entry.asset_id);
            let Some(binding) = current.get(entry.asset_id.as_str()) else {
                continue;
            };
            if !replaced && old_entry_is_current(entry, old, binding, &snapshot.revision) {
                entries.push(entry.clone());
            }
        }
        for (asset_id, kinds) in &old.coverage {
            let Some(binding) = current.get(asset_id.as_str()) else {
                continue;
            };
            let retained = kinds
                .iter()
                .filter(|kind| {
                    !(replaced_kinds.contains(*kind) && selected_asset(selected, asset_id))
                        && index_kind_current(old, binding, kind)
                        && (!matches!(kind.as_str(), "marker" | "metadata")
                            || old.project_revision == snapshot.revision)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !retained.is_empty() {
                coverage.insert(asset_id.clone(), retained);
            }
        }
    }

    entries.extend(rebuilt.entries);
    for (asset_id, kinds) in rebuilt.coverage {
        let target = coverage.entry(asset_id).or_default();
        target.extend(kinds);
        target.sort();
        target.dedup();
    }
    entries.sort_by(|left, right| {
        (
            &left.kind,
            &left.asset_id,
            left.source_start_ms,
            &left.evidence_id,
        )
            .cmp(&(
                &right.kind,
                &right.asset_id,
                right.source_start_ms,
                &right.evidence_id,
            ))
    });
    entries.dedup_by(|left, right| left.evidence_id == right.evidence_id);
    let selected_kinds = coverage
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    finalize_index(MediaEvidenceIndex {
        schema: INDEX_SCHEMA.into(),
        generator: generator_identity(),
        index_id: String::new(),
        project_id: rebuilt.project_id,
        project_revision: snapshot.revision.clone(),
        selected_kinds,
        bindings: current_bindings,
        coverage,
        entries,
    })
}
