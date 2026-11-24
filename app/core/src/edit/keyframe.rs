//! Set and validate one media clip's parameter keyframe track.
use super::*;

pub fn keyframe(
    project: &mut Project,
    clip_id: &str,
    param: crate::types::KfParam,
    points: Vec<crate::types::KfPoint>,
    interp: crate::types::KfInterp,
) -> Result<Vec<OpEffect>, CutError> {
    keyframe_inner(project, clip_id, param, points, interp, true)
}

// Only replay of a saved operation uses this route. Live edits stay bounded.
pub(crate) fn keyframe_legacy(
    project: &mut Project,
    clip_id: &str,
    param: crate::types::KfParam,
    points: Vec<crate::types::KfPoint>,
    interp: crate::types::KfInterp,
) -> Result<Vec<OpEffect>, CutError> {
    keyframe_inner(project, clip_id, param, points, interp, false)
}

/// Set (or clear) a media clip's KEYFRAMES for ONE parameter (verb `edit.keyframe`).
/// `points` REPLACES that param's keyframe track (SET semantics); an empty list
/// removes it. The renderer animates the param over the clip via an ffmpeg time-
/// expression (opacity → overlay alpha, volume → audio gain) — a keyframed param
/// OVERRIDES its static counterpart. Track validation: `opacity` needs a VIDEO-track
/// clip (animated on the overlay alpha), `volume` needs an AUDIO-track clip. Returns
/// the op effect (old + new keyframe lists) for the receipt/undo.
fn keyframe_inner(
    project: &mut Project,
    clip_id: &str,
    param: crate::types::KfParam,
    mut points: Vec<crate::types::KfPoint>,
    interp: crate::types::KfInterp,
    enforce_bound: bool,
) -> Result<Vec<OpEffect>, CutError> {
    use crate::types::KfParam;
    if enforce_bound && points.len() > crate::keyframe_limit::MAX_EDIT_KEYFRAME_POINTS {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "clip '{clip_id}' has {} keyframe points for {param:?}; the limit is {} per track",
                points.len(),
                crate::keyframe_limit::MAX_EDIT_KEYFRAME_POINTS
            ),
            "exports are only checked for keyframe tracks up to this length",
        )
        .with_clip(clip_id)
        .with_suggested_action("remove or combine points in this keyframe track, then try again"));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let kind = project.track(&track_id).unwrap().kind;
    let timeline_duration_ms = project.track(&track_id).unwrap().clips[idx].timeline_duration_ms();
    let ok = match param {
        KfParam::Opacity | KfParam::PosX | KfParam::PosY | KfParam::Scale => {
            kind == TrackKind::Video
        }
        KfParam::Volume => kind == TrackKind::Audio,
    };
    if !ok {
        let need = match param {
            KfParam::Opacity => "a video-track clip (opacity animates the overlay alpha)",
            KfParam::PosX | KfParam::PosY => {
                "a video-track clip (pos_x/pos_y animate the overlay PiP position)"
            }
            KfParam::Scale => "a video-track clip (scale animates the clip zoom)",
            KfParam::Volume => "an audio-track clip (volume animates the audio gain)",
        };
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("keyframing '{param:?}' needs {need}"),
            "opacity → video clip, volume → audio clip",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "keyframes apply to media clips",
        )
        .with_clip(clip_id));
    };
    if enforce_bound {
        let total = c
            .keyframes
            .iter()
            .filter(|k| k.param != param)
            .fold(points.len(), |count, k| {
                count.saturating_add(k.points.len())
            });
        if total > crate::keyframe_limit::MAX_KEYFRAME_TOTAL_POINTS {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "clip '{clip_id}' has {total} keyframe points in total; the limit is {}",
                    crate::keyframe_limit::MAX_KEYFRAME_TOTAL_POINTS
                ),
                "reduce this clip's keyframe points and try again",
            )
            .with_clip(clip_id)
            .with_suggested_action("reduce this clip's keyframe points and try again"));
        }
    }
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.keyframe"));
    }
    if let Some(point) = points
        .iter()
        .find(|point| point.t_ms > timeline_duration_ms)
    {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "keyframe at {}ms is beyond '{clip_id}' timeline duration of {timeline_duration_ms}ms",
                point.t_ms
            ),
            "keyframe times are clip-local timeline milliseconds after retiming; move the point within the clip duration",
        )
        .with_clip(clip_id));
    }
    // Scale keyframes and edit.animate (Ken Burns) BOTH drive a `zoompan` on a base
    // clip — having both on one clip is ambiguous (two competing zooms). The keyframe
    // channel is the richer (multi-point, eased) form, so reject it when a static
    // animation is already set (clear the animation first). Only enforced when there
    // are actually points to set (an empty Scale list is a CLEAR — always allowed).
    if matches!(param, KfParam::Scale) && !points.is_empty() && c.animation.is_some() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' already has an edit.animate Ken Burns zoom; scale keyframes and edit.animate are mutually exclusive (the keyframe channel is the multi-point eased form)"),
            "clear the animation first: edit.animate {clip, enabled:false}, then set the scale keyframes",
        )
        .with_clip(clip_id));
    }
    // Sort the control points by time (the renderer's expression assumes ordered
    // points); clamp opacity values into [0,1] (volume is an open multiplier; scale
    // is clamped to the zoompan-legal [1,10] at render, matching edit.animate).
    points.sort_by_key(|p| p.t_ms);
    if matches!(param, KfParam::Opacity) {
        for p in &mut points {
            p.value = p.value.clamp(0.0, 1.0);
        }
    }
    let old = c.keyframes.clone();
    // SET semantics: drop any existing track for this param, then add the new one
    // (empty points = just clear). Identity (no keyframes) → byte-identical replay.
    c.keyframes.retain(|k| k.param != param);
    if !points.is_empty() {
        c.keyframes.push(crate::types::Keyframe {
            lead_in: None,
            param,
            points,
            interp,
        });
    }
    let new = c.keyframes.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "param": param, "old_keyframes": old, "keyframes": new}),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Actor, OpRecord, OpStatus};
    use serde_json::json;

    #[test]
    fn new_edit_refuses_old_saved_operation_replays() {
        let mut project = Project::new("limit", crate::ProjectSettings::default());
        project
            .track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(super::super::make_media_clip(
                "c1", "a1", 0, 20_000,
            )));
        let points: Vec<crate::KfPoint> = (0..=crate::keyframe_limit::MAX_EDIT_KEYFRAME_POINTS)
            .map(|t_ms| crate::KfPoint {
                t_ms: t_ms as u64,
                value: 0.5,
            })
            .collect();
        let before = project.clone();
        let edit_error = keyframe(
            &mut project,
            "c1",
            crate::KfParam::Opacity,
            points.clone(),
            crate::KfInterp::Hold,
        )
        .unwrap_err();
        assert_eq!(project, before);
        assert!(edit_error.message.contains("10001 keyframe points"));
        assert!(edit_error.message.contains("limit is 10000"));
        assert!(edit_error.suggested_action.is_some());
        let op = OpRecord {
            op_id: "op_legacy_oversized".into(),
            ts: "2026-01-01T00:00:00.000Z".into(),
            actor: Actor::system(),
            verb: "edit.keyframe".into(),
            args: json!({"clip":"c1","param":"opacity","points":points,"interp":"hold"}),
            rationale: None,
            effects: vec![],
            inverse: None,
            status: OpStatus::Applied,
        };
        crate::store::apply_record(&mut project, &op, &[]).unwrap();
        let Clip::Media(replayed) = &project.track("v1").unwrap().clips[0] else {
            panic!("expected media clip")
        };
        assert_eq!(replayed.keyframes[0].points.len(), points.len());
        keyframe(
            &mut project,
            "c1",
            crate::KfParam::Opacity,
            points[..crate::keyframe_limit::MAX_EDIT_KEYFRAME_POINTS].to_vec(),
            crate::KfInterp::Hold,
        )
        .unwrap();
        let Clip::Media(media) = &project.track("v1").unwrap().clips[0] else {
            panic!("expected media clip")
        };
        assert_eq!(
            media.keyframes[0].points.len(),
            crate::keyframe_limit::MAX_EDIT_KEYFRAME_POINTS
        );
    }

    #[test]
    fn new_keyframe_edit_refuses_absolute_clip_total_but_allows_clearing() {
        let mut project = Project::new("limit", crate::ProjectSettings::default());
        let mut clip = super::super::make_media_clip("c1", "a1", 0, 1_000);
        clip.keyframes.push(crate::Keyframe {
            lead_in: None,
            param: crate::KfParam::PosY,
            points: vec![
                crate::KfPoint {
                    t_ms: 0,
                    value: 0.5
                };
                crate::keyframe_limit::MAX_KEYFRAME_TOTAL_POINTS
            ],
            interp: crate::KfInterp::Hold,
        });
        project
            .track_mut("v1")
            .unwrap()
            .clips
            .push(Clip::Media(clip));
        let error = keyframe(
            &mut project,
            "c1",
            crate::KfParam::PosX,
            vec![crate::KfPoint {
                t_ms: 0,
                value: 0.5,
            }],
            crate::KfInterp::Hold,
        )
        .unwrap_err();
        assert!(error.message.contains("200001"), "{error}");
        assert!(error.message.contains("200000"), "{error}");
        keyframe(
            &mut project,
            "c1",
            crate::KfParam::PosY,
            vec![],
            crate::KfInterp::Hold,
        )
        .unwrap();
    }
}
