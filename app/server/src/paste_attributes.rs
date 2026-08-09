//! paste_attributes.rs — `edit.paste_attributes`.
//!
//! The "Paste Attributes" verb: copy a SOURCE clip's look /
//! geometry / retime / audio treatment onto N target clips in one call. A PURE
//! ORCHESTRATOR (the audio.cleanup_voice / recipe.run pattern): it records NO
//! op of its own — it takes ONE auto-checkpoint, then dispatches the existing
//! replay-safe verbs (edit.grade / edit.transform / edit.crop / edit.speed /
//! edit.gain / edit.fade / edit.effect / edit.eq) per target, and stops +
//! reports the checkpoint on the first failure (never half-applied silently).
//!
//! Copy semantics per category (`which`):
//!   grade      → edit.grade with the source's parametric+LUT values
//!   transform  → edit.transform (x/y/scale/opacity) + edit.crop when the
//!                source HAS one (a source without a crop skips the crop —
//!                reported in `skipped`, never a silent no-op)
//!   speed      → edit.speed {factor} (1.0 copies as an explicit reset)
//!   volume     → edit.gain {db} for AUDIO-track sources + edit.fade when the
//!                source HAS fades. Video clip gain is skipped because video
//!                tracks contribute pixels only; their visual fades still copy.
//!   effects    → edit.effect with the source's full effects list (SET
//!                semantics — an empty list honestly clears the target's) +
//!                edit.eq when the source HAS an EQ
//!
//! Attributes the source simply doesn't carry are SKIPPED and listed in the
//! result's `skipped` array — the caller always sees what was and wasn't
//! copied. Kind mismatches (e.g. grade onto an audio clip) fail through the
//! sub-verb's own validation and surface as the failed step + revert hint.
//!
//! Callers: dispatch.rs route "edit.paste_attributes". Deps: dispatch_send
//! (sub-verb dispatch), AppState, cut-core types.

use crate::dispatch::{dispatch_send, no_project, parse_args};
use crate::state::AppState;
use cut_core::{error_codes, Actor, Clip, CutError, TrackKind, VerbResult};
use serde_json::{json, Value};
use std::collections::HashMap;

const CATEGORIES: [&str; 5] = ["grade", "transform", "speed", "volume", "effects"];

/// The source clip's copyable attributes, extracted under one read lock.
struct SourceAttrs {
    audio_track: bool,
    grade: Option<Value>,
    transform: Option<Value>,
    crop: Option<Value>,
    speed: f64,
    gain_db: f64,
    fade: Option<Value>,
    effects: Value,
    eq: Option<Value>,
}

