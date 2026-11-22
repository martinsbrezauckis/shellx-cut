//! Keep saved tracks renderable under the absolute per-clip safety ceiling.
use cut_core::{error_codes, keyframe_limit::MAX_KEYFRAME_TOTAL_POINTS, CutError, Edl, Project};

pub(super) fn require_bounded_keyframes(project: &Project, edl: &Edl) -> Result<(), CutError> {
    cut_core::redaction_limit::check_project(project)?;
    for track in &project.tracks {
        for clip in &track.clips {
            if let cut_core::Clip::Media(media) = clip {
                check_total(&media.id, media.keyframes.iter().map(|k| k.points.len()))?;
            }
        }
    }
    for segment in &edl.segments {
        let clip = segment.clip_id.as_deref().unwrap_or("unknown");
        check_total(clip, segment.keyframes.iter().map(|k| k.points.len()))?;
        for track in &segment.keyframes {
            if track.lead_in.as_ref().is_some_and(|lead| {
                lead.t_ms >= 0
                    || !lead.value.is_finite()
                    || track.points.len() < 2
                    || track.points.first().is_none_or(|p| p.t_ms != 0)
            }) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "invalid keyframe curve lead-in",
                    "a clipped easing needs a finite preceding point before local zero",
                ));
            }
        }
    }
    Ok(())
}

fn check_total(clip: &str, points: impl Iterator<Item = usize>) -> Result<(), CutError> {
    let count = points.fold(0usize, usize::saturating_add);
    if count > MAX_KEYFRAME_TOTAL_POINTS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "clip '{clip}' has {count} keyframe points in total; the limit is {MAX_KEYFRAME_TOTAL_POINTS}"
            ),
            "reduce this clip's keyframe points and try again",
        )
        .with_clip(clip)
        .with_suggested_action("reduce this clip's keyframe points and try again"));
    }
    Ok(())
}
