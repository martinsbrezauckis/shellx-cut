//! Validate imported/replayed timeline snapshots before mutating project state.

use crate::error::{codes, CutError};
use crate::types::{Clip, Track};

pub(super) fn validate_tracks(tracks: &[Track]) -> Result<(), CutError> {
    for track in tracks {
        if let Some(mode) = track.blend_mode.as_deref() {
            if !mode.is_empty() && !crate::types::is_valid_blend_mode(mode) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "malformed timeline: invalid track blend mode",
                    format!("track '{}' has unsupported blend mode '{mode}'", track.id),
                ));
            }
        }
        for clip in &track.clips {
            if let Clip::Media(c) = clip {
                if let Some(kind) = c.xfade_kind.as_deref() {
                    if !crate::types::is_valid_transition(kind) {
                        return Err(CutError::new(
                            codes::INVALID_ARGS,
                            "malformed timeline: invalid clip transition",
                            format!(
                                "clip '{}' on track '{}' has unsupported transition '{kind}'",
                                c.id, track.id
                            ),
                        )
                        .with_clip(&c.id));
                    }
                }
                if c.src_in_ms > c.src_out_ms {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        "malformed timeline: media clip source range is inverted",
                        format!(
                            "clip '{}' on track '{}' has src_in_ms {} > src_out_ms {}",
                            c.id, track.id, c.src_in_ms, c.src_out_ms
                        ),
                    )
                    .with_clip(&c.id)
                    .with_suggested_action("repair the timeline snapshot before replaying it"));
                }
            }
        }
    }
    Ok(())
}
