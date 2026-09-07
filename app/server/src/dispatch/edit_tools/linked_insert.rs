//! Atomic linked A/V insertion. The public verb lowers to typed core steps so
//! replay stays stable while a failed audio leg can never leak a video leg.

use super::*;

#[derive(serde::Deserialize)]
#[allow(dead_code)] // rationale is carried by the durable operation record.
struct Args {
    asset: String,
    at_ms: u64,
    video_track: Option<String>,
    audio_track: Option<String>,
    create_video_track: Option<bool>,
    create_audio_track: Option<bool>,
    src_range_ms: Option<[u64; 2]>,
    ripple: Option<bool>,
    rationale: Option<String>,
}

/// `edit.insert_linked` is deliberately a different verb from `edit.insert`:
/// optional second-leg arguments on that single-track surface would leave it
/// unclear whether a caller asked for atomic A/V or a video-only insertion.
pub(in crate::dispatch) async fn edit_insert_linked(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let a: Args = parse_args(args.clone())?;
    let rationale = a.rationale.clone();
    let (op, video_track, audio_track, created_video_track, created_audio_track, src_range, ripple) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        let project = &store.project;
        let src_range = muxed_video_range(project, &a)?;
        let (video_track, created_video_track) = resolve_destination(
            project,
            a.video_track.as_deref(),
            a.create_video_track.unwrap_or(false),
            cut_core::TrackKind::Video,
            "video_track",
            "create_video_track",
        )?;
        let (audio_track, created_audio_track) = resolve_destination(
            project,
            a.audio_track.as_deref(),
            a.create_audio_track.unwrap_or(false),
            cut_core::TrackKind::Audio,
            "audio_track",
            "create_audio_track",
        )?;
        let ripple = a.ripple.unwrap_or(true);
        let mut normalized_args = args;
        normalized_args["src_range_ms"] = json!(src_range);
        normalized_args["ripple"] = json!(ripple);

        let mut steps = Vec::new();
        if created_video_track {
            steps.push(InverseOp {
                verb: "edit.add_track".into(),
                args: json!({"kind": "video", "id": video_track}),
            });
        }
        if created_audio_track {
            steps.push(InverseOp {
                verb: "edit.add_track".into(),
                args: json!({"kind": "audio", "id": audio_track}),
            });
        }
        // Each splice shifts its own target lane. Defer the cross-track ripple
        // until both target clips exist so neither target receives a duplicate
        // gap and every other media/caption/marker target moves exactly once.
        steps.push(InverseOp {
            verb: "edit.insert".into(),
            args: json!({
                "asset": a.asset,
                "track": video_track,
                "at_ms": a.at_ms,
                "src_range_ms": src_range,
                "ripple": false,
            }),
        });
        steps.push(InverseOp {
            verb: "edit.insert".into(),
            args: json!({
                "asset": a.asset,
                "track": audio_track,
                "at_ms": a.at_ms,
                "src_range_ms": src_range,
                "ripple": false,
            }),
        });
        if ripple {
            steps.push(InverseOp {
                verb: "edit._ripple_open_gap".into(),
                args: json!({
                    "exclude_tracks": [video_track, audio_track],
                    "at_ms": a.at_ms,
                    "duration_ms": src_range[1] - src_range[0],
                }),
            });
        }
        let op = guard_call("edit.insert_linked", || {
            store.apply_lowered(
                "edit.insert_linked",
                normalized_args,
                actor,
                rationale,
                steps,
                vec![],
            )
        })?;
        (
            op,
            video_track,
            audio_track,
            created_video_track,
            created_audio_track,
            src_range,
            ripple,
        )
    };

    // `apply_lowered` only returns after commit_staged. Publish afterward so a
    // rejected second insertion produces neither a visible op nor an event.
    let video_clip_id = inserted_clip_id(&op, &video_track)?;
    let audio_clip_id = inserted_clip_id(&op, &audio_track)?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({
            "video_clip_id": video_clip_id,
            "audio_clip_id": audio_clip_id,
            "video_track": video_track,
            "audio_track": audio_track,
            "created_video_track": created_video_track,
            "created_audio_track": created_audio_track,
            "at_ms": a.at_ms,
            "src_range_ms": src_range,
            "ripple": ripple,
        }),
        vec![op_id],
    ))
}

