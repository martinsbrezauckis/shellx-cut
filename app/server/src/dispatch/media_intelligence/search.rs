use super::model::*;
use super::navigation::{cursor_offset, occurrences, Occurrence};
use super::status::{index_kind_current, status_value};
use super::visual::visual_candidates;
use super::*;
use std::collections::BTreeSet;

#[derive(Debug, serde::Deserialize)]
pub(super) struct SearchArgs {
    pub(super) query: String,
    pub(super) asset_ids: Option<Vec<String>>,
    pub(super) kinds: Option<Vec<String>>,
    pub(super) scope: Option<String>,
    pub(super) limit: Option<usize>,
    pub(super) cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub(super) entry: EvidenceEntry,
    pub(super) match_kind: &'static str,
    pub(super) relevance: Option<f32>,
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .map(|term| {
            term.trim_matches(|character: char| !character.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn entry_is_current(
    entry: &EvidenceEntry,
    index: &MediaEvidenceIndex,
    binding: &SourceBinding,
    project_revision: &Option<String>,
) -> bool {
    if matches!(entry.kind.as_str(), "marker" | "metadata")
        && index.project_revision != *project_revision
    {
        return false;
    }
    index_kind_current(index, binding, &entry.kind)
        && match entry.kind.as_str() {
            "transcript" => binding.transcript_sha256.as_deref(),
            "scene" | "beat" => binding.perception_sha256.as_deref(),
            "visual" => binding.visual_sha256.as_deref(),
            "marker" | "metadata" => Some(entry.provenance_sha256.as_str()),
            _ => None,
        }
        .is_some_and(|hash| {
            matches!(entry.kind.as_str(), "marker" | "metadata") || hash == entry.provenance_sha256
        })
}

pub(super) fn evidence_hit_value(
    current: &[SourceBinding],
    entry: EvidenceEntry,
    match_kind: &str,
    relevance: Option<f32>,
    all_occurrences: Vec<Occurrence>,
) -> Value {
    let occurrence_count = all_occurrences.len();
    let occurrences = all_occurrences.into_iter().take(8).collect::<Vec<_>>();
    let available = binding_map(current)
        .get(entry.asset_id.as_str())
        .is_some_and(|binding| binding.available);
    json!({
        "schema": "shellx-cut/evidence-hit/1",
        "evidence_id": entry.evidence_id,
        "asset_id": entry.asset_id,
        "source_start_ms": entry.source_start_ms,
        "source_end_ms": entry.source_end_ms,
        "anchor_ms": entry.anchor_ms,
        "kind": entry.kind,
        "excerpt": entry.excerpt,
        "speaker": entry.speaker,
        "match": match_kind,
        "relevance": relevance,
        "provenance": {"kind": entry.kind, "sha256": entry.provenance_sha256},
        "available": available,
        "occurrence_count": occurrence_count,
        "occurrences": occurrences,
    })
}

fn structured_candidates(
    index: &MediaEvidenceIndex,
    current: &[SourceBinding],
    kinds: &BTreeSet<String>,
    selected: Option<&BTreeSet<String>>,
    query: &str,
    project_revision: &Option<String>,
) -> (Vec<Candidate>, usize) {
    let bindings = binding_map(current);
    let query = normalize(query);
    let terms = query.split_whitespace().collect::<Vec<_>>();
    let mut stale = 0;
    let mut candidates = Vec::new();
    for entry in &index.entries {
        if !kinds.contains(&entry.kind)
            || selected.is_some_and(|ids| !ids.contains(&entry.asset_id))
        {
            continue;
        }
        let Some(binding) = bindings.get(entry.asset_id.as_str()) else {
            stale += 1;
            continue;
        };
        if !entry_is_current(entry, index, binding, project_revision) {
            stale += 1;
            continue;
        }
        let haystack = normalize(&entry.excerpt);
        if terms.iter().all(|term| haystack.contains(term)) {
            candidates.push(Candidate {
                entry: entry.clone(),
                match_kind: if haystack == query || haystack.contains(&query) {
                    "exact"
                } else {
                    "contains"
                },
                relevance: None,
            });
        }
    }
    (candidates, stale)
}

pub(super) async fn search_value(
    snapshot: ProjectSnapshot,
    args: SearchArgs,
) -> Result<Value, CutError> {
    let query = args.query.trim().to_string();
    if query.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "query is empty",
            "describe a moment or type spoken words",
        ));
    }
    let index = load_index(&snapshot.dir).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "media intelligence index is not built",
            "run media.intelligence_rebuild, then search again",
        )
    })?;
    let kinds: BTreeSet<String> = args
        .kinds
        .unwrap_or_else(|| ALL_KINDS.iter().map(|kind| kind.to_string()).collect())
        .into_iter()
        .collect();
    if kinds.iter().any(|kind| !ALL_KINDS.contains(&kind.as_str())) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "unknown evidence kind",
            "use the kinds in schema/verbs.json",
        ));
    }
    let selected = args
        .asset_ids
        .map(|ids| ids.into_iter().collect::<BTreeSet<_>>());
    let selected_vec = selected
        .as_ref()
        .map(|ids| ids.iter().cloned().collect::<Vec<_>>());
    let current = current_bindings(&snapshot, selected_vec.as_deref())?;
    let limit = args.limit.unwrap_or(40).clamp(1, 100);
    let (mut candidates, stale_excluded) = structured_candidates(
        &index,
        &current,
        &kinds,
        selected.as_ref(),
        &query,
        &snapshot.revision,
    );
    let mut warnings = Vec::new();
    if kinds.contains("visual") {
        let visual = run_blocking("media.intelligence_search visual query", {
            let snapshot = snapshot.clone();
            let index = index.clone();
            let current = current.clone();
            let selected = selected.clone();
            let query = query.clone();
            move || {
                visual_candidates(
                    &snapshot,
                    &index,
                    &current,
                    selected.as_ref(),
                    &query,
                    limit,
                )
            }
        })
        .await;
        match visual {
            Ok(hits) => candidates.extend(hits),
            Err(error) if error.code == error_codes::UNIMPLEMENTED => warnings.push(error.message),
            Err(error) => warnings.push(format!("Visual evidence unavailable: {}", error.message)),
        }
    }
    candidates.sort_by(|left, right| {
        let rank = |candidate: &Candidate| match candidate.match_kind {
            "exact" => 0,
            "semantic" => 1,
            _ => 2,
        };
        rank(left)
            .cmp(&rank(right))
            .then_with(|| {
                right
                    .relevance
                    .partial_cmp(&left.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                (
                    &left.entry.asset_id,
                    left.entry.source_start_ms,
                    &left.entry.evidence_id,
                )
                    .cmp(&(
                        &right.entry.asset_id,
                        right.entry.source_start_ms,
                        &right.entry.evidence_id,
                    ))
            })
    });
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| seen.insert(candidate.entry.evidence_id.clone()));
    let scope = args.scope.as_deref().unwrap_or("all_project_media");
    if !matches!(scope, "all_project_media" | "this_sequence") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "unknown evidence scope",
            "use all_project_media or this_sequence",
        ));
    }
    let selected_key = selected
        .as_ref()
        .map(|ids| ids.iter().cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    let kind_key = kinds.iter().cloned().collect::<Vec<_>>().join(",");
    let seed = opaque_id(&[&index.index_id, &query, &kind_key, &selected_key, scope]);
    let offset = cursor_offset(args.cursor.as_deref(), &seed)?;
    let mut scoped = Vec::new();
    for candidate in candidates {
        let scoped_occurrences = if scope == "this_sequence" {
            let values = occurrences(&snapshot.project, &candidate.entry);
            if !values
                .iter()
                .any(|occurrence| occurrence.sequence_id == snapshot.project.active_sequence)
            {
                continue;
            }
            Some(values)
        } else {
            None
        };
        scoped.push((candidate, scoped_occurrences));
    }
    if offset > scoped.len() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "evidence cursor is beyond the current result set",
            "restart this search from the first page",
        ));
    }
    let total = scoped.len();
    let page = scoped
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(candidate, scoped_occurrences)| {
            let all_occurrences = scoped_occurrences
                .unwrap_or_else(|| occurrences(&snapshot.project, &candidate.entry));
            evidence_hit_value(
                &current,
                candidate.entry,
                candidate.match_kind,
                candidate.relevance,
                all_occurrences,
            )
        })
        .collect::<Vec<_>>();
    let next_offset = offset.saturating_add(page.len());
    let next_cursor = (next_offset < total).then(|| format!("{seed}:{next_offset}"));
    let status = status_value(&snapshot, selected_vec.as_deref())?;
    Ok(json!({
        "schema": SEARCH_SCHEMA,
        "query": query,
        "index_id": index.index_id,
        "complete": status["complete"],
        "stale_excluded": stale_excluded,
        "warnings": warnings,
        "count": page.len(),
        "hits": page,
        "next_cursor": next_cursor,
    }))
}
