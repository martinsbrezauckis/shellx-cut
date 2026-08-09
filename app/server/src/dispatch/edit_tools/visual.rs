use super::*;

/// edit.transform{clip, x?, y?, scale?, opacity?} — overlay geometry + opacity
/// on a video clip (normalized: x/y = top-left fraction of frame, scale = width
/// fraction, opacity = 0..1 alpha; identity 0,0,1,1 clears). Read by the renderer
/// for clips on OVERLAY video tracks (any video track after the first) —
/// picture-in-picture, with opacity for blend/ghost looks.
pub(in crate::dispatch) async fn edit_transform(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        x: Option<f64>,
        y: Option<f64>,
        scale: Option<f64>,
        opacity: Option<f64>,
    }
    let _a: Args = parse_args(args.clone())?; // arg-shape validation up front
    commit_core(state, "edit.transform", args, actor).await
}

/// edit.crop{clip, x, y, w, h} — set the clip's SOURCE crop rectangle
/// "framing correctness"). All values are SOURCE PIXELS (not normalized):
/// the rectangle of the source frame to KEEP. Applied by the renderer in
/// source space BEFORE the conform scale/pad and any overlay transform
/// (crop → conform → transform). The canonical use: remove a baked-in
/// letterbox/pillarbox reported as the import's `content_bbox` perception
/// fact (e.g. an OBS canvas/window mismatch). An identity crop (origin +
/// full source size, when the asset is probed) clears the crop. Core
/// bounds-checks against the probed source geometry.
pub(in crate::dispatch) async fn edit_crop(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses + bounds-checks
    struct Args {
        clip: String,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    }
    let _a: Args = parse_args(args.clone())?; // arg-shape validation up front
    commit_core(state, "edit.crop", args, actor).await
}

/// Fence a `.cube` 3D-LUT path at the DISPATCH layer (live only): it must end in
/// `.cube` and exist on disk. We ship no LUT files (keeps the artifact license-clean;
/// pro LUTs are the user's to supply); a bad path is rejected up front, not deferred to
/// a render failure. Core stores the path verbatim so replay never touches the
/// filesystem. Shared by edit.grade and edit.grade_stack (each stack layer).
fn fence_cube_lut(lut: &str) -> Result<(), CutError> {
    let p = std::path::Path::new(lut);
    let is_cube = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cube"));
    if !is_cube {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("lut '{lut}' is not a .cube file"),
            "grading reads a 3D LUT in the .cube format (ffmpeg lut3d)",
        )
        .with_suggested_action(
            "pass a path ending in .cube, or omit lut for parametric-only grading",
        ));
    }
    if !p.is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("lut file '{lut}' not found"),
            "the LUT must exist on disk when the grade is applied",
        )
        .with_suggested_action("pass an absolute path to an existing .cube LUT"));
    }
    Ok(())
}

/// edit.grade — native color grading (parametric eq + optional .cube LUT). The
/// LUT path is FENCED here (live only); see [`fence_cube_lut`].
pub(in crate::dispatch) async fn edit_grade(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses + applies the defaults
    struct Args {
        clip: String,
        contrast: Option<f64>,
        brightness: Option<f64>,
        saturation: Option<f64>,
        gamma: Option<f64>,
        temperature_k: Option<u32>,
        lut: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if let Some(lut) = &a.lut {
        fence_cube_lut(lut)?;
    }
    commit_core(state, "edit.grade", args, actor).await
}

/// edit.grade_stack{clip, grades:[{contrast?,brightness?,saturation?,gamma?,
/// temperature_k?,lut?}…], rationale?} — LAYERED color grading: a node-stack of grade
/// layers applied IN ORDER on one clip (a serial grading workflow vs the single
/// `edit.grade`). Each layer is the SAME shape `edit.grade` accepts; a layer specifies
/// only the knobs it changes (the rest default to identity). A `.cube` LUT on ANY layer
/// is fenced here exactly like `edit.grade`. Identity layers are dropped by core; an
/// empty / all-identity `grades` clears the clip's grade entirely (byte-identical to
/// ungraded). A single non-identity layer renders byte-identical to the equivalent
/// `edit.grade`. Video media clips only (core enforces).
pub(in crate::dispatch) async fn edit_grade_stack(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses each layer into a ClipGrade
    struct LayerArg {
        contrast: Option<f64>,
        brightness: Option<f64>,
        saturation: Option<f64>,
        gamma: Option<f64>,
        temperature_k: Option<u32>,
        lut: Option<String>,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        #[serde(default)]
        grades: Vec<LayerArg>,
    }
    let a: Args = parse_args(args.clone())?;
    // Fence every layer's LUT up front (a bad path on any layer fails the whole op,
    // never a partial render failure).
    for layer in &a.grades {
        if let Some(lut) = &layer.lut {
            fence_cube_lut(lut)?;
        }
    }
    commit_core(state, "edit.grade_stack", args, actor).await
}

/// edit.grade_window{clip, shape?, points?, feather?, invert?, contrast?, brightness?,
/// saturation?, gamma?, temperature_k?, lut?, enabled?, remove_index?, rationale?} —
/// GEOMETRIC POWER WINDOW: a region-scoped grade (a geometric grade window).
/// `shape`/`points`/`feather`/`invert` define the REGION (the same geometry vocabulary as
/// `edit.add_mask`); the grade knobs (same as `edit.grade`) are applied ONLY inside it.
/// Windows APPEND/stack; `remove_index` atomically removes one window; `enabled:false`
/// clears all windows. The grade's `.cube` LUT is fenced here exactly like `edit.grade`.
/// Core enforces video/base-track + identity-grade rules. Routes the timeline to the
/// software render path (region composite).
pub(in crate::dispatch) async fn edit_grade_window(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses + applies the defaults
    struct Args {
        clip: String,
        shape: Option<String>,
        points: Option<Vec<[f64; 2]>>,
        feather: Option<f64>,
        invert: Option<bool>,
        contrast: Option<f64>,
        brightness: Option<f64>,
        saturation: Option<f64>,
        gamma: Option<f64>,
        temperature_k: Option<u32>,
        lut: Option<String>,
        enabled: Option<bool>,
        remove_index: Option<usize>,
    }
    let a: Args = parse_args(args.clone())?;
    if let (None, Some(lut)) = (a.remove_index, &a.lut) {
        fence_cube_lut(lut)?;
    }
    commit_core(state, "edit.grade_window", args, actor).await
}

/// grade.save{clip, name, rationale?} — snapshot a clip's CURRENT single grade into the
/// project's grade GALLERY under `name` (a saved grade preset). A re-save
/// under the same name REPLACES it. Errors actionably when no clip is given, the clip is
/// ungraded, or the clip uses a layered grade stack (a single-grade preset can't capture
/// it). Project metadata op (off the undo cursor) — `grade.apply` is the undoable side.
pub(in crate::dispatch) async fn grade_save(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: Option<String>,
        name: String,
    }
    let a: Args = parse_args(args.clone())?;
    let name = a.name.trim();
    if name.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "preset name is empty".to_string(),
            "grade.save stores the look under a name",
        )
        .with_suggested_action(
            "pass a non-empty name, e.g. grade.save{clip:\"c1\", name:\"look1\"}",
        ));
    }
    let clip_id = a.clip.as_deref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "no clip given".to_string(),
            "grade.save snapshots a CLIP's current grade — pass the clip whose look to save",
        )
        .with_suggested_action("grade.save{clip:\"c1\", name:\"look1\"}")
    })?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    // Read the clip's current single grade (clone it out, then drop the borrow before
    // the mutable save below).
    let grade = {
        let (track_id, idx) = store.project.find_clip(clip_id).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no clip '{clip_id}' on the timeline"),
                "clip must be an existing clip id (project.state lists clips)",
            )
            .with_clip(clip_id)
        })?;
        let cut_core::Clip::Media(c) = &store.project.track(track_id).expect("track").clips[idx]
        else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("clip '{clip_id}' is not a media clip"),
                "a grade lives on a media (video) clip",
            )
            .with_clip(clip_id));
        };
        match &c.grade {
            Some(g) => g.clone(),
            None if !c.grade_stack.is_empty() => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "clip '{clip_id}' uses a layered grade stack ({} layers), not a single grade",
                        c.grade_stack.len()
                    ),
                    "grade.save captures a SINGLE grade preset — it can't snapshot a multi-layer stack",
                )
                .with_suggested_action(
                    "save from a clip graded with edit.grade, or apply a single edit.grade first",
                )
                .with_clip(clip_id));
            }
            None => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{clip_id}' has no grade to save"),
                    "grade.save snapshots a clip's CURRENT grade — this clip is ungraded",
                )
                .with_suggested_action("apply edit.grade to the clip first, then grade.save")
                .with_clip(clip_id));
            }
        }
    };
    let (preset, op) = guard_call("grade.save", || {
        store.save_grade_preset(name, grade, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({ "preset": preset }),
        vec![op_id],
    ))
}