pub(crate) async fn edit_paste_attributes(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        from_clip: String,
        to_clips: Vec<String>,
        which: Vec<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;

    if a.which.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "`which` must name at least one attribute category".to_string(),
            format!("valid categories: {}", CATEGORIES.join(", ")),
        ));
    }
    for w in &a.which {
        if !CATEGORIES.contains(&w.as_str()) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown attribute category '{w}'"),
                format!("valid categories: {}", CATEGORIES.join(", ")),
            ));
        }
    }
    // Pasting onto the source itself is a no-op — drop it rather than error, so
    // "select all, paste" just works.
    let targets: Vec<String> = a
        .to_clips
        .iter()
        .filter(|c| **c != a.from_clip)
        .cloned()
        .collect();
    if targets.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "`to_clips` has no paste targets (empty, or only the source clip itself)".to_string(),
            "pass at least one clip id different from from_clip",
        ));
    }

    // Extract the source attributes under one scoped read lock (dropped before
    // any sub-dispatch, which takes the write lock).
    let (src, target_kinds): (SourceAttrs, HashMap<String, TrackKind>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (media, source_kind) = store
            .project
            .tracks
            .iter()
            .find_map(|track| {
                track.clips.iter().find_map(|clip| match clip {
                    Clip::Media(media) if media.id == a.from_clip => {
                        Some((media, track.kind))
                    }
                    _ => None,
                })
            })
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no media clip '{}' to copy attributes from", a.from_clip),
                    "from_clip must be a media clip on the current timeline (project.state lists ids)",
                )
            })?;
        let target_ids = &targets;
        let target_kinds = store
            .project
            .tracks
            .iter()
            .flat_map(|track| {
                let kind = track.kind;
                track.clips.iter().filter_map(move |clip| match clip {
                    Clip::Media(media) if target_ids.contains(&media.id) => {
                        Some((media.id.clone(), kind))
                    }
                    _ => None,
                })
            })
            .collect();
        (
            SourceAttrs {
                audio_track: source_kind == TrackKind::Audio,
                grade: media
                    .grade
                    .as_ref()
                    .map(|g| serde_json::to_value(g).unwrap_or(Value::Null)),
                transform: media
                    .transform
                    .as_ref()
                    .map(|t| serde_json::to_value(t).unwrap_or(Value::Null)),
                crop: media
                    .crop
                    .as_ref()
                    .map(|c| serde_json::to_value(c).unwrap_or(Value::Null)),
                speed: media.speed,
                gain_db: media.gain_db,
                fade: media
                    .fade
                    .as_ref()
                    .map(|f| serde_json::to_value(f).unwrap_or(Value::Null)),
                effects: serde_json::to_value(&media.effects).unwrap_or_else(|_| json!([])),
                eq: media
                    .eq
                    .as_ref()
                    .map(|e| serde_json::to_value(e).unwrap_or(Value::Null)),
            },
            target_kinds,
        )
    };

    let why = a.rationale.clone().unwrap_or_else(|| {
        format!(
            "paste attributes from {} ({})",
            a.from_clip,
            a.which.join("+")
        )
    });

    // Build the per-target sub-verb plan once; each entry = (step label, verb, args
    // WITHOUT the clip key). Skipped attributes are reported, not silently dropped.
    let mut plan: Vec<(String, &str, Value)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for w in &a.which {
        match w.as_str() {
            "grade" => match &src.grade {
                Some(g) => {
                    let mut args = g.clone();
                    // ClipGrade fields == edit.grade args (contrast/brightness/
                    // saturation/gamma/temperature_k/lut) — pass through.
                    if let Some(o) = args.as_object_mut() {
                        o.retain(|_, v| !v.is_null());
                    }
                    plan.push(("grade".into(), "edit.grade", args));
                }
                None => skipped.push("grade (source has no grade)".into()),
            },
            "transform" => {
                match &src.transform {
                    Some(t) => plan.push(("transform".into(), "edit.transform", t.clone())),
                    None => skipped.push("transform (source has no transform)".into()),
                }
                match &src.crop {
                    Some(c) => plan.push(("crop".into(), "edit.crop", c.clone())),
                    None => skipped.push("crop (source has no crop)".into()),
                }
            }
            "speed" => plan.push(("speed".into(), "edit.speed", json!({"factor": src.speed}))),
            "volume" => {
                if src.audio_track {
                    plan.push(("gain".into(), "edit.gain", json!({"db": src.gain_db})));
                } else {
                    skipped.push("gain (video tracks contribute no rendered audio)".into());
                }
                match &src.fade {
                    Some(f) => plan.push(("fade".into(), "edit.fade", f.clone())),
                    None => skipped.push("fade (source has no fades)".into()),
                }
            }
            "effects" => {
                plan.push((
                    "effects".into(),
                    "edit.effect",
                    json!({"effects": src.effects}),
                ));
                match &src.eq {
                    Some(e) => plan.push(("eq".into(), "edit.eq", e.clone())),
                    None => skipped.push("eq (source has no EQ)".into()),
                }
            }
            _ => unreachable!("validated above"),
        }
    }
    // Resolve target-specific applicability before creating a checkpoint. An
    // audio source can be pasted across a mixed selection, but gain only has a
    // rendered meaning on audio targets; visual fades remain independently
    // applicable to video targets. Unknown target ids stay in the call list so
    // the underlying verb returns its normal NOT_FOUND error.
    let mut calls: Vec<(String, String, &str, Value)> = Vec::new();
    for clip in &targets {
        for (step, verb, base) in &plan {
            if *verb == "edit.gain"
                && target_kinds
                    .get(clip)
                    .is_some_and(|kind| *kind != TrackKind::Audio)
            {
                skipped.push(format!("gain on {clip} (target is not on an audio track)"));
                continue;
            }
            calls.push((clip.clone(), step.clone(), *verb, base.clone()));
        }
    }

    if calls.is_empty() {
        return Ok(VerbResult::ok(json!({
            "status": "ok",
            "from_clip": a.from_clip,
            "targets": targets,
            "which": a.which,
            "applied": [],
            "skipped": skipped,
            "checkpoint": null,
            "revert_hint": null,
        })));
    }

    // One auto-checkpoint only when at least one sub-verb will run. An all-skipped
    // paste is a successful read-only outcome and must not dirty project history.
    let cp = dispatch_send(
        state,
        "project.checkpoint",
        json!({"name": "before-paste-attributes",
               "rationale": format!("auto: before edit.paste_attributes from {}", a.from_clip)}),
        actor.clone(),
    )
    .await;
    if !cp.ok {
        return Ok(cp);
    }
    let checkpoint_id = cp
        .result
        .as_ref()
        .and_then(|r| r["checkpoint"]["id"].as_str())
        .unwrap_or_default()
        .to_string();

    // Apply the plan to each target; stop on the first failure and hand back the
    // checkpoint so the partial paste is one revert away.
    let mut applied: Vec<Value> = Vec::new();
    for (clip, step, verb, mut call) in calls {
        if let Some(o) = call.as_object_mut() {
            o.insert("clip".into(), json!(clip));
            o.insert("rationale".into(), json!(why));
        }
        let res = dispatch_send(state, verb, call, actor.clone()).await;
        applied.push(
            json!({"clip": clip, "step": step, "verb": verb, "ok": res.ok,
                             "error": res.error}),
        );
        if !res.ok {
            return Ok(VerbResult::ok(json!({
                "status": "failed",
                "failed_step": format!("{step} ({verb}) on {clip}"),
                "applied": applied,
                "skipped": skipped,
                "checkpoint": checkpoint_id,
                "revert_hint": format!(
                    "a step failed mid-paste — project.revert{{to:\"{checkpoint_id}\"}} undoes the partial paste"
                ),
            })));
        }
    }

    Ok(VerbResult::ok(json!({
        "status": "ok",
        "from_clip": a.from_clip,
        "targets": targets,
        "which": a.which,
        "applied": applied,
        "skipped": skipped,
        "checkpoint": checkpoint_id,
        "revert_hint": format!("project.revert{{to:\"{checkpoint_id}\"}} undoes the whole paste in one step"),
    })))
}
