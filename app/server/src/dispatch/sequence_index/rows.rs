//! Row projection and filtering for `project.sequence_index`.

use cut_core::{Asset, Clip, Project, TrackKind};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) type IndexedRow = (usize, u64, u8, String, Value);

pub(super) struct RowFilters<'a> {
    pub kind: &'a str,
    pub sequence: Option<&'a str>,
    pub track_kind: Option<&'a str>,
    pub status: &'a str,
    pub terms: &'a [String],
    pub include_gaps: bool,
}

#[derive(Clone, Copy)]
struct StatusFacts {
    gap: bool,
    offline: bool,
    effect_count: usize,
    visible: bool,
    locked: bool,
    muted: bool,
}

fn track_kind(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Caption => "caption",
    }
}

fn source_label(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn asset_is_offline(project_dir: &Path, asset: &Asset) -> bool {
    let path = PathBuf::from(&asset.path);
    let resolved = if path.is_relative() {
        project_dir.join(path)
    } else {
        path
    };
    !resolved.is_file()
}

fn status_matches(status: &str, facts: StatusFacts) -> bool {
    match status {
        "all" => true,
        "issues" => facts.gap || facts.offline,
        "offline" => facts.offline,
        "gaps" => facts.gap,
        "effects" => facts.effect_count > 0,
        "hidden" => !facts.visible,
        "locked" => facts.locked,
        "muted" => facts.muted,
        _ => false,
    }
}

fn matches_terms(terms: &[String], fields: &[&str]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let haystack = fields.join(" ").to_lowercase();
    terms.iter().all(|term| haystack.contains(term))
}

pub(super) fn collect_rows(
    project_dir: &Path,
    project: &Project,
    filters: RowFilters<'_>,
) -> Vec<IndexedRow> {
    let offline_assets: std::collections::BTreeMap<String, bool> = project
        .assets
        .iter()
        .map(|(id, asset)| (id.clone(), asset_is_offline(project_dir, asset)))
        .collect();
    let active_sequence = &project.active_sequence;
    let mut rows = Vec::new();

    for (sequence_order, sequence) in project.sequences.iter().enumerate() {
        if filters
            .sequence
            .is_some_and(|id| id != sequence.id.as_str())
        {
            continue;
        }
        let active = sequence.id == *active_sequence;
        if filters.kind != "marker" {
            for track in &sequence.tracks {
                let row_track_kind = track_kind(track.kind);
                if filters
                    .track_kind
                    .is_some_and(|kind| kind != row_track_kind)
                {
                    continue;
                }
                let mut cursor_ms = 0_u64;
                for clip in &track.clips {
                    let duration_ms = clip.timeline_duration_ms();
                    match clip {
                        Clip::Gap(_) if filters.include_gaps => {
                            let facts = StatusFacts {
                                gap: true,
                                offline: false,
                                effect_count: 0,
                                visible: track.visible,
                                locked: track.locked,
                                muted: track.muted,
                            };
                            if status_matches(filters.status, facts)
                                && matches_terms(
                                    filters.terms,
                                    &[
                                        &sequence.name,
                                        &sequence.id,
                                        &track.id,
                                        row_track_kind,
                                        "timeline gap",
                                        "gap",
                                    ],
                                )
                            {
                                let id = format!("gap:{}:{cursor_ms}", track.id);
                                rows.push((
                                    sequence_order,
                                    cursor_ms,
                                    0,
                                    id.clone(),
                                    json!({
                                        "kind": "clip",
                                        "sequence_id": sequence.id,
                                        "sequence_name": sequence.name,
                                        "active": active,
                                        "id": id,
                                        "at_ms": cursor_ms,
                                        "end_ms": cursor_ms.saturating_add(duration_ms),
                                        "track_id": track.id,
                                        "track_kind": row_track_kind,
                                        "clip_kind": "gap",
                                        "label": "Timeline gap",
                                        "effect_count": 0,
                                        "effects": [],
                                        "offline": false,
                                        "track_visible": track.visible,
                                        "track_locked": track.locked,
                                        "track_muted": track.muted,
                                        "issues": ["gap"],
                                    }),
                                ));
                            }
                            cursor_ms = cursor_ms.saturating_add(duration_ms);
                        }
                        Clip::Gap(_) => {
                            cursor_ms = cursor_ms.saturating_add(duration_ms);
                        }
                        Clip::Media(media) => {
                            let asset = project.assets.get(&media.asset);
                            let asset_label = asset
                                .map(|asset| source_label(&asset.path))
                                .unwrap_or(media.asset.as_str());
                            let offline = offline_assets.get(&media.asset).copied().unwrap_or(true);
                            let effects: Vec<&str> =
                                media.effects.iter().map(|effect| effect.kind()).collect();
                            let effects_text = effects.join(" ");
                            let mut state_terms = Vec::new();
                            if offline {
                                state_terms.push("offline");
                            }
                            if !track.visible {
                                state_terms.push("hidden");
                            }
                            if track.locked {
                                state_terms.push("locked");
                            }
                            if track.muted {
                                state_terms.push("muted");
                            }
                            let state_text = state_terms.join(" ");
                            let issues = if offline { vec!["offline"] } else { Vec::new() };
                            let facts = StatusFacts {
                                gap: false,
                                offline,
                                effect_count: effects.len(),
                                visible: track.visible,
                                locked: track.locked,
                                muted: track.muted,
                            };
                            if status_matches(filters.status, facts)
                                && matches_terms(
                                    filters.terms,
                                    &[
                                        &sequence.name,
                                        &sequence.id,
                                        &track.id,
                                        row_track_kind,
                                        &media.id,
                                        &media.asset,
                                        asset_label,
                                        &effects_text,
                                        &state_text,
                                    ],
                                )
                            {
                                rows.push((
                                    sequence_order,
                                    cursor_ms,
                                    0,
                                    track.id.clone(),
                                    json!({
                                        "kind": "clip",
                                        "sequence_id": sequence.id,
                                        "sequence_name": sequence.name,
                                        "active": active,
                                        "id": media.id,
                                        "at_ms": cursor_ms,
                                        "end_ms": cursor_ms.saturating_add(duration_ms),
                                        "track_id": track.id,
                                        "track_kind": row_track_kind,
                                        "clip_kind": "media",
                                        "label": asset_label,
                                        "asset": media.asset,
                                        "src_in_ms": media.src_in_ms,
                                        "src_out_ms": media.src_out_ms,
                                        "effect_count": effects.len(),
                                        "effects": effects,
                                        "offline": offline,
                                        "track_visible": track.visible,
                                        "track_locked": track.locked,
                                        "track_muted": track.muted,
                                        "issues": issues,
                                    }),
                                ));
                            }
                            cursor_ms = cursor_ms.saturating_add(duration_ms);
                        }
                        Clip::Caption(caption) => {
                            let at_ms = caption.range_ms[0];
                            let mut state_terms = Vec::new();
                            if !track.visible {
                                state_terms.push("hidden");
                            }
                            if track.locked {
                                state_terms.push("locked");
                            }
                            let state_text = state_terms.join(" ");
                            let facts = StatusFacts {
                                gap: false,
                                offline: false,
                                effect_count: 0,
                                visible: track.visible,
                                locked: track.locked,
                                muted: track.muted,
                            };
                            if status_matches(filters.status, facts)
                                && matches_terms(
                                    filters.terms,
                                    &[
                                        &sequence.name,
                                        &sequence.id,
                                        &track.id,
                                        row_track_kind,
                                        &caption.id,
                                        &caption.text,
                                        &state_text,
                                    ],
                                )
                            {
                                rows.push((
                                    sequence_order,
                                    at_ms,
                                    0,
                                    track.id.clone(),
                                    json!({
                                        "kind": "clip",
                                        "sequence_id": sequence.id,
                                        "sequence_name": sequence.name,
                                        "active": active,
                                        "id": caption.id,
                                        "at_ms": at_ms,
                                        "end_ms": caption.range_ms[1],
                                        "track_id": track.id,
                                        "track_kind": row_track_kind,
                                        "clip_kind": "caption",
                                        "label": caption.text,
                                        "effect_count": 0,
                                        "effects": [],
                                        "offline": false,
                                        "track_visible": track.visible,
                                        "track_locked": track.locked,
                                        "track_muted": track.muted,
                                        "issues": [],
                                    }),
                                ));
                            }
                        }
                    }
                }
            }
        }
        if filters.kind != "clip" && filters.track_kind.is_none() && filters.status == "all" {
            for marker in &sequence.markers {
                if matches_terms(
                    filters.terms,
                    &[
                        &sequence.name,
                        &sequence.id,
                        &marker.id,
                        &marker.label,
                        marker.note.as_deref().unwrap_or(""),
                        marker.color.as_deref().unwrap_or(""),
                    ],
                ) {
                    rows.push((
                        sequence_order,
                        marker.at_ms,
                        1,
                        marker.id.clone(),
                        json!({
                            "kind": "marker",
                            "sequence_id": sequence.id,
                            "sequence_name": sequence.name,
                            "active": active,
                            "id": marker.id,
                            "at_ms": marker.at_ms,
                            "end_ms": marker.at_ms,
                            "label": marker.label,
                            "note": marker.note,
                            "color": marker.color,
                        }),
                    ));
                }
            }
        }
    }
    rows
}