/// grade.apply{clip, name, rationale?} — apply a saved grade preset's params to a TARGET
/// clip ("copy a look between shots"). Resolves the preset, then LOWERS to a plain
/// `edit.grade` with the preset's concrete params — so the recorded op is a normal,
/// replay-safe `edit.grade` (independent of whether the preset still exists later) and
/// it composes / undoes / replays like any per-clip grade. One op.
pub(in crate::dispatch) async fn grade_apply(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        name: String,
    }
    let a: Args = parse_args(args.clone())?;
    // Resolve the preset (clone its grade), dropping the read guard before edit_grade
    // takes the write guard below.
    let grade = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store
            .project
            .grade_presets
            .iter()
            .find(|p| p.name == a.name)
            .map(|p| p.grade.clone())
    };
    let grade = grade.ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no grade preset '{}'", a.name),
            "the gallery has no preset by that name",
        )
        .with_suggested_action("grade.list shows saved presets; grade.save adds one")
    })?;
    // Lower to edit.grade with CONCRETE params (replay-safe; no preset reference in the
    // log). Build the args the edit.grade handler accepts, carrying the rationale.
    let mut lowered = json!({
        "clip": a.clip,
        "contrast": grade.contrast,
        "brightness": grade.brightness,
        "saturation": grade.saturation,
        "gamma": grade.gamma,
    });
    if let Some(k) = grade.temperature_k {
        lowered["temperature_k"] = json!(k);
    }
    if let Some(l) = &grade.lut {
        lowered["lut"] = json!(l);
    }
    if let Some(r) = args.get("rationale").and_then(|r| r.as_str()) {
        lowered["rationale"] = json!(r);
    } else {
        lowered["rationale"] = json!(format!("grade.apply preset '{}'", a.name));
    }
    edit_grade(state, lowered, actor).await
}

/// grade.list{} — list the project's saved grade presets (the gallery). Read-only, no
/// op. Returns each preset as {name, grade} plus a count.
pub(in crate::dispatch) async fn grade_list(state: &AppState) -> Result<VerbResult, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let presets: Vec<_> = store
        .project
        .grade_presets
        .iter()
        .map(|p| json!({ "name": p.name, "grade": p.grade }))
        .collect();
    Ok(VerbResult::ok(
        json!({ "presets": presets, "count": presets.len() }),
    ))
}

/// edit.color_space{clip, input?, rationale?} — tag a media clip's INPUT color space
/// (the source footage's space: a log/sRGB/Rec.2020 source). The renderer converts it
/// INTO the project working space (and on to output) before grade/effects. `input`
/// omitted / null CLEARS the tag (source then assumed already in the working space →
/// renders byte-identical to an untagged clip). Supported: rec709, rec2020, srgb,
/// linear — an unknown name is rejected with an actionable error.
pub(in crate::dispatch) async fn edit_color_space(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses + applies
    struct Args {
        clip: String,
        input: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    // Validate the space name here so a typo errors actionably before commit (core
    // re-validates on replay). None / null = clear the tag.
    if let Some(s) = &a.input {
        if cut_core::ColorSpace::parse(s).is_none() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown color space '{s}'"),
                format!(
                    "supported input spaces: {}",
                    cut_core::ColorSpace::SUPPORTED
                ),
            )
            .with_suggested_action(format!(
                "pass one of: {}, or omit input to clear the tag",
                cut_core::ColorSpace::SUPPORTED
            )));
        }
    }
    commit_core(state, "edit.color_space", args, actor).await
}

/// edit.color_match{clip, reference, strength?=1.0, rationale?} — match a clip's
/// COLOUR / tonality to a REFERENCE clip ("make this shot match that shot", the
/// standard colorist tool). v1: sample ONE representative mid-clip frame from each
/// of `clip` (target) and `reference`, compute per-channel mean+std (RGB
/// Reinhard-style transfer, in cut_media::color_match), and DERIVE an edit.grade
/// correction — brightness ← luma-mean delta, contrast ← luma-std ratio,
/// saturation ← chroma ratio, temperature ← warm/cool (R−B) delta — scaled by
/// `strength` (0 = identity, 1 = full match). The correction is APPLIED through
/// the normal `edit.grade` path, so the result composes / undoes / replays like
/// any per-clip grade. The colour ANALYSIS is live (it decodes frames); the
/// COMMITTED op is a plain, replay-safe grade (no resampling on replay). Matching
/// a clip to itself derives an identity grade (a clean no-op that clears the
/// grade). The receipt carries the derivation + both stat sets honestly.
/// Resolve a media clip id → (absolute asset path, representative mid-clip
/// SOURCE time in seconds) for the pixel-sampling colour verbs
/// (`edit.color_match`, `edit.auto_balance`). MUST be called under the project
/// read lock; it does NO decode (the slow ffmpeg sample runs after the lock
/// drops). Errors actionably when the id is missing, is not a media clip
/// (gaps/captions have no pixels), or its source asset is absent.
fn resolve_clip_asset_path(store: &ProjectStore, id: &str) -> Result<(PathBuf, f64), CutError> {
    let (tid, idx) = store.project.find_clip(id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("clip '{id}' not found"),
            "pass a media clip id that is on the timeline",
        )
        .with_suggested_action("check project.state for the clip ids")
    })?;
    let clip = &store.project.track(tid).expect("track exists").clips[idx];
    let cut_core::Clip::Media(mc) = clip else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("'{id}' is not a media clip"),
            "this verb samples pixels — gaps / caption clips have none",
        )
        .with_clip(id));
    };
    let asset = store.project.assets.get(&mc.asset).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("asset {} for clip '{id}' is missing", mc.asset),
            "the clip's source asset is not in the project",
        )
    })?;
    let mut path = PathBuf::from(&asset.path);
    if path.is_relative() {
        path = store.dir.join(path);
    }
    let mid_s = ((mc.src_in_ms + mc.src_out_ms) as f64 / 2.0) / 1000.0;
    Ok((path, mid_s))
}

