use super::*;

/// edit.track{clip, bbox|point, start_ms?, end_ms?, every_ms?, engine?, track_scale?,
/// rationale?} — MOTION TRACKING. Runs cv2 (CSRT→template fallback) over the
/// clip's SOURCE across the clip-local window `[start_ms, end_ms]`, and returns the
/// trajectory plus ready-to-apply `pos_x`/`pos_y` (+`scale`) keyframe arrays. It is a
/// MEASUREMENT (non-mutating, like verify.scopes) — the op log stays cv2-free and
/// replay-deterministic. Pipe `keyframes.pos_x`/`pos_y` straight into
/// `edit.keyframe {clip:<overlay>, param:"pos_x"|"pos_y", points:…}` so a title /
/// PiP / blur box FOLLOWS the tracked subject.
pub(in crate::dispatch) async fn edit_track(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        bbox: Option<[f64; 4]>,
        point: Option<[f64; 2]>,
        start_ms: Option<u64>,
        end_ms: Option<u64>,
        every_ms: Option<u64>,
        engine: Option<String>,
        track_scale: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    // Seed: an explicit box OR a point (the runner grows a small box around it).
    let seed = match (&a.bbox, &a.point) {
        (Some(b), _) => crate::track::Seed::Bbox(*b),
        (None, Some(p)) => crate::track::Seed::Point(*p),
        (None, None) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "edit.track needs a seed region: one of bbox [x,y,w,h] or point [x,y]",
                "all values are fractions of the frame (top-left origin)",
            ))
        }
    };
    let engine = a.engine.unwrap_or_else(|| "auto".to_string());
    if !matches!(
        engine.as_str(),
        "auto" | "csrt" | "kcf" | "mil" | "template"
    ) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown tracker engine '{engine}'"),
            "engine: auto | csrt | kcf | mil | template",
        ));
    }
    let track_scale = a.track_scale.unwrap_or(false);

    // Resolve the clip → its source asset + source range + speed (the tracker runs on
    // the SOURCE; clip-local times map through src_in_ms + speed).
    let (project, _edl, _dir, _at) = snapshot(state).await?;
    let (track_id, idx) = project
        .find_clip(&a.clip)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no clip '{}'", a.clip),
                "clip ids come from project.state / edit.* results",
            )
            .with_clip(&a.clip)
        })?;
    let clip = &project.track(&track_id).unwrap().clips[idx];
    let cut_core::Clip::Media(c) = clip else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("'{}' is not a media clip", a.clip),
            "motion tracking needs a media clip (it tracks the source pixels)",
        )
        .with_clip(&a.clip));
    };
    let (asset_id, src_in_ms, src_out_ms, speed) =
        (c.asset.clone(), c.src_in_ms, c.src_out_ms, c.speed);
    let speed = if speed > 0.0 { speed } else { 1.0 };

    // Clip-local window → SOURCE ms. Default end = the clip's full duration.
    let clip_dur_local = (((src_out_ms.saturating_sub(src_in_ms)) as f64) / speed).round() as u64;
    let start_local = a.start_ms.unwrap_or(0).min(clip_dur_local);
    let end_local = a.end_ms.unwrap_or(clip_dur_local).min(clip_dur_local);
    if end_local <= start_local {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("edit.track window is empty (start {start_local}ms ≥ end {end_local}ms)"),
            "give a start_ms/end_ms inside the clip's duration",
        )
        .with_clip(&a.clip));
    }
    let src_start = src_in_ms + (start_local as f64 * speed).round() as u64;
    let src_end = (src_in_ms + (end_local as f64 * speed).round() as u64).min(src_out_ms);
    let every = a.every_ms.unwrap_or(100).max(1);

    let (src_path, _hash) = asset_info(state, &asset_id).await?;
    let rt = crate::track::runtime().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "motion tracking needs the perception sidecar (python + cv2), which is not installed",
            "run system.doctor; install the perception sidecar so its venv carries cv2",
        )
        .with_suggested_action("system.doctor → install the perception sidecar")
    })?;

    let res = run_blocking("edit.track", move || {
        crate::track::run_tracker(&rt, &src_path, &seed, src_start, src_end, every, &engine)
    })
    .await?;

    let kf = crate::track::to_keyframes(&res, src_in_ms, speed, track_scale);
    let first = res.points.first();
    let last = res.points.last();
    let box_json = |p: &crate::track::TrackPoint| json!({"cx": p.cx, "cy": p.cy, "w": p.w, "h": p.h, "t_ms": p.t_ms, "ok": p.ok});
    let mut keyframes = json!({
        "pos_x": kf.pos_x,
        "pos_y": kf.pos_y,
    });
    if let Some(scale) = &kf.scale {
        keyframes["scale"] = json!(scale);
    }
    let points: Vec<Value> = res.points.iter().map(box_json).collect();
    let out = json!({
        "clip": a.clip,
        "engine": res.engine,
        "fps": res.fps,
        "source": {"width": res.width, "height": res.height},
        "window_ms": [start_local, end_local],
        "sampled": res.points.len(),
        "coverage": res.coverage,
        "first": first.map(box_json),
        "last": last.map(box_json),
        "keyframes": keyframes,
        "points": points,
        "apply_hint": "bind to a target: edit.keyframe {clip:\"<overlay>\", param:\"pos_x\", points: keyframes.pos_x} (and pos_y) → it follows the tracked subject",
    });
    Ok(VerbResult::ok(out))
}

