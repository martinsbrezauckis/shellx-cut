use super::build::{add_entry, clean_text};
use super::model::*;
use cut_core::TrackKind;
use std::collections::BTreeSet;

pub(super) fn marker_entries(
    entries: &mut Vec<EvidenceEntry>,
    snapshot: &ProjectSnapshot,
    selected: &BTreeSet<String>,
) {
    let mut project = snapshot.project.clone();
    project.ensure_sequence_bank();
    project.sync_active_sequence();
    for sequence in project.sequences.clone() {
        let mut view = project.clone();
        if !view.switch_sequence(&sequence.id) {
            continue;
        }
        let edl = cut_core::edl_from_project(&view);
        for marker in &sequence.markers {
            let segment = edl
                .segments
                .iter()
                .filter(|segment| {
                    segment
                        .asset
                        .as_ref()
                        .is_some_and(|asset| selected.contains(asset))
                        && segment.timeline_in_ms <= marker.at_ms
                        && marker.at_ms < segment.timeline_out_ms
                })
                .min_by_key(|segment| match segment.track_kind {
                    TrackKind::Video => (0, segment.track.as_str()),
                    TrackKind::Audio => (1, segment.track.as_str()),
                    TrackKind::Caption => (2, segment.track.as_str()),
                });
            let Some(segment) = segment else { continue };
            let (Some(asset), Some(src_in), Some(src_out)) = (
                segment.asset.as_deref(),
                segment.src_in_ms,
                segment.src_out_ms,
            ) else {
                continue;
            };
            let offset = cut_core::tl_off_to_src(
                marker.at_ms.saturating_sub(segment.timeline_in_ms),
                segment.speed,
            );
            let anchor = if segment.reverse {
                src_out.saturating_sub(offset).max(src_in)
            } else {
                src_in.saturating_add(offset).min(src_out)
            };
            let excerpt = clean_text(
                &format!(
                    "{}{}",
                    marker.label,
                    marker
                        .note
                        .as_deref()
                        .map(|note| format!(" · {note}"))
                        .unwrap_or_default()
                ),
                320,
            );
            let marker_json =
                serde_json::to_vec(&(sequence.id.as_str(), marker)).unwrap_or_default();
            let provenance = digest_bytes(&marker_json);
            add_entry(
                entries,
                asset,
                "marker",
                [anchor, anchor.saturating_add(1)],
                anchor,
                excerpt,
                None,
                &provenance,
                Some((&sequence.id, marker.at_ms, &marker.id)),
            );
        }
    }
}