pub(in crate::dispatch) async fn edit_color_match(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        reference: String,
        strength: Option<f64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let strength = a.strength.unwrap_or(1.0).clamp(0.0, 1.0);

    let (target_path, target_s, ref_path, ref_s) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (tp, ts) = resolve_clip_asset_path(store, &a.clip)?;
        let (rp, rs) = resolve_clip_asset_path(store, &a.reference)?;
        (tp, ts, rp, rs)
    };

    // Sample both frames + derive the grade (NO lock held; decodes via ffmpeg).
    let mc =
        cut_media::color_match::match_color(&target_path, target_s, &ref_path, ref_s, strength)?;

    // Apply the derived grade through the NORMAL edit.grade path (replay-safe:
    // the committed op carries concrete grade params, never the reference).
    let mut grade_args = serde_json::Map::new();
    grade_args.insert("clip".into(), json!(a.clip));
    grade_args.insert("contrast".into(), json!(mc.derived.contrast));
    grade_args.insert("brightness".into(), json!(mc.derived.brightness));
    grade_args.insert("saturation".into(), json!(mc.derived.saturation));
    grade_args.insert("gamma".into(), json!(mc.derived.gamma));
    if let Some(k) = mc.derived.temperature_k {
        grade_args.insert("temperature_k".into(), json!(k));
    }
    grade_args.insert(
        "rationale".into(),
        json!(a.rationale.clone().unwrap_or_else(|| format!(
            "edit.color_match: matched '{}' to '{}' (strength {strength:.2}, {} space)",
            a.clip, a.reference, mc.space
        ))),
    );
    let committed = commit_core(state, "edit.grade", Value::Object(grade_args), actor).await?;
    let op_ids = committed.op_ids.unwrap_or_default();

    // HONEST receipt: derivation + both stat sets + the committed grade op id.
    let result = json!({
        "clip": a.clip,
        "reference": a.reference,
        "space": mc.space,
        "strength": mc.strength,
        "identity": mc.identity,
        "derived": mc.derived,
        "stats": { "target": mc.target, "reference": mc.reference },
        "sampled_at_s": { "target": target_s, "reference": ref_s },
        "analysis_note": "single representative mid-clip frame per side; plain RGB v1; gamma remains identity and green-magenta tint is not modeled",
        "op_ids": op_ids.clone(),
    });
    Ok(VerbResult::ok_with_ops(result, op_ids))
}

/// edit.auto_balance{clip, strength?=1.0, mode?="gray_world", rationale?} —
/// ONE-CLICK REFERENCE-FREE auto white-balance + exposure (the "Auto Color" /
/// "Balance Color" available in many editors). Unlike
/// `edit.color_match` it needs NO reference clip: it samples ONE representative
/// mid-clip frame of `clip`, NEUTRALISES the frame's OWN colour cast (gray_world
/// = the whole-frame average should be grey; white_patch = the bright
/// near-neutral highlights should be white) and NUDGES exposure toward a
/// mid-luma target, then DERIVES + COMMITS the correction through the normal
/// `edit.grade` path — so it composes / undoes / replays like any per-clip grade
/// (it OVERWRITES any existing grade). `strength` 0 ⇒ an identity grade (clears
/// any auto-balance). The colour ANALYSIS is live (it decodes one frame); the
/// COMMITTED op is a plain, replay-safe grade (concrete params). The receipt
/// carries the frame stats + the derivation honestly. v1 limits mirror
/// color_match: single frame, plain RGB (not LAB), and the warm/cool axis only
/// (`edit.grade` has no green-magenta tint knob).
pub(in crate::dispatch) async fn edit_auto_balance(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        strength: Option<f64>,
        mode: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let strength = a.strength.unwrap_or(1.0).clamp(0.0, 1.0);
    let mode = cut_media::color_match::AutoBalanceMode::parse(a.mode.as_deref())?;

    // Resolve the clip → (asset path, representative mid-clip source time) under
    // the read lock; the slow ffmpeg decode runs after the lock drops.
    let (clip_path, clip_s) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        resolve_clip_asset_path(store, &a.clip)?
    };

    // Sample the frame + derive the auto-balance grade (NO lock held).
    let ab = cut_media::color_match::auto_balance(&clip_path, clip_s, mode, strength)?;

    // Apply the derived grade through the NORMAL edit.grade path (replay-safe:
    // the committed op carries concrete grade params, no resampling on replay).
    let mut grade_args = serde_json::Map::new();
    grade_args.insert("clip".into(), json!(a.clip));
    grade_args.insert("contrast".into(), json!(ab.derived.contrast));
    grade_args.insert("brightness".into(), json!(ab.derived.brightness));
    grade_args.insert("saturation".into(), json!(ab.derived.saturation));
    grade_args.insert("gamma".into(), json!(ab.derived.gamma));
    if let Some(k) = ab.derived.temperature_k {
        grade_args.insert("temperature_k".into(), json!(k));
    }
    grade_args.insert(
        "rationale".into(),
        json!(a.rationale.clone().unwrap_or_else(|| format!(
            "edit.auto_balance: auto white-balance + exposure on '{}' (strength {strength:.2}, {} mode)",
            a.clip,
            mode.as_str()
        ))),
    );
    let committed = commit_core(state, "edit.grade", Value::Object(grade_args), actor).await?;
    let op_ids = committed.op_ids.unwrap_or_default();

    // HONEST receipt: mode, strength, the frame stats (mean_rgb + mean_luma +
    // full channel stats + the white-patch highlight warmth), the derived grade,
    // and the committed grade op id.
    let result = json!({
        "clip": a.clip,
        "mode": ab.mode.as_str(),
        "strength": ab.strength,
        "identity": ab.identity,
        "derived": ab.derived,
        "stats": {
            "mean_rgb": [ab.stats.mean_r, ab.stats.mean_g, ab.stats.mean_b],
            "mean_luma": ab.stats.mean_luma,
            "channel": ab.stats,
            "highlight_warmth": ab.highlight_warmth,
        },
        "sampled_at_s": clip_s,
        "analysis_note": "single representative mid-clip frame; plain RGB v1; saturation and gamma remain identity and green-magenta tint is not modeled",
        "op_ids": op_ids.clone(),
    });
    Ok(VerbResult::ok_with_ops(result, op_ids))
}

