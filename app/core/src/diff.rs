//! diff.rs — checkpoint-to-checkpoint diff (timeline/op-log contract `diff(a,b)`).
//!
//! Role: "ops between two checkpoints + computed summary (clips added/removed/
//! moved, duration delta, per-track ranges touched)" — the review-rail diff
//! view and `project.diff` verb are rendered from this.
//! Dependencies: types.rs, ops.rs. Primary callers: server (project.diff),
//! UI review rail (via the verb).

use crate::error::CutError;
use crate::ops::OpRecord;
use crate::types::Project;
use serde::{Deserialize, Serialize};

/// A reference into the log: either a checkpoint id/name or a raw op id.
/// String-typed at the API boundary; resolution happens in `diff`.
pub type LogRef = String;

/// Per-track summary of touched timeline ranges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackTouch {
    pub track: String,
    /// Merged [in, out) ranges (ms) affected between the two points.
    pub ranges_ms: Vec<[u64; 2]>,
}

/// Computed diff summary between two log positions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffSummary {
    /// Resolved op-id range (exclusive `from`, inclusive `to`).
    pub from_op: String,
    pub to_op: String,
    /// The raw ops in between, in log order (review rail shows these).
    pub ops: Vec<OpRecord>,
    pub clips_added: Vec<String>,
    pub clips_removed: Vec<String>,
    pub clips_moved: Vec<String>,
    /// to.duration_ms - from.duration_ms (negative = tightened).
    pub duration_delta_ms: i64,
    pub tracks_touched: Vec<TrackTouch>,
}

fn is_source_time_range_key(key: &str) -> bool {
    key == "src_range_ms"
        || key == "old_src_ms"
        || key == "new_src_ms"
        || key == "source_span_ms"
        || key.starts_with("src_")
        || key.starts_with("source_")
}

fn detail_timeline_ranges(detail: &serde_json::Map<String, serde_json::Value>) -> Vec<[u64; 2]> {
    let mut ranges = Vec::new();
    for (key, value) in detail {
        if is_source_time_range_key(key) {
            continue;
        }
        if key == "range_ms"
            || key == "old_range_ms"
            || key == "removed_ms"
            || key == "added_ms"
            || key == "gap_filled_ms"
            || key == "rippled_gap_ms"
        {
            if let Ok(r) = serde_json::from_value::<[u64; 2]>(value.clone()) {
                if r[0] < r[1] {
                    ranges.push(r);
                }
            }
        }
    }
    ranges
}

fn explicit_timing_change_clip_ids(ops: &[OpRecord]) -> std::collections::BTreeSet<String> {
    ops.iter()
        .filter_map(|op| match op.verb.as_str() {
            "edit.trim" | "edit.speed" | "edit.speed_ramp" => {
                op.args.get("clip").and_then(|v| v.as_str())
            }
            _ => None,
        })
        .map(str::to_string)
        .collect()
}

