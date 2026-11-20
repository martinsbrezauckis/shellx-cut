//! move_placement.rs — where `edit.move` puts the moved clip on its
//! destination track.
//!
//! A move first lifts the clip (its old slot becomes a gap, so the source
//! track never ripples) and then places it at `at_ms`. Until 0.6.116 the
//! placement was always a splice: it pushed everything after `at_ms` on the
//! destination track right by the clip's length. For a linked A/V move the
//! audio half is placed on its own track at its own time, so the splice put it
//! in front of its own vacated gap and pushed every later audio clip out of
//! sync with its picture (fix list item 176).
//!
//! Live moves without ripple now OVERWRITE the destination slot, the default
//! drag in Premiere Pro and DaVinci Resolve: the clip replaces what lies under
//! `[at_ms, at_ms + duration)`, clips there are trimmed or removed, and every
//! later clip on every track keeps its position. The rippling move keeps the
//! splice and opens the same time on the other tracks (Premiere's insert
//! drag), so nothing goes out of sync either way.
//!
//! The placement is recorded on the operation (`placement` argument). Logged
//! moves without it replay as the historical splice, so opening an older
//! project rebuilds exactly the timeline it was saved with.
use crate::error::{codes, CutError};
use crate::overwrite_edit::{place_over_track, validate_overwrite_track};
use crate::types::{Clip, Project};

/// How a moved clip lands on its destination track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovePlacement {
    /// Splice in at `at_ms` and push the destination track's later clips
    /// right. The historical behaviour and the replay default for logged moves
    /// that carry no `placement`; live moves use it only with `ripple: true`,
    /// where the other tracks open the same time.
    #[default]
    Insert,
    /// Replace whatever lies under the moved clip's new slot; nothing after the
    /// slot moves on any track.
    Overwrite,
}

impl MovePlacement {
    /// The placement a live `edit.move` records for its resolved ripple mode.
    pub fn for_live_move(ripple: bool) -> Self {
        if ripple {
            MovePlacement::Insert
        } else {
            MovePlacement::Overwrite
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MovePlacement::Insert => "insert",
            MovePlacement::Overwrite => "overwrite",
        }
    }
}

/// Overwrite-place the already lifted `clip` on `to_track` at `at_ms`.
/// `split_id` is used only when one existing media clip spans both edges of
/// the slot (its right half needs a fresh id); returns that id when used plus
/// the effect detail of the overwrite (overlapped/removed clips, gap consumed,
/// tail padding). Validation happens on the post-lift track, so the moved
/// clip's own vacated slot never blocks it; the store applies every edit on a
/// staged copy, so a refusal here leaves the project unchanged.
pub(crate) fn overwrite_place(
    project: &mut Project,
    to_track: &str,
    at_ms: u64,
    clip: Clip,
    split_id: String,
) -> Result<(Option<String>, serde_json::Value), CutError> {
    let duration_ms = clip.timeline_duration_ms();
    let end_ms = at_ms.checked_add(duration_ms).ok_or_else(|| {
        CutError::new(
            codes::INVALID_ARGS,
            "the moved clip would end past the largest timeline position",
            "at_ms plus the clip's duration must fit in u64 milliseconds",
        )
    })?;
    let original = project
        .track(to_track)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no track '{to_track}'"),
                "the destination track must exist in the open project",
            )
        })?
        .clone();
    let moved_id = clip.id().map(str::to_string);
    validate_overwrite_track(&original, at_ms, end_ms, duration_ms, "moved clip").map_err(
        |error| match &moved_id {
            Some(id) if error.clip_id.is_none() => error.with_clip(id),
            _ => error,
        },
    )?;
    let detail = place_over_track(project, &original, at_ms, end_ms, clip, Some(split_id));
    let split = detail
        .get("split_clip")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Ok((split, detail))
}