/// edit.effect{clip, effects[]} — SET a media clip's visual effects (vignette /
/// sharpen / blur / grain / chroma_key). The typed ClipEffect enum validates
/// each effect at deserialization; chroma key is refused on a base-track clip
/// (core). A core op (replay-safe).
pub(in crate::dispatch) async fn edit_effect(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core(state, "edit.effect", args, actor).await
}

/// edit.adjustment{range_ms, grade?, effect?/effects?, track?} — a non-destructive
/// ADJUSTMENT LAYER: a colour grade / look effect applied across a TIME SPAN to the
/// COMPOSITE of everything beneath it (the adjustment layer), NOT
/// baked per clip. v1 renders it as the TOP-MOST composite layer over its span (so
/// "the tracks beneath" = all video tracks); a single TIME-GATED grade/effect pass on
/// the composite, kept off the filtergraph entirely when no adjustment exists (so a
/// timeline without one renders byte-identical). `grade` is the edit.grade object
/// shape; `effect`/`effects` are edit.effect's VISUAL look effects (audio effects and
/// chroma-key are refused in core). `track` is advisory in v1 (the layer is always
/// top-most) and recorded for the audit trail. The LUT path inside `grade` is fenced
/// HERE (live only — exists + ends .cube), exactly like edit.grade, so replay stays
/// filesystem-independent. A core op (replay-safe); validation + the deterministic
/// adjN id allocation live in cut_core::edit::add_adjustment.
pub(in crate::dispatch) async fn edit_adjustment(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    // Fence a grade LUT before commit (mirror edit_grade): a bad path is rejected up
    // front, never deferred to a render failure. Core stores it verbatim for replay.
    if let Some(lut) = args
        .get("grade")
        .and_then(|g| g.get("lut"))
        .and_then(|l| l.as_str())
    {
        let p = std::path::Path::new(lut);
        let is_cube = p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cube"));
        if !is_cube {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("lut '{lut}' is not a .cube file"),
                "the adjustment grade reads a 3D LUT in the .cube format (ffmpeg lut3d)",
            )
            .with_suggested_action(
                "pass a path ending in .cube, or omit grade.lut for parametric-only grading",
            ));
        }
        if !p.is_file() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("lut file '{lut}' not found"),
                "the LUT must exist on disk when the adjustment is applied",
            )
            .with_suggested_action("pass an absolute path to an existing .cube LUT"));
        }
    }
    commit_core(state, "edit.adjustment", args, actor).await
}

/// edit.matte{clip, mode?=remove, model?=rvm, bg?, quality?=good, enabled?=true}
/// — AI background removal/replacement (a straight-alpha subject cutout without a
/// green screen). Stores the matte INTENT on the clip; core refuses a `remove`
/// matte on a base-track clip (nothing under the canvas to reveal). The typed
/// enums validate mode/model/quality/bg up front. The alpha itself is baked by
/// the matting sidecar — that lives in the render bake step, keyed to the
/// asset content; this verb is replay-safe (core only records the intent).
pub(in crate::dispatch) async fn edit_matte(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        mode: Option<cut_core::MatteMode>,
        model: Option<cut_core::MatteModel>,
        bg: Option<cut_core::MatteBg>,
        quality: Option<cut_core::MatteQuality>,
        seed: Option<cut_core::MatteSeed>,
        enabled: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?; // arg-shape + enum validation up front
    let enabling = a.enabled != Some(false);

    // Snapshot the bake inputs (source path + content hash + project dir) WITHOUT
    // holding the write lock across the slow, network bake.
    let bake_input = if enabling {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (tid, idx) = store.project.find_clip(&a.clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("clip '{}' not found", a.clip),
                "pass a media clip id on a video track",
            )
        })?;
        let clip = &store.project.track(tid).expect("track exists").clips[idx];
        let cut_core::Clip::Media(mc) = clip else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("'{}' is not a media clip", a.clip),
                "matte applies to media clips",
            ));
        };
        let asset = store.project.assets.get(&mc.asset).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("asset {} for clip '{}' is missing", mc.asset, a.clip),
                "the clip's source asset is not in the project",
            )
        })?;
        let mut path = PathBuf::from(&asset.path);
        if path.is_relative() {
            path = store.dir.join(path);
        }
        let m = cut_core::ClipMatte {
            mode: a.mode.unwrap_or_default(),
            model: a.model.unwrap_or_default(),
            bg: a.bg.clone(),
            quality: a.quality.unwrap_or_default(),
            seed: a.seed.clone(),
        };
        Some((store.dir.clone(), path, asset.hash.clone(), m))
    } else {
        None
    };

    // Bake the alpha (content-addressed, cached) BEFORE recording the op, so a
    // sidecar failure fails the verb cleanly — never a matte op pointing at a
    // missing alpha. Runs with NO lock held.
    let receipt = match bake_input {
        Some((dir, path, hash, m)) => Some(crate::matte::ensure_baked(&dir, &path, &hash, &m)?),
        None => None,
    };

    // Commit the replay-safe core op; ride the bake receipt along in the result.
    let rationale = a.rationale.clone();
    let include_legacy_inverse = wants_legacy_inverse(&args);
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let op = guard_call("edit.matte", || {
        store.apply("edit.matte", args, actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let mut result = shape_core_result("edit.matte", &op, include_legacy_inverse);
    if let (Some(stats), Value::Object(map)) = (&receipt, &mut result) {
        map.insert(
            "matte".into(),
            serde_json::to_value(stats).unwrap_or(Value::Null),
        );
    }
    Ok(VerbResult::ok_with_ops(result, vec![op_id]))
}

/// edit.reverse{clip, enabled?=true} — play a media clip BACKWARD (video
/// `reverse` / audio `areverse`). The timeline duration is unchanged. ffmpeg
/// `reverse` BUFFERS THE WHOLE CLIP IN RAM (~W·H·1.5·fps bytes per second at the
/// project resolution), so this FENCES the clip size LIVE — refuse a clip whose
/// estimated reverse buffer would exceed the cap, with a clear "split it" hint,
/// rather than letting the render OOM the box. The fence is live-only (replay
/// never re-checks: a logged reverse was already validated), so core just stores
/// the flag and replay stays deterministic + filesystem-independent.
pub(in crate::dispatch) async fn edit_reverse(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses + applies the default
    struct Args {
        clip: String,
        enabled: Option<bool>,
    }
    let a: Args = parse_args(args.clone())?;
    // Only fence when ENABLING (clearing a reverse frees memory, never needs it).
    if a.enabled != Some(false) {
        // ~4 GiB: above this a single-clip reverse risks OOM on a 16-GB box once
        // ffmpeg's own working set + the OS are accounted for (about 40 seconds
        // at 1080p30 or 20 seconds at 4K30).
        const REVERSE_RAM_CAP: f64 = 4.0 * 1024.0 * 1024.0 * 1024.0;
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        if let Some((tr, idx)) = store.project.find_clip(&a.clip) {
            if let Some(clip @ cut_core::Clip::Media(_)) =
                store.project.track(tr).and_then(|t| t.clips.get(idx))
            {
                let st = &store.project.settings;
                let dur_s = clip.timeline_duration_ms() as f64 / 1000.0;
                let est = st.width as f64 * st.height as f64 * 1.5 * st.fps * dur_s;
                if est > REVERSE_RAM_CAP {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        format!(
                            "reversing '{}' would need ~{:.1} GB RAM (cap {:.0} GB)",
                            a.clip,
                            est / 1e9,
                            REVERSE_RAM_CAP / 1e9
                        ),
                        "ffmpeg `reverse` buffers the whole clip in memory; this clip \
                         is too long for a single in-memory reverse at the project resolution",
                    )
                    .with_suggested_action(
                        "split the clip (edit.split) and reverse the shorter pieces, \
                         or lower the project resolution",
                    ));
                }
            }
        }
    }
    commit_core(state, "edit.reverse", args, actor).await
}

