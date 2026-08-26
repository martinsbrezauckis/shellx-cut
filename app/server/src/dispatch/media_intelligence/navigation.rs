use super::model::EvidenceEntry;
use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Occurrence {
    pub(super) sequence_id: String,
    clip_id: String,
    track_id: String,
    timeline_start_ms: u64,
    timeline_end_ms: u64,
}

pub(super) fn occurrences(project: &cut_core::Project, entry: &EvidenceEntry) -> Vec<Occurrence> {
    let mut project = project.clone();
    project.ensure_sequence_bank();
    project.sync_active_sequence();
    let mut mapped: BTreeMap<(String, String, String), (u64, u64)> = BTreeMap::new();
    for sequence in project.sequences.clone() {
        let mut view = project.clone();
        if !view.switch_sequence(&sequence.id) {
            continue;
        }
        for segment in cut_core::edl_from_project(&view).segments {
            if segment.asset.as_deref() != Some(entry.asset_id.as_str()) {
                continue;
            }
            let (Some(src_in), Some(src_out), Some(clip_id)) =
                (segment.src_in_ms, segment.src_out_ms, segment.clip_id)
            else {
                continue;
            };
            let lo = entry.source_start_ms.max(src_in);
            let hi = entry.source_end_ms.min(src_out);
            if lo >= hi {
                continue;
            }
            let (start, end) = if segment.reverse {
                (
                    segment.timeline_in_ms + cut_core::src_off_to_tl(src_out - hi, segment.speed),
                    segment.timeline_in_ms + cut_core::src_off_to_tl(src_out - lo, segment.speed),
                )
            } else {
                (
                    segment.timeline_in_ms + cut_core::src_off_to_tl(lo - src_in, segment.speed),
                    segment.timeline_in_ms + cut_core::src_off_to_tl(hi - src_in, segment.speed),
                )
            };
            let key = (sequence.id.clone(), clip_id, segment.track);
            let range = mapped.entry(key).or_insert((start, end));
            range.0 = range.0.min(start);
            range.1 = range.1.max(end);
        }
    }
    mapped
        .into_iter()
        .map(
            |((sequence_id, clip_id, track_id), (start, end))| Occurrence {
                sequence_id,
                clip_id,
                track_id,
                timeline_start_ms: start,
                timeline_end_ms: end.max(start.saturating_add(1)),
            },
        )
        .collect()
}

pub(super) fn cursor_offset(cursor: Option<&str>, seed: &str) -> Result<usize, CutError> {
    let Some(cursor) = cursor else { return Ok(0) };
    let (actual_seed, offset) = cursor.rsplit_once(':').ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "invalid evidence cursor",
            "restart this search",
        )
    })?;
    if actual_seed != seed {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "evidence cursor belongs to a different query or index snapshot",
            "restart this search from the first page",
        ));
    }
    offset.parse().map_err(|_| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "invalid evidence cursor offset",
            "restart this search",
        )
    })
}
