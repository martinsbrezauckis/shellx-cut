//! Laid timeline layout projection for Sequence Index navigation rows.

use cut_core::{edl_from_project, Project, Sequence};
use std::collections::{BTreeMap, VecDeque};

/// Keep media ranges by id (a speed-ramped clip can expand to several EDL
/// segments) and anonymous gaps in track order. Captions have absolute ranges
/// in the project model and are projected directly by the row collector.
#[derive(Default)]
pub(crate) struct LaidTrackLayout {
    pub(crate) media_ranges: BTreeMap<String, (u64, u64)>,
    pub(crate) gap_ranges: VecDeque<(u64, u64)>,
}

/// The Sequence Index is a navigation surface, so row positions must use the
/// same laid timeline layout as rendering and `ui.playhead`. In
/// particular, the EDL pulls every clip after a valid crossfade back by its
/// overlap; summing clip durations here would instead report the nominal
/// clip-duration cursor and make cross-sequence navigation seek too late.
pub(crate) fn laid_layouts(
    project: &Project,
    sequence: &Sequence,
) -> BTreeMap<String, LaidTrackLayout> {
    // `project_sequence_index` already works from an isolated clone. Switch a
    // second clone to this sequence so cut_core's EDL remains the one canonical
    // source of timeline layout rather than recreating crossfade/ramp math in
    // this read-only row projector.
    let mut sequence_project = project.clone();
    let switched = sequence_project.switch_sequence(&sequence.id);
    debug_assert!(switched);
    if !switched {
        return BTreeMap::new();
    }
    let edl = edl_from_project(&sequence_project);
    let mut layouts: BTreeMap<String, LaidTrackLayout> = BTreeMap::new();

    for segment in edl.segments {
        let layout = layouts.entry(segment.track).or_default();
        match (segment.clip_id, segment.asset, segment.caption_text) {
            (Some(clip_id), Some(_asset), _) => {
                let range = layout
                    .media_ranges
                    .entry(clip_id)
                    .or_insert((segment.timeline_in_ms, segment.timeline_out_ms));
                range.0 = range.0.min(segment.timeline_in_ms);
                range.1 = range.1.max(segment.timeline_out_ms);
            }
            (None, _, None) => layout
                .gap_ranges
                .push_back((segment.timeline_in_ms, segment.timeline_out_ms)),
            _ => {}
        }
    }
    layouts
}