/// edit.stabilize{clip, smoothing?=15, enabled?=true} — smooth camera shake on a
/// VIDEO clip. The renderer runs a `vidstabdetect` analysis PRE-PASS (caches a
/// per-clip `.trf` motion file under `<project>/stab/`) then `vidstabtransform` at
/// render time. CPU-only + the pre-pass → opts the timeline off the GPU fast-track;
/// applied on the exact render paths (render.final / render.frame{compose} /
/// render.range), while fast preview/scrub show it UNstabilized. A core op
/// (replay-safe — the op stores smoothing, the `.trf` regenerates from source).
pub(in crate::dispatch) async fn edit_stabilize(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core(state, "edit.stabilize", args, actor).await
}

/// edit.freeze{clip, at_ms?=0, enabled?=true} — HOLD one source frame (at `at_ms`
/// into the clip's visible range) for the clip's whole slot. Video-track media
/// clips only (core-validated); audio plays through; the timeline duration is
/// unchanged. A core op (replay-safe). No live fence needed — freeze is cheap
/// (a single decoded frame cloned by `tpad`).
pub(in crate::dispatch) async fn edit_freeze(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core(state, "edit.freeze", args, actor).await
}

/// The Ken Burns presets (edit.animate `preset`): a named from→to pan/zoom built
/// from `amount` (default 0.3 = a 1.3× move). Returns the two AnimState JSON
/// objects, or an error listing the valid presets for an unknown name.
fn resolve_anim_preset(preset: &str, amount: f64) -> Result<(Value, Value), CutError> {
    let z = 1.0 + amount.max(0.0);
    let st = |zoom: f64, x: f64, y: f64| json!({ "zoom": zoom, "x": x, "y": y });
    let (from, to) =
        match preset {
            "zoom_in" => (st(1.0, 0.5, 0.5), st(z, 0.5, 0.5)),
            "zoom_out" => (st(z, 0.5, 0.5), st(1.0, 0.5, 0.5)),
            // pans need zoom > 1 to have room to move the window across the frame.
            "pan_left" => (st(z, 0.7, 0.5), st(z, 0.3, 0.5)),
            "pan_right" => (st(z, 0.3, 0.5), st(z, 0.7, 0.5)),
            "pan_up" => (st(z, 0.5, 0.7), st(z, 0.5, 0.3)),
            "pan_down" => (st(z, 0.5, 0.3), st(z, 0.5, 0.7)),
            other => return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown animate preset '{other}'"),
                "preset must be one of: zoom_in, zoom_out, pan_left, pan_right, pan_up, pan_down",
            )
            .with_suggested_action(
                "use a preset, or pass explicit from/to {zoom,x,y} objects instead",
            )),
        };
    Ok((from, to))
}

