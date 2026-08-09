use super::*;

/// edit.speed{clip, factor, preserve_pitch?=true} — per-clip constant retime
/// (slow-mo / speed-up). Validates the factor range + the pitch mode here, then
/// commits through the core edit verb (replay/diff/restore reproduce it). v1
/// preserves pitch (atempo); varispeed (pitch-follows-speed) is a reserved v2
/// effect, rejected loudly rather than silently ignored.
pub(in crate::dispatch) async fn edit_speed(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        factor: f64,
        #[serde(default = "default_preserve_pitch")]
        preserve_pitch: bool,
        #[allow(dead_code)] // recorded on the op by commit_core (pulled from args)
        rationale: Option<String>,
    }
    fn default_preserve_pitch() -> bool {
        true
    }
    let a: Args = parse_args(args.clone())?;
    if !a.preserve_pitch {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "preserve_pitch:false (varispeed / pitch-follows-speed) is not supported in v1",
            "v1 edit.speed keeps pitch natural (sped-up speech stays human); omit preserve_pitch or pass true",
        )
        .with_clip(&a.clip)
        .with_suggested_action("varispeed is a planned v2 effect; for now retime preserves pitch"));
    }
    if !a.factor.is_finite() || a.factor < 0.25 || a.factor > 4.0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("speed factor {} is out of range", a.factor),
            "factor must be between 0.25 and 4.0 (¼× slow-motion to 4× fast)",
        )
        .with_clip(&a.clip));
    }
    commit_core(state, "edit.speed", args, actor).await
}

/// Speed-ramp factor bounds — the SAME proven range as edit.speed. The audio
/// retime (`audio_speed_filter`) handles [0.25,4.0] with a single sqrt-split
/// atempo; a wider range would need a multi-stage atempo chain (out of v1 scope),
/// so a per-segment ramp factor stays inside the validated window.
const RAMP_FACTOR_MIN: f64 = 0.25;
const RAMP_FACTOR_MAX: f64 = 4.0;

/// edit.speed_ramp{clip, points:[{at_ms, factor}], preserve_pitch?, segments?,
/// rationale?} — VARIABLE speed / time remapping (a "speed
/// curve"): a clip whose playback speed CHANGES over its length, vs the constant
/// edit.speed.
///
/// REPRESENTATION (method A, realized at EDL-derivation time): the clip stores a
/// piecewise-linear speed curve (`points`, source-offset/factor) as ONE field; the
/// EDL EXPANDS it into `segments` contiguous CONSTANT-speed sub-segments sampled
/// from the curve, each rendered by the proven per-segment setpts (video) + atempo
/// (audio) path. So it is a SINGLE, replay-safe op (one clip field, no new ids —
/// unlike a real split, which would need per-split PinnedIds), the SOFTWARE render
/// of any non-ramped clip stays byte-identical, and AUDIO works (pitch-preserved
/// atempo per sub-segment). Smoothness = `segments` (a discrete approximation of
/// the curve; honest, not a continuous warp). The realized timeline length is the
/// integral of (1/speed) over the source = the sum of the sub-segment durations.
///
/// `points:[]` CLEARS the ramp (back to constant speed). preserve_pitch:false
/// (varispeed) is reserved (mirrors edit.speed). The clip must be PLAIN enough for
/// sub-segmentation (no constant speed≠1, reverse, freeze, animation, keyframes,
/// matte, mask, stabilize) — the core verb refuses otherwise with an actionable
/// hint. Receipt: {clip, points, new_duration_ms, method, segments}.
pub(in crate::dispatch) async fn edit_speed_ramp(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        #[serde(default)]
        points: Vec<cut_core::SpeedRampPoint>,
        #[serde(default = "default_preserve_pitch")]
        preserve_pitch: bool,
        segments: Option<usize>,
        #[allow(dead_code)] // recorded on the op by commit_core (pulled from args)
        rationale: Option<String>,
    }
    fn default_preserve_pitch() -> bool {
        true
    }
    let a: Args = parse_args(args.clone())?;
    if !a.preserve_pitch {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "preserve_pitch:false (varispeed / pitch-follows-speed) is not supported in v1",
            "edit.speed_ramp keeps pitch natural per sub-segment (sped-up speech stays human); omit preserve_pitch or pass true",
        )
        .with_clip(&a.clip)
        .with_suggested_action("varispeed is a planned v2 effect; for now the ramp preserves pitch"));
    }
    // CLEAR path: an empty points list removes any existing ramp. Pass through with
    // a resolved (but unused) segments so the stored op is self-contained.
    if a.points.is_empty() {
        let mut norm = args.clone();
        if let Value::Object(m) = &mut norm {
            m.insert("points".into(), json!([]));
            m.insert(
                "segments".into(),
                json!(a.segments.unwrap_or(cut_core::DEFAULT_RAMP_SEGMENTS)),
            );
            m.insert("preserve_pitch".into(), json!(true));
        }
        return commit_core(state, "edit.speed_ramp", norm, actor).await;
    }
    // SET path: validate the curve.
    if a.points.len() < 2 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("a speed ramp needs ≥ 2 control points (got {})", a.points.len()),
            "a ramp interpolates speed BETWEEN points; one point is just a constant speed (use edit.speed)",
        )
        .with_clip(&a.clip)
        .with_suggested_action("pass points like [{at_ms:0,factor:1},{at_ms:1500,factor:3},{at_ms:3000,factor:1}], or [] to clear"));
    }
    for (i, p) in a.points.iter().enumerate() {
        if !p.factor.is_finite() || p.factor < RAMP_FACTOR_MIN || p.factor > RAMP_FACTOR_MAX {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("ramp point {i} factor {} is out of range", p.factor),
                format!("each factor must be between {RAMP_FACTOR_MIN} and {RAMP_FACTOR_MAX} (¼× slow-motion to 4× fast) — the validated retime range"),
            )
            .with_clip(&a.clip));
        }
        if i > 0 && p.at_ms <= a.points[i - 1].at_ms {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("ramp points must be strictly ascending by at_ms (point {i} at {}ms ≤ previous)", p.at_ms),
                "at_ms is the source offset into the clip; give increasing, distinct positions",
            )
            .with_clip(&a.clip));
        }
    }
    // Resolve + clamp the segment granularity (recorded so replay is default-stable).
    let segments = a
        .segments
        .unwrap_or(cut_core::DEFAULT_RAMP_SEGMENTS)
        .clamp(cut_core::MIN_RAMP_SEGMENTS, cut_core::MAX_RAMP_SEGMENTS);
    // Normalize the args the op records: explicit resolved `segments`,
    // preserve_pitch, and an authoritative project timebase. The resolver runs
    // under the commit lock and overwrites any attempted private values.
    let mut norm = args.clone();
    if let Value::Object(m) = &mut norm {
        m.insert("segments".into(), json!(segments));
        m.insert("preserve_pitch".into(), json!(true));
    }
    commit_core_with_project(
        state,
        "edit.speed_ramp",
        norm,
        actor,
        |project, recorded| {
            let Value::Object(args) = recorded else {
                return;
            };
            args.insert("timebase_fps".into(), json!(project.settings.fps));
            args.insert(
                "timebase_audio_rate".into(),
                json!(project.settings.audio_rate),
            );
        },
    )
    .await
}