/// edit.add_track{kind: video|audio, id?} — empty track for compositing
/// (music bed / overlay). project.create only makes v1/a1t.
pub(in crate::dispatch) async fn edit_add_track(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        kind: String,
        id: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    // Friendlier arg validation than core's serde error for the enum.
    if !matches!(a.kind.as_str(), "video" | "audio") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown track kind '{}'", a.kind),
            "must be video|audio (caption tracks are created by captions.* verbs)",
        ));
    }
    commit_core(state, "edit.add_track", args, actor).await
}

/// edit.remove_track{track, force?} — remove a track (and, with force, its
/// clips). Refuses the last video/audio track and a non-empty track without
/// force. Closes the add-but-never-remove gap; the UI auto-cleans an emptied
/// overlay lane after a delete so no dead row is left behind.
pub(in crate::dispatch) async fn edit_remove_track(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        track: String,
        force: Option<bool>,
    }
    let _a: Args = parse_args(args.clone())?; // arg-shape validation up front
    commit_core(state, "edit.remove_track", args, actor).await
}

/// edit.reorder_track{track, index} — move a track to a new stacking position
/// WITHIN ITS OWN KIND. Video track order = compositing z-order (first video
/// track = base canvas, later video tracks overlay on top), so this brings an
/// overlay layer FORWARD (higher index) or sends it BACK. `index` is
/// GROUP-RELATIVE: it is the target position among tracks of the SAME kind
/// (0 = first track of that kind), translated to the absolute track index
/// internally — a track can never be reordered out of its kind group, which
/// keeps the `[Video…, Audio…, Caption…]` grouping (and the z-order invariant)
/// intact. `index` clamps to [0, same_kind_count-1]. (For the Layer panel, which
/// reorders video tracks and where the video group is the Vec prefix, the
/// group-relative index equals the absolute index — no behavior change.)
pub(in crate::dispatch) async fn edit_reorder_track(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        track: String,
        index: usize,
    }
    let _a: Args = parse_args(args.clone())?; // arg-shape validation up front
    commit_core(state, "edit.reorder_track", args, actor).await
}

/// edit.blend{track, mode?="normal"} — set an overlay video track's LAYER blend
/// mode (multiply/screen/overlay/…) so the whole track composites onto everything
/// below it with that blend, only where the overlay has content. mode "normal" (or
/// omitted) clears it. Core validates the mode + that the track is video. A core op.
pub(in crate::dispatch) async fn edit_blend(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core(state, "edit.blend", args, actor).await
}