/// edit.animate{clip, preset?|from?/to?, amount?, enabled?=true} — Ken Burns
/// pan/zoom. A `preset` (zoom_in/out, pan_left/right/up/down) is RESOLVED here to
/// explicit from/to coordinates so the OP LOG stores resolved values and replay
/// never depends on the preset table (live-only resolution, like edit.grade's LUT
/// fence). Without a preset, raw from/to {zoom,x,y} pass through. enabled:false
/// clears. Core clamps + identity→None. A core op (replay-safe).
pub(in crate::dispatch) async fn edit_animate(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; we re-read fields explicitly below
    struct Args {
        clip: String,
        preset: Option<String>,
        amount: Option<f64>,
        from: Option<Value>,
        to: Option<Value>,
        enabled: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let enabled = a.enabled.unwrap_or(true);
    let resolved = if !enabled {
        json!({ "clip": a.clip, "enabled": false, "rationale": a.rationale })
    } else if let Some(preset) = a.preset.as_deref() {
        let (from, to) = resolve_anim_preset(preset, a.amount.unwrap_or(0.3))?;
        json!({ "clip": a.clip, "from": from, "to": to, "enabled": true, "rationale": a.rationale })
    } else {
        // Raw from/to (or defaults = identity, which the core treats as a clear).
        json!({ "clip": a.clip, "from": a.from, "to": a.to, "enabled": true, "rationale": a.rationale })
    };
    commit_core(state, "edit.animate", resolved, actor).await
}

/// edit.keyframe{clip, param, points:[{t_ms,value}], interp?} — animate ONE
/// parameter over the clip via keyframes (opacity → overlay alpha, volume → audio
/// gain). SET semantics: points REPLACE that param's track; empty clears it. A
/// keyframed param OVERRIDES its static counterpart. Core validates param↔track
/// (opacity→video, volume→audio) + sorts/clamps the points. A core op (replay-safe).
pub(in crate::dispatch) async fn edit_keyframe(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core(state, "edit.keyframe", args, actor).await
}

// --- edit.auto_zoom — emphasis-driven punch-in zooms ------------------------

/// Default punch-in depth (12% push-in — the subtle short-form look).
const AUTO_ZOOM_DEFAULT_INTENSITY: f64 = 0.12;
/// Hard ceiling on the push-in (a 50% zoom is already extreme; keep it subtle).
const AUTO_ZOOM_MAX_INTENSITY: f64 = 0.5;
/// Default hold at full zoom, ms (the beat lands, then settles back).
const AUTO_ZOOM_DEFAULT_HOLD_MS: u64 = 500;
/// Ease-in (rise) length of a punch, ms — fixed (the look, not a knob).
const AUTO_ZOOM_RISE_MS: u64 = 280;
/// Ease-back (fall) length of a punch, ms — slightly slower than the rise.
const AUTO_ZOOM_FALL_MS: u64 = 420;
/// Minimum spacing between punches, ms — so zooms never bunch on adjacent beats.
const AUTO_ZOOM_MIN_SPACING_MS: u64 = 4000;
/// Relative peak threshold (0..1): a peak must clear `min + frac*(max-min)` of the
/// envelope's dynamic range. Permissive on purpose — selectivity comes from the
/// strict-local-maximum + min-spacing + max_zooms cap; this only kills a flat clip.
const AUTO_ZOOM_THRESHOLD_FRAC: f64 = 0.5;
/// Default zoom density: about one punch per this many ms of clip (capped below).
const AUTO_ZOOM_MS_PER_ZOOM: u64 = 8000;
/// Hard cap on the auto-derived `max_zooms` (a sane density ceiling).
const AUTO_ZOOM_MAX_ZOOMS_CAP: usize = 24;
/// Utterance-segmentation pause for the transcript trigger, ms (a gap longer than
/// this between words starts a new utterance — a fresh emphasis beat).
const AUTO_ZOOM_PAUSE_MS: u64 = 400;

/// edit.auto_zoom{clip, intensity?=0.12, max_zooms?, hold_ms?=500, trigger?="energy",
/// rationale?} — add subtle PUNCH-IN zooms at the emphasis moments of a clip (the
/// dynamic short-form / talking-head look: the frame pushes in on the loud beats).
///
/// EMPHASIS DETECTION (live, from the clip's perception report):
/// - `trigger:"energy"` (default) — local maxima of the per-window momentary
///   loudness envelope (`Loudness.windows`, the same perception audio facts
///   `edit.duck` consumes), mapped to clip-local time, NON-MAXIMUM-suppressed by
///   strength + `min_spacing`, capped at `max_zooms` (default ≈ one per 8 s).
/// - `trigger:"transcript"` — utterance/sentence START times (the transcript word
///   model, segmented at `.?!` or a pause gap, like transcript.remove_retakes),
///   forward-greedily min-spaced and capped.
///
/// REPRESENTATION (replay-safe): each emphasis point lowers to a `scale` keyframe
/// RAMP `1.0 → 1.0+intensity → 1.0` (rise / hold / fall), COMMITTED through the
/// normal `edit.keyframe {param:"scale"}` path — the existing centred-`zoompan`
/// render path (the multi-point generalization of `edit.animate`). The committed op
/// carries CONCRETE keyframes, so replay needs no perception. SET semantics on the
/// scale track: re-running OVERWRITES a previous auto_zoom. A manual `edit.animate`
/// Ken Burns on the clip must be cleared first (the core enforces they're mutually
/// exclusive — the keyframe channel is the richer form). v1 zooms are CENTRED
/// (subject-tracked focal points are a record-integration follow-up).
///
/// Honest receipt `{clip, zooms:[{at_ms, peak, scale}], intensity, trigger, count,
/// op_ids}` — `peak` is the trigger metric (momentary LUFS for energy, the
/// preceding pause-gap ms for transcript). No loudness/words → actionable error;
/// intensity 0 or no emphasis peaks → a clean `count:0` no-op (no op appended).
pub(in crate::dispatch) async fn edit_auto_zoom(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        intensity: Option<f64>,
        max_zooms: Option<usize>,
        hold_ms: Option<u64>,
        trigger: Option<String>,
        #[allow(dead_code)] // recorded on the lowered op by commit_core
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let intensity = a
        .intensity
        .unwrap_or(AUTO_ZOOM_DEFAULT_INTENSITY)
        .clamp(0.0, AUTO_ZOOM_MAX_INTENSITY);
    let hold_ms = a.hold_ms.unwrap_or(AUTO_ZOOM_DEFAULT_HOLD_MS);
    let trigger = a.trigger.as_deref().unwrap_or("energy");
    if !matches!(trigger, "energy" | "transcript") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown trigger '{trigger}'"),
            "trigger must be \"energy\" (loudness peaks) or \"transcript\" (sentence starts)",
        )
        .with_clip(&a.clip));
    }

    // Resolve the clip's source facts + load its perception report (read lock; no
    // slow work — load_report is a small JSON read).
    struct ClipFacts {
        asset: String,
        src_in: u64,
        src_out: u64,
        speed: f64,
        clip_dur: u64,
    }
    let (facts, report) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (tid, idx) = store.project.find_clip(&a.clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("clip '{}' not found", a.clip),
                "pass a media clip id that is on the timeline",
            )
            .with_clip(&a.clip)
            .with_suggested_action("check project.state for the clip ids")
        })?;
        let clip = &store.project.track(tid).expect("track exists").clips[idx];
        let cut_core::Clip::Media(mc) = clip else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("'{}' is not a media clip", a.clip),
                "auto_zoom animates a video clip's scale — gaps / caption clips have no picture",
            )
            .with_clip(&a.clip));
        };
        let speed = if mc.speed > 0.0 { mc.speed } else { 1.0 };
        let facts = ClipFacts {
            asset: mc.asset.clone(),
            src_in: mc.src_in_ms,
            src_out: mc.src_out_ms,
            speed,
            clip_dur: clip.timeline_duration_ms(),
        };
        let report =
            cut_perception::load_report(&store.receipts_dir(), &facts.asset)?.ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!(
                        "asset '{}' for clip '{}' has no perception report",
                        facts.asset, a.clip
                    ),
                    "auto_zoom reads emphasis from the perception audio analysis",
                )
                .with_clip(&a.clip)
                .with_suggested_action(
                    "run media.perception{asset} (or wait for the import chain), then retry",
                )
            })?;
        (facts, report)
    };

    // Default zoom density from the clip length (≈ one per 8 s), capped.
    let max_zooms = a
        .max_zooms
        .unwrap_or_else(|| ((facts.clip_dur / AUTO_ZOOM_MS_PER_ZOOM).max(1)) as usize)
        .min(AUTO_ZOOM_MAX_ZOOMS_CAP);

    // Map an asset-SOURCE time (ms) into this clip's local timeline (ms), or None
    // if it falls outside the clip's [src_in, src_out] window (identity at speed 1).
    let map_local = |src_ms: u64| -> Option<u64> {
        if src_ms < facts.src_in || src_ms > facts.src_out {
            return None;
        }
        Some(cut_core::src_off_to_tl(src_ms - facts.src_in, facts.speed))
    };

    // Build the emphasis set: (clip-local t_ms, trigger metric).
    let emphasis: Vec<(u64, f64)> = match trigger {
        "energy" => {
            let loud = report.loudness.as_ref().filter(|l| !l.windows.is_empty()).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    "the perception report has no loudness windows (no audio energy to read)",
                    "auto_zoom energy peaks come from the momentary-loudness envelope",
                )
                .with_clip(&a.clip)
                .with_suggested_action(
                    "use trigger:\"transcript\" (a clip with speech), or re-run media.perception on a clip with audio",
                )
            })?;
            let mut env: Vec<(u64, f64)> = loud
                .windows
                .iter()
                .filter_map(|w| map_local(w.at_ms).map(|t| (t, w.momentary_lufs)))
                .collect();
            env.sort_by_key(|&(t, _)| t);
            cut_core::auto_zoom::pick_energy_peaks(
                &env,
                AUTO_ZOOM_MIN_SPACING_MS,
                max_zooms,
                AUTO_ZOOM_THRESHOLD_FRAC,
            )
        }
        _ => {
            // transcript: utterance starts within the clip's source window.
            let words: Vec<cut_perception::WordSpan> = report
                .words
                .as_ref()
                .map(|t| {
                    t.words
                        .iter()
                        .filter(|w| w.start_ms >= facts.src_in && w.start_ms <= facts.src_out)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            if words.is_empty() {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    "no transcript words in the clip's range",
                    "auto_zoom's transcript trigger reads sentence/utterance starts",
                )
                .with_clip(&a.clip)
                .with_suggested_action("run media.transcribe first, or use trigger:\"energy\""));
            }
            // (start clip-local t_ms, preceding pause-gap ms) per utterance.
            let mut starts: Vec<(u64, f64)> =
                speech_text::retake_utterances(&words, AUTO_ZOOM_PAUSE_MS)
                    .into_iter()
                    .filter_map(|r| {
                        let w = &words[r[0]];
                        map_local(w.start_ms).map(|t| {
                            let prev_end = if r[0] == 0 { 0 } else { words[r[0] - 1].end_ms };
                            (t, w.start_ms.saturating_sub(prev_end) as f64)
                        })
                    })
                    .collect();
            starts.sort_by_key(|&(t, _)| t);
            let times: Vec<u64> = starts.iter().map(|&(t, _)| t).collect();
            let kept: std::collections::HashSet<u64> =
                cut_core::auto_zoom::space_times(&times, AUTO_ZOOM_MIN_SPACING_MS, max_zooms)
                    .into_iter()
                    .collect();
            starts
                .into_iter()
                .filter(|&(t, _)| kept.contains(&t))
                .collect()
        }
    };

    let times: Vec<u64> = emphasis.iter().map(|&(t, _)| t).collect();
    let points = cut_core::auto_zoom::build_zoom_points(
        &times,
        facts.clip_dur,
        intensity,
        AUTO_ZOOM_RISE_MS,
        hold_ms,
        AUTO_ZOOM_FALL_MS,
    );

    let scale = 1.0 + intensity;
    let zooms: Vec<Value> = emphasis
        .iter()
        .map(|&(t, p)| json!({ "at_ms": t, "peak": p, "scale": scale }))
        .collect();

    // intensity 0 OR no emphasis peaks → a clean no-op (no op appended).
    if points.is_empty() {
        let note = if intensity <= 0.0 {
            "intensity 0 — no zoom applied".to_string()
        } else {
            format!(
                "no emphasis peaks found ({trigger}) — the clip's audio has no clear loud beats to punch on"
            )
        };
        return Ok(VerbResult::ok(json!({
            "clip": a.clip,
            "zooms": [],
            "intensity": intensity,
            "trigger": trigger,
            "count": 0,
            "note": note,
        })));
    }

    // Commit the punches as concrete scale keyframes via the normal edit.keyframe
    // path (replay-safe; eased so the punch reads professional). SET semantics →
    // a re-run overwrites a previous auto_zoom; a manual edit.animate is rejected
    // by the core (mutually exclusive) with an actionable message.
    let rationale = a.rationale.clone().unwrap_or_else(|| {
        format!(
            "edit.auto_zoom: {} punch-in zoom(s) at {trigger} emphasis, intensity {intensity:.2}",
            times.len()
        )
    });
    let kf_args = json!({
        "clip": a.clip,
        "param": "scale",
        "points": points,
        "interp": "ease_in_out_quad",
        "rationale": rationale,
    });
    let committed = commit_core(state, "edit.keyframe", kf_args, actor).await?;
    let op_ids = committed.op_ids.unwrap_or_default();

    Ok(VerbResult::ok_with_ops(
        json!({
            "clip": a.clip,
            "zooms": zooms,
            "intensity": intensity,
            "trigger": trigger,
            "count": times.len(),
            "op_ids": op_ids.clone(),
        }),
        op_ids,
    ))
}

