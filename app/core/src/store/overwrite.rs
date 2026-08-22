//! store/overwrite.rs — persisted overwrite verb decoding and replay pins.

use crate::error::CutError;
use crate::ops::OpEffect;
use crate::rebase::PinnedIds;
use crate::types::Project;
use serde_json::Value;

/// Decode the normalized durable overwrite args and pass its per-target clip
/// pins to the timeline primitive in target order. New logs always contain the
/// exact source range; missing pins only occur for malformed/manual records and
/// safely fall back to positional allocation like the other core edit verbs.
pub(super) fn apply(
    project: &mut Project,
    args: &Value,
    pinned: Option<&mut PinnedIds>,
) -> Result<Vec<OpEffect>, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        at_ms: u64,
        video_track: Option<String>,
        audio_track: Option<String>,
        src_range_ms: [u64; 2],
    }
    let args: Args = serde_json::from_value(args.clone())?;
    let mut tracks = Vec::with_capacity(2);
    if let Some(track) = args.video_track {
        tracks.push(track);
    }
    if let Some(track) = args.audio_track {
        tracks.push(track);
    }
    let (added_ids, split_ids) = match pinned {
        Some(pins) => {
            let added = (0..tracks.len())
                .filter_map(|_| pins.next_added_clip())
                .collect();
            let mut splits = Vec::new();
            while let Some(id) = pins.next_split_clip() {
                splits.push(id);
            }
            (added, splits)
        }
        None => (Vec::new(), Vec::new()),
    };
    crate::overwrite_edit::overwrite_pinned(
        project,
        &args.asset,
        &tracks,
        args.at_ms,
        args.src_range_ms,
        (!added_ids.is_empty()).then_some(added_ids.as_slice()),
        (!split_ids.is_empty()).then_some(split_ids.as_slice()),
    )
}