/// edit.track_visible{track, on} — visual output visibility for video/caption
/// tracks. Hidden tracks stay editable in the project but do not contribute
/// pixels/subtitles to preview/export. Audio tracks use edit.mute instead.
pub(in crate::dispatch) async fn edit_track_visible(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation up front; core re-parses
    struct Args {
        track: String,
        on: bool,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.track_visible", args, actor).await
}

/// edit.track_lock{track, on} — persisted timeline edit guard. The engine stores
/// the flag; the human timeline UI blocks drag/trim/drop gestures for locked rows.
pub(in crate::dispatch) async fn edit_track_lock(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation up front; core re-parses
    struct Args {
        track: String,
        on: bool,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.track_lock", args, actor).await
}

/// edit.mute{track, on} — NON-DESTRUCTIVE per-track mute. Flags the track muted
/// (or clears it); the track's gain is never touched, so the dialed level survives
/// a reload (this replaces the old -100 dB-gain-write mute that lost the level).
/// A muted track contributes SILENCE to the audio mix (resolved at mix time, see
/// Project::audio_track_audible). Replay-safe: the flag lives in project state and
/// the op is logged like any other edit. Caption tracks are refused (no audio).
pub(in crate::dispatch) async fn edit_mute(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation up front; core re-parses
    struct Args {
        track: String,
        on: bool,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.mute", args, actor).await
}

/// edit.solo{track, on} — NON-DESTRUCTIVE per-track solo. When ANY track is soloed,
/// only soloed tracks are audible; the rest contribute silence WITHOUT touching
/// gain. An explicit mute still wins over solo. Replay-safe; gain untouched; caption
/// tracks refused.
pub(in crate::dispatch) async fn edit_solo(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation up front; core re-parses
    struct Args {
        track: String,
        on: bool,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.solo", args, actor).await
}

/// edit.pan — non-destructive per-track stereo balance.
/// Core validates the range and refuses caption tracks; center = no filter.
pub(in crate::dispatch) async fn edit_pan(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation up front; core re-parses
    struct Args {
        track: String,
        pan: f64,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.pan", args, actor).await
}

/// edit.slip / edit.roll / edit.slide_edit — the pro trim trio (core editing
/// Core logic lives in cut_core::trim_edit; these are thin validation handlers.
pub(in crate::dispatch) async fn edit_slip(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        by_ms: i64,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.slip", args, actor).await
}

pub(in crate::dispatch) async fn edit_roll(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        track: String,
        at_ms: u64,
        by_ms: i64,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.roll", args, actor).await
}

pub(in crate::dispatch) async fn edit_slide_edit(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        by_ms: i64,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.slide_edit", args, actor).await
}

pub(in crate::dispatch) async fn edit_add_marker(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        at_ms: u64,
        label: String,
        note: Option<String>,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.add_marker", args, actor).await
}

pub(in crate::dispatch) async fn edit_remove_marker(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        id: String,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.remove_marker", args, actor).await
}

/// edit.restore{op_id, mode?} — undo a prior op; appends a NEW op (the log is
/// never rewritten, the append-only operation-log contract).
/// - mode:"tip" (DEFAULT) — recompute the target's pre-op journal prefix.
///   TIP-ONLY: a non-tip target returns a guardrail error because that prefix
///   would discard later ops.
/// - mode:"rebase" — selective non-tip undo: reproduce the timeline as if the
///   op never happened, KEEPING later ops. Refused (guardrail, naming the
///   dependents) if any later op references an id the target created.
pub(in crate::dispatch) async fn edit_restore(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        op_id: String,
        /// "tip" (default) | "rebase". Validated by core (store::apply).
        #[serde(default)]
        mode: Option<String>,
    }
    let _a: Args = parse_args(args.clone())?;
    // Core's apply handles BOTH modes end-to-end: tip recomputes the prefix;
    // rebase runs the dependency gate + id-pinned skip-replay
    // + verify, then appends the rebase op (cut_core::store::rebase_out).
    commit_core(state, "edit.restore", args, actor).await
}
