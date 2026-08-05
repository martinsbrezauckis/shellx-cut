use super::*;

pub(super) struct LinkedMedia {
    pub clip: String,
    pub track: String,
    pub duration_ms: u64,
}

impl LinkedMedia {
    pub fn move_steps(
        &self,
        primary_clip: &str,
        primary_track: &str,
        at_ms: u64,
        ripple: bool,
    ) -> Vec<cut_core::InverseOp> {
        let mut steps = vec![
            cut_core::InverseOp {
                verb: "edit.move".into(),
                args: json!({
                    "clip": primary_clip,
                    "to_track": primary_track,
                    "at_ms": at_ms,
                    "ripple": false,
                }),
            },
            cut_core::InverseOp {
                verb: "edit.move".into(),
                args: json!({
                    "clip": self.clip,
                    "to_track": self.track,
                    "at_ms": at_ms,
                    "ripple": false,
                }),
            },
        ];
        if ripple {
            steps.push(cut_core::InverseOp {
                verb: "edit._ripple_open_gap".into(),
                args: json!({
                    "exclude_tracks": [primary_track, self.track],
                    "at_ms": at_ms,
                    "duration_ms": self.duration_ms,
                }),
            });
        }
        steps
    }

    pub fn trim_steps(
        &self,
        primary_clip: &str,
        src_in_ms: Option<u64>,
        src_out_ms: Option<u64>,
    ) -> Vec<cut_core::InverseOp> {
        let trim_args = |clip: &str| {
            let mut args = serde_json::Map::new();
            args.insert("clip".into(), json!(clip));
            if let Some(value) = src_in_ms {
                args.insert("src_in_ms".into(), json!(value));
            }
            if let Some(value) = src_out_ms {
                args.insert("src_out_ms".into(), json!(value));
            }
            cut_core::InverseOp {
                verb: "edit.trim".into(),
                args: Value::Object(args),
            }
        };
        vec![trim_args(primary_clip), trim_args(&self.clip)]
    }
}

/// Resolve one exact imported A/V counterpart. Linkage is deliberately
/// inferred from the auto-placement shape rather than stored as a fragile
/// second identity model: opposite kind, same asset/source window, and the
/// same absolute timeline span. Ambiguity is a guardrail error, never a guess.
pub(super) async fn resolve_linked_media(
    state: &AppState,
    clip_id: &str,
    enabled: bool,
    verb: &str,
) -> Result<Option<LinkedMedia>, CutError> {
    if !enabled {
        return Ok(None);
    }
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let project = &store.project;
    let (from_track, idx) = project.find_clip(clip_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no clip '{clip_id}' on the timeline"),
            "clip must be an existing media clip id (project.state lists clips)",
        )
        .with_clip(clip_id)
    })?;
    let from_kind = project
        .track(from_track)
        .expect("find_clip track exists")
        .kind;
    let cut_core::Clip::Media(primary) = &project
        .track(from_track)
        .expect("find_clip track exists")
        .clips[idx]
    else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("clip '{clip_id}' is not a media clip"),
            format!("{verb} accepts video/audio media clips; use captions.* for captions"),
        )
        .with_clip(clip_id));
    };
    let opposite = match from_kind {
        cut_core::TrackKind::Video => Some(cut_core::TrackKind::Audio),
        cut_core::TrackKind::Audio => Some(cut_core::TrackKind::Video),
        cut_core::TrackKind::Caption => None,
    };
    let edl = cut_core::edl_from_project(project);
    let span_for = |id: &str| {
        let mut segments = edl
            .segments
            .iter()
            .filter(|segment| segment.clip_id.as_deref() == Some(id));
        let first = segments.next()?;
        let mut start = first.timeline_in_ms;
        let mut end = first.timeline_out_ms;
        for segment in segments {
            start = start.min(segment.timeline_in_ms);
            end = end.max(segment.timeline_out_ms);
        }
        Some((start, end))
    };
    let primary_span = span_for(clip_id).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("clip '{clip_id}' has no timeline segment"),
            format!("repair the project timeline before using {verb} on this clip"),
        )
        .with_clip(clip_id)
    })?;
    let mut matches = Vec::new();
    if let Some(opposite) = opposite {
        for track in project.tracks.iter().filter(|track| track.kind == opposite) {
            for clip in &track.clips {
                let cut_core::Clip::Media(candidate) = clip else {
                    continue;
                };
                if candidate.asset == primary.asset
                    && candidate.src_in_ms == primary.src_in_ms
                    && candidate.src_out_ms == primary.src_out_ms
                    && span_for(&candidate.id) == Some(primary_span)
                {
                    matches.push((candidate.id.clone(), track.id.clone(), track.locked));
                }
            }
        }
    }
    match matches.as_slice() {
        [] => Ok(None),
        [(clip, track, true)] => Err(CutError::new(
            error_codes::GUARDRAIL,
            format!("linked clip '{clip}' is on locked track '{track}'"),
            format!("applying {verb} to only one half would desynchronize the linked media pair"),
        )
        .with_clip(clip_id)
        .with_suggested_action(
            format!("unlock the linked track, or pass linked:false to deliberately apply {verb} to one clip"),
        )),
        [(clip, track, false)] => Ok(Some(LinkedMedia {
            clip: clip.clone(),
            track: track.clone(),
            duration_ms: primary_span.1 - primary_span.0,
        })),
        _ => Err(CutError::new(
            error_codes::GUARDRAIL,
            format!(
                "clip '{clip_id}' has {} exact linked-media candidates",
                matches.len()
            ),
            format!("the editor cannot choose a linked counterpart without risking the wrong {verb}"),
        )
        .with_clip(clip_id)
        .with_suggested_action(
            format!("remove the duplicate linked candidate, or pass linked:false to apply {verb} to one clip"),
        )),
    }
}