fn muxed_video_range(project: &cut_core::Project, a: &Args) -> Result<[u64; 2], CutError> {
    let source = project.assets.get(&a.asset).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{}' in the project", a.asset),
            "import the source before inserting it",
        )
    })?;
    let probe = source.probe.as_ref();
    let kind = probe
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if kind != "video" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("asset '{}' is not a probed video source", a.asset),
            "use edit.insert for audio, still images, or video without linked audio",
        ));
    }
    if probe
        .and_then(|value| value.get("has_audio"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("asset '{}' has no probed audio stream", a.asset),
            "use edit.insert for video-only placement",
        ));
    }
    let duration = probe
        .and_then(|value| value.get("duration_ms"))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("asset '{}' has no probed duration", a.asset),
                "wait for media.probe before a linked A/V insert",
            )
        })?;
    let range = a.src_range_ms.unwrap_or([0, duration]);
    if range[0] >= range[1] || range[1] > duration {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "src_range_ms [{}, {}) is outside asset '{}'",
                range[0], range[1], a.asset
            ),
            format!("choose a non-empty range within the {}ms source", duration),
        ));
    }
    Ok(range)
}

fn resolve_destination(
    project: &cut_core::Project,
    existing: Option<&str>,
    create: bool,
    expected_kind: cut_core::TrackKind,
    track_field: &str,
    create_field: &str,
) -> Result<(String, bool), CutError> {
    match (existing, create) {
        (Some(_), true) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("pass {track_field} or {create_field}:true, not both"),
            "each linked A/V leg needs exactly one destination strategy",
        )),
        (None, false) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("edit.insert_linked needs {track_field} or {create_field}:true"),
            "choose an existing typed track or create the destination in this operation",
        )),
        (Some(track_id), false) => {
            let track = project.track(track_id).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no track '{track_id}'"),
                    format!("{track_field} must name an existing {expected_kind:?} track"),
                )
            })?;
            if track.kind != expected_kind {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("track '{track_id}' is not a {expected_kind:?} track"),
                    format!("pass a {expected_kind:?} track in {track_field}"),
                ));
            }
            if track.locked {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    format!("track '{track_id}' is locked"),
                    "unlock the linked destination before inserting",
                ));
            }
            Ok((track_id.to_string(), false))
        }
        (None, true) => Ok((next_track_id(project, expected_kind)?, true)),
    }
}

fn next_track_id(
    project: &cut_core::Project,
    kind: cut_core::TrackKind,
) -> Result<String, CutError> {
    let (prefix, suffix) = match kind {
        cut_core::TrackKind::Video => ("v", ""),
        cut_core::TrackKind::Audio => ("a", "t"),
        cut_core::TrackKind::Caption => unreachable!("linked insertion only creates media tracks"),
    };
    let highest = project
        .tracks
        .iter()
        .filter_map(|track| {
            track
                .id
                .strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse::<u64>()
                .ok()
        })
        .max();
    let next = match highest {
        Some(current) => current.checked_add(1).ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                format!("cannot create {kind:?} track: automatic track ID space is exhausted"),
                format!("select an existing {kind:?} destination before inserting"),
            )
        })?,
        None => 1,
    };
    Ok(format!("{prefix}{next}{suffix}"))
}

fn inserted_clip_id(op: &OpRecord, track: &str) -> Result<String, CutError> {
    op.effects
        .iter()
        .find(|effect| {
            effect.track.as_deref() == Some(track) && effect.detail.get("added_clip").is_some()
        })
        .and_then(|effect| effect.detail.get("added_clip"))
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "linked insertion committed without a clip receipt",
                format!("expected an added_clip effect on track '{track}'"),
            )
        })
}
