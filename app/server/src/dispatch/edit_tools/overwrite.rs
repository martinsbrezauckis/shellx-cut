//! overwrite.rs — source-window resolution for one atomic no-ripple overwrite.

use super::*;

/// `edit.overwrite` fills `[at_ms, at_ms + source_duration)` on the explicit
/// video and/or audio destinations. Unlike insert, it removes material under
/// that interval and never shifts material after it; both destinations commit
/// together as one durable core operation.
pub(in crate::dispatch) async fn edit_overwrite(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale/group_id are recorded by commit_core
    struct Args {
        asset: String,
        at_ms: u64,
        video_track: Option<String>,
        audio_track: Option<String>,
        src_range_ms: Option<[u64; 2]>,
        source_in_ms: Option<u64>,
        source_out_ms: Option<u64>,
        duration_ms: Option<u64>,
        rationale: Option<String>,
        group_id: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.video_track.is_none() && a.audio_track.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.overwrite needs video_track, audio_track, or both",
            "choose the destination tracks explicitly; overwrite never guesses a target",
        ));
    }
    let supplied_range = match (a.src_range_ms, a.source_in_ms, a.source_out_ms) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "pass src_range_ms or source_in_ms/source_out_ms, not both",
                "the two forms name the same source window",
            ))
        }
        (Some(range), None, None) => Some(range),
        (None, Some(_), None) | (None, None, Some(_)) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "source_in_ms and source_out_ms must be supplied together",
                "pass both Source Monitor marks, or use src_range_ms:[in_ms,out_ms]",
            ))
        }
        (None, Some(source_in_ms), Some(source_out_ms)) => Some([source_in_ms, source_out_ms]),
        (None, None, None) => None,
    };

    let normalized_range = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;
        validate_destinations(project, a.video_track.as_deref(), a.audio_track.as_deref())?;
        let source = project.assets.get(&a.asset).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{}' in the project", a.asset),
                "import the source asset before overwriting",
            )
        })?;
        let probe = source.probe.as_ref();
        let kind = probe
            .and_then(|p| p.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let duration = probe
            .and_then(|p| p.get("duration_ms"))
            .and_then(Value::as_u64);

        if kind == "image" {
            if a.audio_track.is_some() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "asset '{}' is a still image and has no audio stream",
                        a.asset
                    ),
                    "use a video-only overwrite for stills",
                ));
            }
            if supplied_range.is_some() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "a still-image overwrite uses duration_ms, not source marks",
                    "set duration_ms to the fixed timeline slot the image should overwrite",
                ));
            }
            let duration_ms = a.duration_ms.ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "a still-image overwrite needs duration_ms",
                    "stills have no intrinsic source duration; choose the replacement slot length",
                )
            })?;
            if duration_ms == 0 {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "duration_ms must be greater than zero",
                    "an overwrite must replace a non-empty timeline interval",
                ));
            }
            [0, duration_ms]
        } else {
            if a.duration_ms.is_some() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "duration_ms is only valid for still-image assets",
                    "use Source In/Out or src_range_ms for timed media",
                ));
            }
            if kind == "audio" && a.video_track.is_some() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "audio-only asset '{}' cannot overwrite a video track",
                        a.asset
                    ),
                    "choose audio_track only, or select a source with video",
                ));
            }
            if a.audio_track.is_some()
                && probe
                    .and_then(|p| p.get("has_audio"))
                    .and_then(Value::as_bool)
                    == Some(false)
            {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("asset '{}' has no audio stream", a.asset),
                    "choose video_track only, or select a source with audio",
                ));
            }
            match supplied_range {
                Some(range) => range,
                None => {
                    let duration_ms = duration.ok_or_else(|| {
                        CutError::new(
                            error_codes::INVALID_ARGS,
                            format!("asset '{}' has no probe duration", a.asset),
                            "wait for media.probe, or provide Source In/Out explicitly",
                        )
                    })?;
                    [0, duration_ms]
                }
            }
        }
    };
    if normalized_range[0] >= normalized_range[1] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "source range [{}, {}) is empty or inverted",
                normalized_range[0], normalized_range[1]
            ),
            "Source Out must be later than Source In",
        ));
    }

    // The durable operation always records the normalized source range. This
    // makes replay independent of a later probe refresh or Source Monitor state.
    let mut committed_args = args;
    let map = committed_args
        .as_object_mut()
        .expect("input schema accepts only object arguments");
    map.insert("src_range_ms".into(), json!(normalized_range));
    map.remove("source_in_ms");
    map.remove("source_out_ms");
    map.remove("duration_ms");
    commit_core(state, "edit.overwrite", committed_args, actor).await
}

fn validate_destinations(
    project: &cut_core::Project,
    video_track: Option<&str>,
    audio_track: Option<&str>,
) -> Result<(), CutError> {
    for (track_id, expected_kind, field) in [
        (video_track, cut_core::TrackKind::Video, "video_track"),
        (audio_track, cut_core::TrackKind::Audio, "audio_track"),
    ] {
        let Some(track_id) = track_id else {
            continue;
        };
        let actual = project.track(track_id).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no track '{track_id}'"),
                format!("{field} must name an existing {expected_kind:?} track"),
            )
        })?;
        if actual.kind != expected_kind {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("track '{track_id}' is not a {expected_kind:?} track"),
                format!("pass a {expected_kind:?} track id in {field}"),
            ));
        }
    }
    Ok(())
}
