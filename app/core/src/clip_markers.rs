//! Clip markers (`ClipMarker`) attached to source-media time on [`MediaClip`].
//!
//! Extracted from `types.rs` to keep the shared types file bounded and to hold
//! clip-marker timeline mapping, deterministic duplicate ID assignment, and
//! lowered `edit._set_clip_markers` mutation logic in one owning module.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::edit::{duplicate_pinned, fx};
use crate::error::{codes, CutError};
use crate::ops::OpEffect;
use crate::rebase::PinnedIds;
use crate::types::{src_off_to_tl, Clip, MediaClip, Project};

/// A clip marker attached to source-media time on a [`MediaClip`].
/// Stored in source-media milliseconds (`source_ms`) so Premiere/Resolve
/// semantics hold across move, trim, split, duplicate, speed, and reverse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipMarker {
    pub id: String,
    pub source_ms: u64,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl MediaClip {
    /// Markers whose `source_ms` falls within this clip's visible `[src_in_ms, src_out_ms)` range.
    pub fn visible_markers(&self) -> Vec<&ClipMarker> {
        self.markers
            .iter()
            .filter(|m| m.source_ms >= self.src_in_ms && m.source_ms < self.src_out_ms)
            .collect()
    }

    /// Map a visible marker's `source_ms` to its timeline position (ms), accounting for
    /// clip start time, trim window `[src_in_ms, src_out_ms)`, constant `speed`, and `reverse`.
    pub fn marker_timeline_ms(&self, clip_start_ms: u64, source_ms: u64) -> Option<u64> {
        if self.freeze.is_some()
            || self.speed_ramp.is_some()
            || self.src_out_ms <= self.src_in_ms
            || source_ms < self.src_in_ms
            || source_ms >= self.src_out_ms
        {
            return None;
        }
        let src_off = if self.reverse {
            self.src_out_ms - source_ms
        } else {
            source_ms - self.src_in_ms
        };
        Some(clip_start_ms.saturating_add(src_off_to_tl(src_off, self.speed)))
    }
}

/// Clear source-bound markers when `edit.replace` swaps a clip to a different asset.
pub(crate) fn on_replace_source(clip: &mut MediaClip, old_asset: &str, new_asset: &str) {
    if old_asset != new_asset {
        clip.markers.clear();
    }
}

fn scan_max_cm_id(tracks: &[crate::types::Track], max: &mut u64) {
    for track in tracks {
        for clip in &track.clips {
            if let Clip::Media(media) = clip {
                for marker in &media.markers {
                    if let Some(n) = marker
                        .id
                        .strip_prefix("cm")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        *max = (*max).max(n);
                    }
                }
            }
        }
    }
}

/// Allocate the next monotonic clip marker integer suffix (`cm1`, `cm2`, ...).
pub fn next_clip_marker_seq(project: &Project) -> u64 {
    let mut max = 0u64;
    scan_max_cm_id(&project.tracks, &mut max);
    for seq in &project.sequences {
        scan_max_cm_id(&seq.tracks, &mut max);
    }
    for nest in &project.nests {
        scan_max_cm_id(&nest.tracks, &mut max);
    }
    max + 1
}

/// Allocate clip markers for a duplicated clip (`edit.duplicate`) with fresh
/// deterministic IDs (pinned on replay/rebase, sequential `cm{N}` on live edit).
pub fn duplicate(
    project: &mut Project,
    clip_id: &str,
    ripple: bool,
    mut pinned: Option<&mut PinnedIds>,
) -> Result<Vec<OpEffect>, CutError> {
    let pin_clip = pinned.as_deref().and_then(|p| p.added_clip.clone());
    let mut effects = duplicate_pinned(project, clip_id, ripple, pin_clip.as_deref())?;
    let Some(new_clip_id) = effects
        .first()
        .and_then(|e| e.detail.get("added_clip"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return Ok(effects);
    };

    let marker_count = project
        .find_clip(&new_clip_id)
        .and_then(|(tr, idx)| match &project.track(tr)?.clips[idx] {
            Clip::Media(m) => Some(m.markers.len()),
            _ => None,
        })
        .unwrap_or(0);
    if marker_count == 0 {
        return Ok(effects);
    }

    let mut next_seq = next_clip_marker_seq(project);
    let mut new_ids = Vec::with_capacity(marker_count);
    for _ in 0..marker_count {
        let id = pinned
            .as_deref_mut()
            .and_then(|p| p.next_clip_marker())
            .unwrap_or_else(|| {
                let id = format!("cm{next_seq}");
                next_seq += 1;
                id
            });
        new_ids.push(id);
    }

    if let Some((tr, idx)) = project
        .find_clip(&new_clip_id)
        .map(|(t, i)| (t.to_string(), i))
    {
        if let Clip::Media(m) = &mut project.track_mut(&tr).expect("clip track exists").clips[idx] {
            for (marker, id) in m.markers.iter_mut().zip(new_ids.iter()) {
                marker.id = id.clone();
            }
        }
    }
    if let Some(first) = effects.first_mut() {
        first
            .detail
            .insert("added_clip_markers".into(), json!(new_ids));
    }
    Ok(effects)
}

#[derive(Deserialize)]
struct SetClipMarkersArgs {
    clip: String,
    markers: Vec<ClipMarkerInput>,
}

#[derive(Deserialize)]
struct ClipMarkerInput {
    #[serde(default)]
    id: Option<String>,
    source_ms: u64,
    label: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    color: Option<String>,
}

/// Replace `clip.markers` on a clip inside `project` (lowered `edit._set_clip_markers` step).
pub fn apply_verb(
    project: &mut Project,
    args: &Value,
    mut pinned: Option<&mut PinnedIds>,
) -> Result<Vec<OpEffect>, CutError> {
    let a: SetClipMarkersArgs = serde_json::from_value(args.clone())?;
    let (track_id, idx) = project.find_clip(&a.clip).ok_or_else(|| {
        CutError::new(
            codes::NOT_FOUND,
            format!("no clip '{}' on the timeline", a.clip),
            "edit._set_clip_markers targets an existing media clip",
        )
        .with_clip(&a.clip)
    })?;
    let track_id = track_id.to_string();
    let mut next_seq = next_clip_marker_seq(project);
    let mut resolved = Vec::with_capacity(a.markers.len());
    let mut added_ids = Vec::with_capacity(a.markers.len());
    for entry in a.markers {
        let id = match entry.id.filter(|s| !s.is_empty()) {
            Some(explicit) => explicit,
            None => pinned
                .as_deref_mut()
                .and_then(|p| p.next_clip_marker())
                .unwrap_or_else(|| {
                    let minted = format!("cm{next_seq}");
                    next_seq += 1;
                    minted
                }),
        };
        added_ids.push(id.clone());
        resolved.push(ClipMarker {
            id,
            source_ms: entry.source_ms,
            label: entry.label,
            note: entry.note,
            color: entry.color,
        });
    }
    resolved.sort_by_key(|m| m.source_ms);

    let Clip::Media(clip) = &mut project.track_mut(&track_id).expect("validated track").clips[idx]
    else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("clip '{}' is not a media clip", a.clip),
            "clip markers can only be attached to media clips",
        )
        .with_clip(&a.clip));
    };
    clip.markers = resolved;

    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip": a.clip,
            "added_clip_markers": added_ids,
        }),
    )])
}
