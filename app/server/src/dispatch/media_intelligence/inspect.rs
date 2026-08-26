use super::model::*;
use super::navigation::occurrences;
use super::search::{entry_is_current, evidence_hit_value};
use super::status::status_value;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const MAX_MEDIA_INSPECTIONS: usize = 100;
pub(super) const MAX_EVIDENCE_ATTACHMENTS: usize = 12;

#[derive(Debug, serde::Deserialize)]
pub(super) struct InspectMediaArgs {
    pub(super) asset_ids: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct InspectRangeArgs {
    pub(super) evidence_ids: Vec<String>,
    pub(super) index_id: Option<String>,
}

fn validate_evidence_ids(ids: &[String]) -> Result<(), CutError> {
    if ids.is_empty() || ids.len() > MAX_EVIDENCE_ATTACHMENTS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "invalid evidence attachment count",
            format!("attach between 1 and {MAX_EVIDENCE_ATTACHMENTS} cited moments"),
        ));
    }
    let unique = ids.iter().collect::<BTreeSet<_>>();
    if unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "invalid evidence attachments",
            "use unique opaque evidence ids returned by media.intelligence_search",
        ));
    }
    Ok(())
}

pub(super) fn inspect_media_value(
    snapshot: &ProjectSnapshot,
    requested: Option<Vec<String>>,
) -> Result<Value, CutError> {
    let requested = requested.map(|ids| ids.into_iter().collect::<BTreeSet<_>>());
    if requested
        .as_ref()
        .is_some_and(|ids| ids.len() > MAX_MEDIA_INSPECTIONS)
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "too many media inspection targets",
            format!("inspect at most {MAX_MEDIA_INSPECTIONS} registered assets at once"),
        ));
    }
    if let Some(ids) = &requested {
        for id in ids {
            if !snapshot.project.assets.contains_key(id) {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("unknown asset '{id}'"),
                    "use registered asset ids from project.state",
                ));
            }
        }
    }
    let selected = snapshot
        .project
        .assets
        .keys()
        .filter(|id| requested.as_ref().is_none_or(|ids| ids.contains(*id)))
        .take(MAX_MEDIA_INSPECTIONS)
        .cloned()
        .collect::<Vec<_>>();
    let status = status_value(snapshot, Some(&selected))?;
    let statuses = status["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value["asset_id"].as_str().map(|id| (id, value)))
        .collect::<BTreeMap<_, _>>();
    let assets = selected
        .iter()
        .filter_map(|id| snapshot.project.assets.get(id).map(|asset| (id, asset)))
        .map(|(id, asset)| {
            let probe = asset.probe.as_ref();
            let kind = probe
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let has_video = kind == "video"
                || probe
                    .and_then(|value| value.get("has_video"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            let has_audio = kind == "audio"
                || probe
                    .and_then(|value| value.get("has_audio"))
                    .and_then(Value::as_bool)
                    .unwrap_or(has_video);
            json!({
                "asset_id": id,
                "content_hash": asset.hash,
                "available": statuses.get(id.as_str()).and_then(|value| value.get("available")).and_then(Value::as_bool).unwrap_or(false),
                "media_kind": kind,
                "duration_ms": probe.and_then(|value| value.get("duration_ms")).and_then(Value::as_u64),
                "has_video": has_video,
                "has_audio": has_audio,
                "evidence": statuses.get(id.as_str()).and_then(|value| value.get("kinds")).cloned().unwrap_or_else(|| json!({})),
            })
        })
        .collect::<Vec<_>>();
    let total = requested
        .as_ref()
        .map(BTreeSet::len)
        .unwrap_or(snapshot.project.assets.len());
    Ok(json!({
        "schema": "shellx-cut/media-inspection/1",
        "project_revision": snapshot.revision,
        "index_id": status["index_id"],
        "total": total,
        "count": assets.len(),
        "truncated": assets.len() < total,
        "assets": assets,
    }))
}

pub(super) fn inspect_range_value(
    snapshot: &ProjectSnapshot,
    expected_index_id: Option<&str>,
    evidence_ids: &[String],
) -> Result<Value, CutError> {
    validate_evidence_ids(evidence_ids)?;
    let index = load_index(&snapshot.dir).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "media intelligence index is not built",
            "run media.intelligence_rebuild, then select current cited moments",
        )
    })?;
    if expected_index_id.is_some_and(|expected| expected != index.index_id) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the attached evidence snapshot changed",
            "return to Find > Moment and select current cited moments",
        ));
    }
    let current = current_bindings(snapshot, None)?;
    let bindings = binding_map(&current);
    let entries = index
        .entries
        .iter()
        .map(|entry| (entry.evidence_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut hits = Vec::with_capacity(evidence_ids.len());
    for id in evidence_ids {
        let entry = entries.get(id.as_str()).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("evidence '{id}' is not in the current index"),
                "return to Find > Moment and select a current cited moment",
            )
        })?;
        let binding = bindings.get(entry.asset_id.as_str()).ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                format!("evidence '{id}' no longer belongs to a registered asset"),
                "refresh the evidence index before inspecting this range",
            )
        })?;
        if !entry_is_current(entry, &index, binding, &snapshot.revision) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!("evidence '{id}' is stale"),
                "rebuild changed evidence, then select it again",
            ));
        }
        hits.push(evidence_hit_value(
            &current,
            (*entry).clone(),
            "inspect",
            None,
            occurrences(&snapshot.project, entry),
        ));
    }
    Ok(json!({
        "schema": "shellx-cut/evidence-inspection/1",
        "index_id": index.index_id,
        "count": hits.len(),
        "hits": hits,
    }))
}