/// edit.add_mask{clip, shape, points, feather?, invert?, effect?, strength?, enabled?}
/// — a vector/freeform MASK scoping a region effect (blur/pixelate/black) to a shape
/// on a BASE-track video clip (the region-blur / privacy-redaction primitive).
/// enabled:false CLEARS it. Core validates track + point counts. A core op (replay-safe).
pub(in crate::dispatch) async fn edit_add_mask(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    commit_core(state, "edit.add_mask", args, actor).await
}

/// edit.redact{clip, shape, points, mode? = blur, feather?, invert?, strength?,
/// range_ms?, enabled?, rationale?} — REDACTION. A TIME-BOUNDED region effect
/// for privacy: blur/pixelate/box a shape on a BASE-track video clip, active ONLY in
/// `range_ms` (clip-local; default whole clip). Shares the mask field + render path
/// with edit.add_mask but is security-framed: `mode` (blur|pixelate|box) + an
/// OVER-BLUR fail-safe default (never under-redact). enabled:false CLEARS. A core op
/// (replay-safe) — the resolved mask + range are stored, so replay needs no cv2/OCR.
pub(in crate::dispatch) async fn edit_redact(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    // Multi-region faces: detect every face at `at_ms` and blur them all at once.
    // (multi-region). Like ocr_auto, detection runs ONCE here; we commit a NORMAL
    // multi-region edit.redact with the resolved rects, so the op log stays
    // detector-free and replay is deterministic. The shared `mode`/`strength`/`feather`/
    // `range_ms` from `args` carry through to every face.
    if args.get("faces").and_then(|v| v.as_bool()).unwrap_or(false) {
        #[derive(serde::Deserialize)]
        struct FaceArgs {
            clip: String,
            at_ms: Option<u64>,
            track_faces: Option<bool>,
        }
        let fa: FaceArgs = parse_args(args.clone())?;
        let at_ms = fa.at_ms.unwrap_or(0);
        if args.get("track").is_some_and(|v| !v.is_array()) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "edit.redact track must be a motion-track array",
                "use track_faces:false to disable face tracking; `track` is reserved for [{t_ms,cx,cy}] motion points",
            ));
        }
        let (project, _edl, _dir, _at) = snapshot(state).await?;
        let (track_id, idx) = project
            .find_clip(&fa.clip)
            .map(|(t, i)| (t.to_string(), i))
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no clip '{}'", fa.clip),
                    "clip ids come from project.state",
                )
                .with_clip(&fa.clip)
            })?;
        let cut_core::Clip::Media(c) = &project.track(&track_id).unwrap().clips[idx] else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("'{}' is not a media clip", fa.clip),
                "faces reads the clip's source pixels",
            )
            .with_clip(&fa.clip));
        };
        let speed = if c.speed > 0.0 { c.speed } else { 1.0 };
        let src_in = c.src_in_ms;
        let src_at = src_in + (at_ms as f64 * speed).round() as u64;
        let (src_path, _hash) = asset_info(state, &c.asset.clone()).await?;
        // Track each face by default (privacy: a moving face MUST stay covered);
        // `track_faces:false` opts into the faster static-detect pass.
        let want_track = fa.track_faces.unwrap_or(true);
        let rt = crate::faces::runtime().ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "faces needs the perception sidecar (opencv), which is not installed",
                "run system.doctor; install the perception sidecar in its venv",
            )
            .with_suggested_action("system.doctor → install the perception sidecar")
        })?;
        let res = run_blocking("edit.redact faces", move || {
            crate::faces::run_faces(&rt, &src_path, src_at, want_track)
        })
        .await?;
        if res.boxes.is_empty() {
            // No faces at this frame → NO op (honest non-mutating receipt).
            return Ok(VerbResult::ok(json!({
                "clip": fa.clip,
                "faces": true,
                "at_ms": at_ms,
                "found": 0,
                "note": "no faces detected at this frame — nothing redacted",
            })));
        }
        // Each face → a region: a rect (size from the seed box) + its CSRT track
        // so a moving face follows. Track t_ms is source ms (from src_at) ->
        // convert to CLIP-LOCAL: (src_t − src_in) / speed, dropping pre-clip points.
        let region_json = |b: &crate::faces::FaceBox| -> Value {
            let rect = crate::faces::box_to_rect(b);
            let mut o = serde_json::Map::new();
            o.insert("shape".into(), json!("rect"));
            o.insert("points".into(), json!(rect));
            if let Some(tk) = &b.track {
                let pts: Vec<Value> = tk
                    .iter()
                    .filter_map(|p| {
                        let clip_t = ((p.t_ms as f64 - src_in as f64) / speed).round();
                        (clip_t >= 0.0)
                            .then(|| json!({"t_ms": clip_t as u64, "cx": p.cx, "cy": p.cy}))
                    })
                    .collect();
                if pts.len() >= 2 {
                    o.insert("track".into(), json!(pts));
                }
            }
            Value::Object(o)
        };
        let regions: Vec<Value> = res.boxes.iter().map(region_json).collect();
        let primary = regions[0].as_object().unwrap().clone();
        let mut new_args = args.clone();
        if let Some(obj) = new_args.as_object_mut() {
            obj.remove("faces");
            obj.remove("at_ms");
            obj.remove("track_faces");
            obj.insert("shape".into(), json!("rect"));
            obj.insert("points".into(), primary["points"].clone());
            if let Some(t) = primary.get("track") {
                obj.insert("track".into(), t.clone());
            }
            obj.insert("boxes".into(), json!(regions[1..]));
        }
        // Lowest detector confidence among the faces — a fail-safe signal (a
        // low-confidence face is still blurred, but the agent/UI can see it).
        let min_conf = res
            .boxes
            .iter()
            .map(|b| b.conf)
            .fold(f64::INFINITY, f64::min);
        let mut vr = commit_core(state, "edit.redact", new_args, actor).await?;
        if let Some(r) = vr.result.as_mut() {
            r["faces"] = json!({
                "found": res.boxes.len(),
                "min_conf": (min_conf * 1000.0).round() / 1000.0,
            });
        }
        return Ok(vr);
    }
    // ocr_auto: OCR the frame at at_ms, find PII, and redact its UNION region.
    // The OCR + matching run ONCE here; we then commit a NORMAL edit.redact with the
    // RESOLVED rect — so the op log stays OCR-free and replay is deterministic.
    if !args
        .get("ocr_auto")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return commit_core(state, "edit.redact", args, actor).await;
    }
    #[derive(serde::Deserialize)]
    struct OcrArgs {
        clip: String,
        at_ms: Option<u64>,
        pii: Option<Vec<String>>,
    }
    let oa: OcrArgs = parse_args(args.clone())?;
    let at_ms = oa.at_ms.unwrap_or(0);

    // Resolve the clip → source path + clip-local at_ms → SOURCE ms (like edit.track).
    let (project, _edl, _dir, _at) = snapshot(state).await?;
    let (track_id, idx) = project
        .find_clip(&oa.clip)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no clip '{}'", oa.clip),
                "clip ids come from project.state",
            )
            .with_clip(&oa.clip)
        })?;
    let cut_core::Clip::Media(c) = &project.track(&track_id).unwrap().clips[idx] else {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("'{}' is not a media clip", oa.clip),
            "ocr_auto reads the clip's source pixels",
        )
        .with_clip(&oa.clip));
    };
    let speed = if c.speed > 0.0 { c.speed } else { 1.0 };
    let src_at = c.src_in_ms + (at_ms as f64 * speed).round() as u64;
    let (src_path, _hash) = asset_info(state, &c.asset.clone()).await?;

    let rt = crate::ocr::runtime().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "ocr_auto needs the perception sidecar + rapidocr-onnxruntime, which is not installed",
            "run system.doctor; install the perception sidecar and rapidocr in its venv",
        )
        .with_suggested_action("system.doctor → install the perception sidecar + rapidocr")
    })?;
    let res = run_blocking("edit.redact ocr", move || {
        crate::ocr::run_ocr(&rt, &src_path, src_at)
    })
    .await?;

    // Fail-safe 1.5%-of-frame margin around the union of all PII boxes.
    let Some(region) = crate::ocr::pii_region(&res, oa.pii.as_deref(), 0.015) else {
        // Nothing sensitive at this frame → NO op (honest non-mutating receipt).
        return Ok(VerbResult::ok(json!({
            "clip": oa.clip,
            "ocr_auto": true,
            "at_ms": at_ms,
            "found": 0,
            "note": "no PII detected at this frame — nothing redacted",
        })));
    };
    let (x0, y0) = (region.cx - region.w / 2.0, region.cy - region.h / 2.0);
    let (x1, y1) = (region.cx + region.w / 2.0, region.cy + region.h / 2.0);

    // Inject the resolved rect; drop the ocr-only keys so the committed op is a
    // plain, schema-valid edit.redact (replay never depends on OCR).
    let mut new_args = args.clone();
    if let Some(obj) = new_args.as_object_mut() {
        obj.remove("ocr_auto");
        obj.remove("at_ms");
        obj.remove("pii");
        obj.insert("shape".into(), json!("rect"));
        obj.insert("points".into(), json!([[x0, y0], [x1, y1]]));
    }
    let mut vr = commit_core(state, "edit.redact", new_args, actor).await?;
    // Attach the OCR summary — CATEGORIES + box only, NEVER the matched text.
    if let Some(r) = vr.result.as_mut() {
        r["ocr"] = json!({
            "found": region.matched,
            "categories": region.categories,
            "box": [x0, y0, x1, y1],
            "min_conf": (region.min_conf * 1000.0).round() / 1000.0,
            "at_ms": at_ms,
        });
    }
    Ok(vr)
}