/// Compute the diff between two log refs (checkpoint id/name or op id).
/// `from`/`to` resolve against `project.checkpoints` first, then as op ids.
/// Pure function over the log — does not touch disk; caller supplies ops.
///
/// Method: replay the log prefix at `from` and at `to` (store::rebuild_from_log
/// — the same deterministic machinery as cache rebuild), then compare clip
/// placements via the EDL. `tracks_touched` is scanned from the in-between
/// ops' recorded effects (removed_ms / added_ms / gap_filled_ms ranges),
/// merged per track.
pub fn diff(
    project: &Project,
    all_ops: &[OpRecord],
    from: &LogRef,
    to: &LogRef,
) -> Result<DiffSummary, CutError> {
    // "now" = the log head (current-head resolution contract: the docs' canonical pre-render
    // invocation is diff{from: last_checkpoint, to:"now"} — make it run as
    // written). Resolved HERE rather than in resolve_ref because only diff
    // has the log in hand; project.revert keeps checkpoint/op-id refs only
    // ("revert to now" would be a no-op request, refusing it is honest).
    let head = || -> Result<String, CutError> {
        all_ops.last().map(|o| o.op_id.clone()).ok_or_else(|| {
            CutError::new(
                crate::error::codes::NOT_FOUND,
                "'now' has nothing to resolve to — the op log is empty",
                "a project always logs project.create as op 1; an empty log means no project",
            )
        })
    };
    let from_op = if from == "now" {
        head()?
    } else {
        resolve_ref(project, from)?
    };
    let to_op = if to == "now" {
        head()?
    } else {
        resolve_ref(project, to)?
    };
    let idx_of = |id: &str| -> Result<usize, CutError> {
        all_ops.iter().position(|o| o.op_id == id).ok_or_else(|| {
            CutError::new(
                crate::error::codes::NOT_FOUND,
                format!("op '{id}' not found in the log"),
                "diff endpoints must be existing checkpoints or op ids",
            )
        })
    };
    let (fi, ti) = (idx_of(&from_op)?, idx_of(&to_op)?);
    if fi > ti {
        return Err(CutError::new(
            crate::error::codes::INVALID_ARGS,
            format!("'{from_op}' comes after '{to_op}' in the log"),
            "diff(from, to) requires from ≤ to; swap the arguments",
        ));
    }
    // Replay both endpoint states (inclusive prefixes — at_op semantics).
    let state_from = crate::store::rebuild_from_log(&all_ops[..=fi])?;
    let state_to = crate::store::rebuild_from_log(&all_ops[..=ti])?;
    let ops: Vec<OpRecord> = all_ops[fi + 1..=ti].to_vec();

    // Clip placement map: id → (track, timeline_in_ms, timeline_out_ms) from
    // the EDL. End time changes are reported for explicit timing edits, but a
    // split/ripple left-remnant is summarized by the new right-half id plus the
    // shifted downstream clips instead of being double-counted as "moved".
    let placements = |p: &Project| -> std::collections::BTreeMap<String, (String, u64, u64)> {
        crate::edl::edl_from_project(p)
            .segments
            .iter()
            .filter_map(|s| {
                Some((
                    s.clip_id.clone()?,
                    (s.track.clone(), s.timeline_in_ms, s.timeline_out_ms),
                ))
            })
            .collect()
    };
    let (pf, pt) = (placements(&state_from), placements(&state_to));
    let clips_added: Vec<String> = pt
        .keys()
        .filter(|k| !pf.contains_key(*k))
        .cloned()
        .collect();
    let clips_removed: Vec<String> = pf
        .keys()
        .filter(|k| !pt.contains_key(*k))
        .cloned()
        .collect();
    let explicit_timing_changes = explicit_timing_change_clip_ids(&ops);
    let clips_moved: Vec<String> = pt
        .iter()
        .filter(|(k, new)| {
            pf.get(*k).is_some_and(|old| {
                let moved_position = old.0 != new.0 || old.1 != new.1;
                let changed_duration = old.2 != new.2;
                moved_position || (changed_duration && explicit_timing_changes.contains(*k))
            })
        })
        .map(|(k, _)| k.clone())
        .collect();

    // Per-track touched ranges from the recorded op effects.
    let mut by_track: std::collections::BTreeMap<String, Vec<[u64; 2]>> = Default::default();
    for op in &ops {
        for e in &op.effects {
            let Some(track) = &e.track else { continue };
            for r in detail_timeline_ranges(&e.detail) {
                by_track.entry(track.clone()).or_default().push(r);
            }
        }
    }
    let tracks_touched = by_track
        .into_iter()
        .map(|(track, mut ranges)| {
            // Merge overlapping/adjacent ranges so the review rail gets clean spans.
            ranges.sort_unstable();
            let mut merged: Vec<[u64; 2]> = Vec::new();
            for r in ranges {
                match merged.last_mut() {
                    Some(last) if r[0] <= last[1] => last[1] = last[1].max(r[1]),
                    _ => merged.push(r),
                }
            }
            TrackTouch {
                track,
                ranges_ms: merged,
            }
        })
        .collect();

    Ok(DiffSummary {
        from_op,
        to_op,
        ops,
        clips_added,
        clips_removed,
        clips_moved,
        duration_delta_ms: state_to.duration_ms() as i64 - state_from.duration_ms() as i64,
        tracks_touched,
    })
}

/// Resolve a LogRef to a concrete op id using the project's checkpoints.
/// Public so `project.revert` shares the exact same resolution rules.
pub fn resolve_ref(project: &Project, r: &LogRef) -> Result<String, CutError> {
    // Checkpoint id or name first (ids are stable, names are human-friendly).
    if let Some(cp) = project
        .checkpoints
        .iter()
        .find(|c| c.id == *r || c.name == *r)
    {
        return Ok(cp.at_op.clone());
    }
    // Fall back to a raw op id of the canonical form.
    if r.starts_with("op_") {
        return Ok(r.clone());
    }
    Err(CutError::new(
        crate::error::codes::NOT_FOUND,
        format!("'{r}' is neither a checkpoint nor an op id"),
        "use project.state to list checkpoints, or project.ops for op ids",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit;
    use serde_json::json;

    #[test]
    fn detail_timeline_ranges_include_review_ranges_but_not_source_ranges() {
        let effect = edit::fx(
            Some("v1"),
            json!({
                "range_ms": [1000, 2000],
                "old_range_ms": [500, 900],
                "src_range_ms": [9000, 12000],
                "old_src_ms": [4000, 8000],
                "added_ms": [3000, 4000],
                "rippled_gap_ms": [4000, 4500],
                "slot_ms": 1000,
                "at_ms": 3000
            }),
        );

        let mut ranges = detail_timeline_ranges(&effect.detail);
        ranges.sort_unstable();
        assert_eq!(
            ranges,
            vec![[500, 900], [1000, 2000], [3000, 4000], [4000, 4500]]
        );
    }
}
