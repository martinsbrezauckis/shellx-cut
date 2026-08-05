//! Title, shape, and slide overlay edit handlers.

use super::*;

/// Resolve an `edit.slide` request into a single position-keyframe track (pos_x or
/// pos_y) — the "easy path" over hand-writing position keyframes. `edge` picks the
/// axis + which side the overlay enters from / exits to; the OFF-screen position is
/// `-scale` (left/top) or `1.0` (right/bottom) in frame fractions (the PiP's own
/// size fraction = its transform `scale`), and the RESTING position is the clip's
/// static transform x/y. `mode:"in"` slides from off-screen to rest over the first
/// `slide_ms`; `mode:"out"` slides from rest to off-screen over the LAST `slide_ms`
/// of the clip's `dur_ms`. Returns (param_name, points-as-json). Pure → unit-tested.
pub(in crate::dispatch) fn resolve_slide(
    t: &cut_core::ClipTransform,
    dur_ms: u64,
    edge: &str,
    mode: &str,
    slide_ms: u64,
) -> Result<(&'static str, Vec<Value>), CutError> {
    let (param, rest, off) = match edge {
        "left" => ("pos_x", t.x, -t.scale),
        "right" => ("pos_x", t.x, 1.0),
        "top" => ("pos_y", t.y, -t.scale),
        "bottom" => ("pos_y", t.y, 1.0),
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown slide edge '{other}'"),
                "edge must be one of: left, right, top, bottom",
            ))
        }
    };
    // Clamp the slide span to the clip length (a 500ms slide on a 300ms clip = 300ms).
    let span = slide_ms.clamp(1, dur_ms.max(1));
    let pt = |t_ms: u64, value: f64| json!({ "t_ms": t_ms, "value": value });
    let points = match mode {
        "in" => vec![pt(0, off), pt(span, rest)],
        "out" => vec![pt(dur_ms.saturating_sub(span), rest), pt(dur_ms, off)],
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown slide mode '{other}'"),
                "mode must be 'in' (off-screen → rest) or 'out' (rest → off-screen)",
            ))
        }
    };
    Ok((param, points))
}

/// edit.slide{clip, edge:"left"|"right"|"top"|"bottom", mode?="in", slide_ms?=500} —
/// animated-PiP convenience: slide an overlay IN from / OUT to a screen edge. Reads
/// the clip's resting transform + timeline length, resolves to position keyframes
/// (resolve_slide), and LOWERS to an `edit.keyframe` op (param=pos_x/pos_y) — so the
/// op log records the resolved keyframes and replay never depends on this verb. Pair
/// with a scaled `edit.transform` for the PiP size; an un-transformed (full-frame)
/// overlay slides the whole frame.
pub(in crate::dispatch) async fn edit_slide(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        edge: String,
        mode: Option<String>,
        slide_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    // Read the clip's resting transform + timeline duration (released before commit).
    let (transform, dur_ms) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (tid, idx) = store.project.find_clip(&a.clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("clip '{}' not found", a.clip),
                "pass a media clip id on an overlay video track",
            )
        })?;
        let clip = &store.project.track(tid).expect("track exists").clips[idx];
        let cut_core::Clip::Media(m) = clip else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("'{}' is not a media clip", a.clip),
                "slide applies to overlay media clips",
            ));
        };
        (
            m.transform
                .clone()
                .unwrap_or_else(cut_core::ClipTransform::identity),
            clip.timeline_duration_ms(),
        )
    };
    let mode = a.mode.as_deref().unwrap_or("in");
    let (param, points) =
        resolve_slide(&transform, dur_ms, &a.edge, mode, a.slide_ms.unwrap_or(500))?;
    commit_core(
        state,
        "edit.keyframe",
        json!({ "clip": a.clip, "param": param, "points": points, "interp": "linear", "rationale": a.rationale }),
        actor,
    )
    .await
}

/// Render a [`TitleSpec`] to a TRANSPARENT overlay `.mov` (content-addressed in
/// the project's `titles/` dir) and register it as a generated MEDIA asset via a
/// non-timeline `media.import` op. Returns the new asset id.
///
/// Shared by [`place_title_overlay`] (the title.add / captions.kinetic PLACEMENT
/// path) and `title_update` (the re-render path for editing a placed title's
/// text renders a fresh overlay and swaps the existing clip onto it in place).
/// The encode runs on a blocking thread; the import op persists + replays (the
/// `.mov` is re-read from disk on replay like any imported media, so the title
/// is never re-encoded outside the live edit).
async fn render_title_asset(
    state: &AppState,
    spec: &cut_media::title::TitleSpec,
    duration_ms: u64,
) -> Result<String, CutError> {
    let (w, h, dir) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let s = &store.project.settings;
        (s.width, s.height, store.dir.clone())
    };
    let titles_dir = dir.join("titles");
    std::fs::create_dir_all(&titles_dir)
        .map_err(|e| CutError::new(error_codes::IO, "create titles dir", e.to_string()))?;
    // A process-unique pending name (the final file is content-addressed by hash,
    // so this only needs to avoid a collision between two in-flight renders).
    let tag = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_mov = titles_dir.join(format!("title_pending_{tag}.mov"));
    {
        let spec_cl = spec.clone();
        let out_cl = tmp_mov.clone();
        tokio::task::spawn_blocking(move || {
            cut_media::render::encode_title_overlay(&spec_cl, &out_cl)
        })
        .await
        .map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                "title encode task panicked",
                e.to_string(),
            )
        })??;
    }
    let hash = cut_core::store::hash_file(&tmp_mov)?;
    // Filename fragment = the hex AFTER the algorithm tag — works for either
    // "sha256:" (full) or "sha256s:" (sampled, big-file) and never lets a colon
    // into the path (a title .mov is tiny, so it is always the full branch, but
    // this stays correct if that ever changes).
    let short = hash.rsplit(':').next().unwrap_or(&hash);
    let mov_path = titles_dir.join(format!("title_{}.mov", &short[..short.len().min(16)]));
    if mov_path != tmp_mov {
        std::fs::rename(&tmp_mov, &mov_path)
            .map_err(|e| CutError::new(error_codes::IO, "finalize title file", e.to_string()))?;
    }
    // Register the generated .mov as an asset — a NON-timeline media.import op
    // (so it persists + replays; it stays registered across the title's undo as
    // a harmless unreferenced asset, never an orphan TRACK).
    let asset = cut_core::Asset {
        path: mov_path.to_string_lossy().into_owned(),
        hash,
        probe: Some(
            json!({"kind":"video","width":w,"height":h,"has_audio":false,"has_video":true,"duration_ms":duration_ms}),
        ),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (id, op) =
        store.record_import(None, asset, Actor::system(), Some("generated title".into()))?;
    state.events.publish(Event::OpApplied { op });
    Ok(id)
}

/// Shared by title.add + captions.kinetic: render a TitleSpec to a TRANSPARENT
/// overlay .mov, register it as a generated asset, and insert it on the
/// top-most `title1` overlay video track at `at_ms`. Lowers to honest import +
/// insert ops; the .mov is content-addressed in the project's titles/ dir.
/// Returns (asset_id, clip_id).
pub(super) async fn place_title_overlay(
    state: &AppState,
    spec: &cut_media::title::TitleSpec,
    at_ms: u64,
    duration_ms: u64,
    actor: Actor,
    rationale: String,
    // The LOGICAL verb this placement belongs to ("title.add" | "captions.kinetic")
    // + its args — so the timeline change commits as ONE op under that name,
    // instead of separate edit.add_track + edit.insert implementation ops (undo
    // of a title is then a single action and Review shows the logical verb).
    logical_verb: &str,
    op_args: Value,
) -> Result<(String, String, Value, String), CutError> {
    // Pick a TITLE overlay track with room at [at_ms, at_ms+duration); else
    // allocate the next titleN. Overlapping titles (e.g. an intro card during
    // kinetic captions) thus STACK on separate tracks and both render, instead
    // of colliding on one shared track.
    let (track_id, need_create) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let edl = cut_core::edl_from_project(&store.project);
        let end = at_ms + duration_ms;
        let title_tracks: Vec<String> = store
            .project
            .tracks
            .iter()
            .filter(|t| t.kind == cut_core::TrackKind::Video && t.id.starts_with("title"))
            .map(|t| t.id.clone())
            .collect();
        let free = title_tracks.iter().find(|tid| {
            !edl.track_segments(tid).any(|seg| {
                seg.asset.is_some() && seg.timeline_in_ms < end && seg.timeline_out_ms > at_ms
            })
        });
        match free {
            Some(t) => (t.clone(), false),
            None => {
                let max_n = title_tracks
                    .iter()
                    .filter_map(|t| t.strip_prefix("title").and_then(|x| x.parse::<u32>().ok()))
                    .max()
                    .unwrap_or(0);
                (format!("title{}", max_n + 1), true)
            }
        }
    };
    // Render the overlay .mov + register it as a generated asset (shared with
    // title.update's re-render path).
    let asset_id = render_title_asset(state, spec, duration_ms).await?;

    // Commit the TIMELINE change (optional new title track + the overlay insert)
    // as ONE lowered op named after the logical verb. apply_lowered runs the
    // steps (real edit verbs) on a clone and records them under `lowered` for
    // replay; only the insert allocates a clip id (the track id is explicit), so
    // replay is id-stable. Undo (tip restore) reverses the whole title at once.
    let mut steps = Vec::new();
    if need_create {
        steps.push(InverseOp {
            verb: "edit.add_track".into(),
            args: json!({"kind":"video","id": track_id}),
        });
    }
    steps.push(InverseOp {
        verb: "edit.insert".into(),
        args: json!({
            "asset": asset_id, "track": track_id, "at_ms": at_ms,
            "src_range_ms": [0, duration_ms], "ripple": false,
        }),
    });
    let op = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        guard_call(logical_verb, || {
            store.apply_lowered(logical_verb, op_args, actor, Some(rationale), steps, vec![])
        })?
    };
    state.events.publish(Event::OpApplied { op: op.clone() });
    let clip_id = op
        .effects
        .iter()
        .find_map(|e| e.detail.get("added_clip"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok((track_id, asset_id, clip_id, op.op_id))
}

/// title.add — native motion-graphics title. Builds a TitleSpec from a
/// preset (lower_third | title_card), renders it to a TRANSPARENT overlay .mov
/// (cut_media::render::encode_title_overlay — resvg per-frame), registers it as
/// a generated asset, and inserts it on a dedicated top-most `title1` overlay
/// video track. Lowers to HONEST import + insert ops; the .mov lives in the
/// project's titles/ dir and is re-read on replay like any imported media.
/// (Per-word KINETIC captions build a richer spec from the transcript — a
/// follow-up; this is the lower-third / title-card surface.)
/// Reject an absurd overlay span before it reaches the per-frame title renderer.
/// `encode_title_overlay` writes one full-canvas RGBA frame (W·H·4 bytes) per
/// frame to a temp file, so an unbounded `range_ms` (e.g. `[0, u64::MAX]`) would
/// try to render billions of frames → fill the disk and never return. A
/// title/shape overlay is a short animated graphic; 10 minutes is far past any
/// real use, and the cap is duration-only (fps-independent) so the check is cheap
/// and clear. The bound applies to both title and shape overlays.
const MAX_OVERLAY_MS: u64 = 10 * 60 * 1000;
fn reject_overlay_too_long(range_ms: [u64; 2]) -> Result<(), CutError> {
    let dur = range_ms[1].saturating_sub(range_ms[0]);
    if dur > MAX_OVERLAY_MS {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "overlay span is too long",
            format!(
                "an animated title/shape is rendered frame-by-frame; the span is {dur} ms, the max is {MAX_OVERLAY_MS} ms (10 min)"
            ),
        ));
    }
    Ok(())
}

fn normalized_unit_arg(label: &str, value: f64) -> Result<f64, CutError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} must be in normalized range [0,1]"),
            format!("got {value:?}"),
        ))
    }
}

fn positive_normalized_unit_arg(label: &str, value: f64) -> Result<f64, CutError> {
    if value.is_finite() && value > 0.0 && value <= 1.0 {
        Ok(value)
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} must be in normalized range (0,1]"),
            format!("got {value:?}"),
        ))
    }
}

fn validate_normalized_box_axis(
    pos_label: &str,
    size_label: &str,
    pos: f64,
    size: f64,
) -> Result<(), CutError> {
    let edge_label = format!("{pos_label} + {size_label}");
    let edge = pos + size;
    if edge.is_finite() && edge <= 1.0 {
        Ok(())
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{edge_label} must stay within the normalized canvas"),
            format!("{pos_label}={pos:?}, {size_label}={size:?}, {edge_label}={edge:?}"),
        ))
    }
}

/// title.add / title.update argument shape — the declarative source of a
/// [`cut_media::title::TitleSpec`]. Lifted to module scope (was inline in
/// `title_add`) so `title_update` can deserialize a recovered title.add op's
/// args into the SAME struct and rebuild the spec through the SAME
/// [`build_title_spec`] path — guaranteeing an edited title renders identically
/// to the original except for the changed field(s). `text` + `range_ms` are the
/// only required fields; everything else carries the preset/free/template
/// defaults. (title-editing regression.)
#[derive(serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct TitleArgs {
    pub(crate) text: String,
    pub(crate) range_ms: [u64; 2],
    pub(crate) preset: Option<String>,
    pub(crate) font_px: Option<f64>,
    pub(crate) color: Option<String>,
    pub(crate) bg: Option<bool>,
    /// Free placement: normalized horizontal anchor [0,1]. When BOTH x and y
    /// are given the title is placed there (overrides the preset position) —
    /// the "drop the title anywhere" path.
    pub(crate) x: Option<f64>,
    /// Free placement: normalized vertical anchor [0,1].
    pub(crate) y: Option<f64>,
    /// Free-placement text alignment at (x,y): "left" | "center" | "right".
    pub(crate) align: Option<String>,
    /// Entry/exit ANIMATION override (fade|slide_up|slide_down|slide_left|
    /// slide_right|pop|none). Omit to keep the preset's tasteful default motion.
    pub(crate) animation: Option<String>,
    /// Animated-text TEMPLATE (typewriter|word_pop|slide_stack|
    /// kinetic_emphasis|lower_third_reveal|caption_karaoke). When set it
    /// takes precedence over preset/free placement and OWNS the motion (the
    /// `animation` override is ignored). The catalog is `title.templates`.
    pub(crate) template: Option<String>,
    /// Accent color #RRGGBB for templates that highlight a word
    /// (kinetic_emphasis, caption_karaoke). Default a warm gold.
    pub(crate) accent: Option<String>,
    /// kinetic_emphasis: the word to emphasize (case-insensitive substring);
    /// omit to emphasize the longest word.
    pub(crate) emphasis: Option<String>,
    pub(crate) rationale: Option<String>,
}

/// Build a [`cut_media::title::TitleSpec`] from declarative [`TitleArgs`] for the
/// given canvas geometry/fps — the pure spec-construction core shared by
/// `title_add` (PLACE a new title) and `title_update` (RE-RENDER an edited
/// title's overlay). Resolves the preset/free/template defaults, validates
/// template + colors, and applies the optional animation override. Assumes the
/// range has already been validated (end > start, not absurdly long).
pub(crate) fn build_title_spec(
    a: &TitleArgs,
    w: u32,
    h: u32,
    fps: f64,
) -> Result<cut_media::title::TitleSpec, CutError> {
    let preset = a.preset.as_deref().unwrap_or("lower_third").to_string();
    let color = a.color.as_deref().unwrap_or("#FFFFFF").to_string();
    let duration_ms = a.range_ms[1] - a.range_ms[0];
    let fps_u = (fps.round() as i64).max(1) as u32;
    // Free placement: when BOTH x and y are supplied, the title is anchored
    // at that normalized point regardless of preset — drives the drag-to-place
    // affordance in the Title drawer. Otherwise the preset fixes the position.
    let free = a.x.zip(a.y);
    // Template default font is a touch larger (templates are display/headline
    // looks); preset/free keep their existing defaults.
    let font_px = a.font_px.unwrap_or(if a.template.is_some() {
        h as f64 * 0.085
    } else if free.is_some() {
        h as f64 * 0.07
    } else if preset == "title_card" {
        h as f64 * 0.11
    } else {
        h as f64 * 0.06
    });
    let bg = a.bg.unwrap_or(if free.is_some() {
        false
    } else {
        preset == "lower_third"
    });
    if let Some((cx, cy)) = free {
        normalized_unit_arg("x", cx)?;
        normalized_unit_arg("y", cy)?;
    }

    // TEMPLATE path: a keyframed animated-text look
    // that takes precedence over preset/free and owns its own motion. Validate
    // the template name + the colors up front for a clean error.
    let spec = if let Some(tpl) = a.template.as_deref() {
        if !cut_media::title::TITLE_TEMPLATE_NAMES.contains(&tpl) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown title template '{tpl}'"),
                format!(
                    "template must be one of: {}",
                    cut_media::title::TITLE_TEMPLATE_NAMES.join(", ")
                ),
            ));
        }
        if !cut_media::title::is_valid_color(&color) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid color '{color}'"),
                "color must be #RRGGBB",
            ));
        }
        let accent = a.accent.as_deref().unwrap_or("#FFD24A").to_string();
        if !cut_media::title::is_valid_color(&accent) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid accent '{accent}'"),
                "accent must be #RRGGBB",
            ));
        }
        cut_media::title::build_template(
            tpl,
            &a.text,
            w,
            h,
            fps_u,
            duration_ms,
            &color,
            font_px,
            &accent,
            a.emphasis.as_deref(),
        )
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown title template '{tpl}'"),
                "see title.templates",
            )
        })?
    } else if let Some((cx, cy)) = free {
        let align = match a.align.as_deref() {
            Some("left") => cut_media::title::TextAlign::Left,
            Some("right") => cut_media::title::TextAlign::Right,
            _ => cut_media::title::TextAlign::Center,
        };
        cut_media::title::free_title(
            &a.text,
            w,
            h,
            fps_u,
            duration_ms,
            &color,
            font_px,
            cx,
            cy,
            align,
            bg,
        )
    } else {
        match preset.as_str() {
            "title_card" => {
                cut_media::title::title_card(&a.text, w, h, fps_u, duration_ms, &color, font_px)
            }
            "lower_third" => cut_media::title::lower_third(
                &a.text,
                w,
                h,
                fps_u,
                duration_ms,
                &color,
                font_px,
                bg,
            ),
            "top_bar" => {
                cut_media::title::top_bar(&a.text, w, h, fps_u, duration_ms, &color, font_px, bg)
            }
            "subtitle" => {
                cut_media::title::subtitle(&a.text, w, h, fps_u, duration_ms, &color, font_px)
            }
            "headline" => {
                cut_media::title::headline(&a.text, w, h, fps_u, duration_ms, &color, font_px)
            }
            other => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("unknown title preset '{other}'"),
                    "preset must be lower_third | title_card | top_bar | subtitle | headline",
                ))
            }
        }
    };

    // ANIMATION override (#title-formats): replace every layer's motion with the
    // named animation's envelope, keeping each layer's position/content. Applies
    // across presets + free placement, so the option count = presets × animations.
    // Templates OWN their motion, so the override is skipped for them.
    let mut spec = spec;
    if a.template.is_none() {
        if let Some(anim) = a.animation.as_deref() {
            let kf = cut_media::title::animation_keyframes(anim).ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("unknown title animation '{anim}'"),
                    format!(
                        "animation must be one of: {}",
                        cut_media::title::TITLE_ANIMATIONS.join(", ")
                    ),
                )
            })?;
            for layer in &mut spec.layers {
                layer.keyframes = kf.clone();
            }
        }
    }
    Ok(spec)
}

pub(in crate::dispatch) async fn title_add(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let a: TitleArgs = parse_args(args.clone())?;
    if a.range_ms[1] <= a.range_ms[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "range_ms must be non-empty (end > start)",
            format!("got {:?}", a.range_ms),
        ));
    }
    reject_overlay_too_long(a.range_ms)?;
    let preset = a.preset.as_deref().unwrap_or("lower_third").to_string();
    let duration_ms = a.range_ms[1] - a.range_ms[0];
    let free = a.x.zip(a.y);

    // Geometry/fps for the spec (place_title_overlay does the .mov + import + insert).
    let (w, h, fps) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let s = &store.project.settings;
        (s.width, s.height, s.fps)
    };
    let spec = build_title_spec(&a, w, h, fps)?;

    let (title_track, asset_id, clip_id, op_id) = place_title_overlay(
        state,
        &spec,
        a.range_ms[0],
        duration_ms,
        actor,
        a.rationale
            .clone()
            .unwrap_or_else(|| format!("title \"{}\"", a.text)),
        "title.add",
        args.clone(),
    )
    .await?;

    // The result reports the look used: the template name when a template drove
    // it, else the preset (or "free" for x/y placement).
    let look = a.template.clone().unwrap_or_else(|| {
        if free.is_some() {
            "free".to_string()
        } else {
            preset.clone()
        }
    });
    Ok(VerbResult::ok_with_ops(
        json!({
            "title_track": title_track,
            "asset_id": asset_id,
            "clip_id": clip_id,
            "preset": look,
            "template": a.template,
            "range_ms": a.range_ms,
        }),
        vec![op_id],
    ))
}

/// The spec-shaping fields a `title.update` call may override on a placed title
/// (everything in [`TitleArgs`] EXCEPT `text`/`range_ms`, which are handled
/// explicitly — `text` is the common edit, `range_ms` is intentionally NOT
/// editable here so the re-render keeps the SAME duration). Used to fold a
/// title.update's provided fields onto a recovered title.add spec.
const TITLE_OVERRIDE_KEYS: &[&str] = &[
    "text",
    "color",
    "preset",
    "font_px",
    "bg",
    "x",
    "y",
    "align",
    "animation",
    "template",
    "accent",
    "emphasis",
];

/// Fold the spec-shaping fields PRESENT in `overrides` onto the `base` title-args
/// object (mutating `base`). Only the keys in [`TITLE_OVERRIDE_KEYS`] move, and
/// only when present and non-null — so a title.update that passes just `{text}`
/// changes the words and keeps every other styling field. `clip`/`range_ms`/
/// `rationale` are never folded.
fn apply_title_overrides(base: &mut Value, overrides: &Value) {
    if let (Some(obj), Some(ov)) = (base.as_object_mut(), overrides.as_object()) {
        for k in TITLE_OVERRIDE_KEYS {
            if let Some(v) = ov.get(*k) {
                if !v.is_null() {
                    obj.insert((*k).to_string(), v.clone());
                }
            }
        }
    }
}

/// Recover the FULL declarative title args ([`TitleArgs`] shape, as JSON) that a
/// title clip currently renders from, by replaying the op-log:
///  1. find the `title.add` op that CREATED this clip (its lowered `edit.insert`
///     effect recorded `added_clip == clip_id`) → the base args (the original spec);
///  2. fold every later `title.update {clip}` op's overrides on top, in op order,
///     so a chain of edits accumulates correctly.
///
/// Errors honestly when the clip wasn't created by `title.add`: a `captions.kinetic`
/// title's text comes from the transcript (not a single editable field), and a
/// clip with no creating title op can't have its spec reconstructed. This is the
/// boundary that keeps `title.update` from silently producing a wrong render.
fn recover_title_args(ops: &[OpRecord], clip_id: &str) -> Result<Value, CutError> {
    let mut base: Option<Value> = None;
    let mut created_by_kinetic = false;
    for op in ops {
        if op.verb == "title.add" || op.verb == "captions.kinetic" {
            let added = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("added_clip").and_then(|v| v.as_str()));
            if added == Some(clip_id) {
                base = Some(op.args.clone());
                created_by_kinetic = op.verb == "captions.kinetic";
            }
        }
    }
    let base = base.ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "can't recover this title's spec to edit",
            "no title.add op created this clip — it may have been generated another way; re-create the title to edit its text",
        )
        .with_clip(clip_id)
    })?;
    if created_by_kinetic {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "this title was generated from captions (captions.kinetic)",
            "its text comes from the transcript — edit the captions/transcript, not title.update",
        )
        .with_clip(clip_id));
    }
    // Fold any prior edits to THIS clip, in op order, so the recovered spec is
    // the CURRENT one (chained title.update calls accumulate).
    let mut merged = base;
    for op in ops {
        if op.verb == "title.update"
            && op.args.get("clip").and_then(|v| v.as_str()) == Some(clip_id)
        {
            apply_title_overrides(&mut merged, &op.args);
        }
    }
    Ok(merged)
}

/// title.update{clip, text?, color?, preset?, font_px?, bg?, x?, y?, align?,
/// animation?, template?, accent?, emphasis?, rationale?} — edit a PLACED title
/// clip's text (and/or style) IN PLACE (title-editing regression). `title.add`
/// only ADDS a title; selecting one gave a generic "Video clip" inspector with
/// no way to change its words. This recovers the originating `title.add` spec
/// from the op-log, merges the provided overrides (`text` is the common case),
/// RE-RENDERS a fresh transparent overlay `.mov` at the SAME duration, and swaps
/// the clip onto the new asset — the clip id, range, track, and position are all
/// kept. Committed as ONE lowered op (the `edit.set_asset` swap; the new `.mov`'s
/// `media.import` rides ahead of it) so undo reverts the whole edit and replay
/// re-imports + re-swaps deterministically.
pub(in crate::dispatch) async fn title_update(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        text: Option<String>,
        color: Option<String>,
        preset: Option<String>,
        font_px: Option<f64>,
        bg: Option<bool>,
        x: Option<f64>,
        y: Option<f64>,
        align: Option<String>,
        animation: Option<String>,
        template: Option<String>,
        accent: Option<String>,
        emphasis: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;

    // 1. The clip must exist AND be a title overlay clip (a clip on a track whose
    //    id starts with "title"). Capture geometry/fps for the re-render here too.
    let (track_id, w, h, fps) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (tid, _idx) = store.project.find_clip(&a.clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("clip '{}' not found", a.clip),
                "pass a title clip id (on a `title*` overlay track)",
            )
        })?;
        if !tid.starts_with("title") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "title.update edits title clips; this is not one",
                "select a title clip (on a `title*` overlay track) — use captions.set_text for captions, edit.* for media clips",
            )
            .with_clip(&a.clip));
        }
        let s = &store.project.settings;
        (tid.to_string(), s.width, s.height, s.fps)
    };

    // 2. Recover the originating title.add spec from the op-log (errors honestly
    //    if it can't be reconstructed, e.g. a kinetic-captions title).
    let ops = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.log.read_all()?
    };
    let mut merged = recover_title_args(&ops, &a.clip)?;

    // 3. Fold THIS call's overrides on top of the recovered (current) spec.
    apply_title_overrides(&mut merged, &args);

    // 4. Rebuild the spec via the SAME path title.add uses (so an edited title
    //    renders identically except for the changed field). Re-validate the range
    //    defensively (it came from a valid title.add, but never trust blindly).
    let ta: TitleArgs = serde_json::from_value(merged.clone()).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "recovered title spec did not parse",
            e.to_string(),
        )
        .with_clip(&a.clip)
    })?;
    if ta.range_ms[1] <= ta.range_ms[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "recovered title range is empty",
            format!("got {:?}", ta.range_ms),
        )
        .with_clip(&a.clip));
    }
    reject_overlay_too_long(ta.range_ms)?;
    let duration_ms = ta.range_ms[1] - ta.range_ms[0];
    let spec = build_title_spec(&ta, w, h, fps)?;

    // 5. Render the new overlay .mov + register it (a media.import op).
    let new_asset = render_title_asset(state, &spec, duration_ms).await?;

    // 6. Commit the swap as ONE lowered op named "title.update": its single step
    //    repoints the clip at the new asset (edit.set_asset). Undo reverts the
    //    whole edit; replay re-imports the persisted .mov then re-swaps.
    let step = InverseOp {
        verb: "edit.set_asset".into(),
        args: json!({ "clip": a.clip, "asset": new_asset }),
    };
    let op = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        guard_call("title.update", || {
            store.apply_lowered(
                "title.update",
                args.clone(),
                actor,
                a.rationale
                    .clone()
                    .or_else(|| Some(format!("title.update \"{}\"", ta.text))),
                vec![step],
                vec![],
            )
        })?
    };
    state.events.publish(Event::OpApplied { op: op.clone() });

    Ok(VerbResult::ok_with_ops(
        json!({
            "clip": a.clip,
            "track": track_id,
            "asset_id": new_asset,
            "text": ta.text,
        }),
        vec![op.op_id],
    ))
}

/// title.templates — list the animated-text TEMPLATE catalog: each entry
/// is {name, description, params}. A pure read (no project needed) so a UI /
/// agent can discover the looks before calling `title.add {template}`. Built
/// from the in-house catalog, so it never drifts from what `title.add` accepts.
pub(in crate::dispatch) async fn title_templates(
    _state: &AppState,
    _args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let templates: Vec<Value> = cut_media::title::TITLE_TEMPLATES
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "params": t.params }))
        .collect();
    Ok(VerbResult::ok(json!({ "templates": templates })))
}

/// edit.add_shape — place a vector SHAPE overlay: rect | ellipse | line |
/// arrow (+ an optional centered label for box shapes = a styled callout box).
/// Rendered by the same in-house resvg keyframe engine as titles (so it gets
/// overlay placement + fade animation + content-addressing) and placed on a
/// top-most title overlay track over `range_ms`. Lowers to honest import +
/// insert ops (the generated .mov lives in the project's titles/ dir).
/// edit.add_shape / shape.update argument shape — the declarative source of a
/// vector-shape [`cut_media::title::TitleSpec`]. Lifted to module scope (was
/// inline in `edit_add_shape`) so `shape_update` can deserialize a RECOVERED
/// edit.add_shape op's args into the SAME struct and rebuild the spec through the
/// SAME [`build_shape_spec`] path — guaranteeing an edited shape renders
/// identically to the original except for the changed field(s). `shape` +
/// `range_ms` are the only required fields; everything else carries the
/// box/endpoint/paint defaults. NOTE: `text` is the centered label (the public
/// `shape.update` field for it is `label`, mapped to `text` by
/// [`apply_shape_overrides`]). (shape-editing regression)
#[derive(serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct ShapeArgs {
    pub(crate) shape: String,
    pub(crate) range_ms: [u64; 2],
    /// Box (rect/ellipse) OR start point (line/arrow) — normalized [0,1].
    pub(crate) x: Option<f64>,
    pub(crate) y: Option<f64>,
    pub(crate) w: Option<f64>,
    pub(crate) h: Option<f64>,
    /// End point (line/arrow) — normalized [0,1].
    pub(crate) x2: Option<f64>,
    pub(crate) y2: Option<f64>,
    pub(crate) fill: Option<String>,
    pub(crate) stroke: Option<String>,
    pub(crate) stroke_px: Option<f64>,
    pub(crate) opacity: Option<f64>,
    pub(crate) radius_px: Option<f64>,
    pub(crate) head_px: Option<f64>,
    /// Optional centered label for a box shape (styled text box).
    pub(crate) text: Option<String>,
    /// Label color (#RRGGBB).
    pub(crate) color: Option<String>,
    pub(crate) font_px: Option<f64>,
    /// Entry/exit animation override (fade|slide_*|pop|none); default fade.
    pub(crate) animation: Option<String>,
    pub(crate) rationale: Option<String>,
}

/// Build a vector-shape [`cut_media::title::TitleSpec`] from declarative
/// [`ShapeArgs`] for the given canvas geometry/fps — the pure spec-construction
/// core shared by `edit_add_shape` (PLACE a new shape) and `shape_update`
/// (RE-RENDER an edited shape's overlay). Validates the shape kind + colors,
/// resolves the box/endpoint + paint defaults, and applies the optional
/// animation override. Assumes the range has already been validated (end >
/// start, not absurdly long) by the caller. (shape-editing regression)
pub(crate) fn build_shape_spec(
    a: &ShapeArgs,
    w_px: u32,
    h_px: u32,
    fps: f64,
) -> Result<cut_media::title::TitleSpec, CutError> {
    if !cut_media::title::SHAPE_KINDS.contains(&a.shape.as_str()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown shape '{}'", a.shape),
            format!(
                "shape must be one of: {}",
                cut_media::title::SHAPE_KINDS.join(", ")
            ),
        ));
    }
    // Validate any provided colors up front for a clean error.
    for (label, c) in [
        ("fill", &a.fill),
        ("stroke", &a.stroke),
        ("color", &a.color),
    ] {
        if let Some(c) = c {
            if !cut_media::title::is_valid_color(c) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("invalid {label} '{c}'"),
                    format!("{label} must be #RRGGBB"),
                ));
            }
        }
    }
    let duration_ms = a.range_ms[1] - a.range_ms[0];
    let fps_u = (fps.round() as i64).max(1) as u32;

    let is_box = matches!(a.shape.as_str(), "rect" | "ellipse");
    // Box defaults (rect/ellipse) vs endpoint defaults (line/arrow).
    let (x, y, w, h) = (
        a.x.unwrap_or(if is_box { 0.30 } else { 0.20 }),
        a.y.unwrap_or(if is_box { 0.40 } else { 0.50 }),
        a.w.unwrap_or(0.40),
        a.h.unwrap_or(0.20),
    );
    let (x2, y2) = (a.x2.unwrap_or(0.80), a.y2.unwrap_or(0.50));
    normalized_unit_arg("x", x)?;
    normalized_unit_arg("y", y)?;
    if is_box {
        positive_normalized_unit_arg("w", w)?;
        positive_normalized_unit_arg("h", h)?;
        validate_normalized_box_axis("x", "w", x, w)?;
        validate_normalized_box_axis("y", "h", y, h)?;
    } else {
        normalized_unit_arg("x2", x2)?;
        normalized_unit_arg("y2", y2)?;
        if (x - x2).abs() < f64::EPSILON && (y - y2).abs() < f64::EPSILON {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "line/arrow endpoints must not be identical",
                "identical endpoints render an invisible shape",
            ));
        }
    }
    // If neither fill nor stroke is given for a box, default to a white outline
    // so the shape is never invisible.
    let (fill, stroke) = if is_box && a.fill.is_none() && a.stroke.is_none() {
        (None, Some("#FFFFFF".to_string()))
    } else {
        (a.fill.clone(), a.stroke.clone())
    };
    let params = cut_media::title::ShapeParams {
        fill,
        opacity: a.opacity.unwrap_or(1.0).clamp(0.0, 1.0),
        stroke,
        stroke_px: a.stroke_px.unwrap_or(if is_box { 4.0 } else { 8.0 }),
        radius_px: a.radius_px.unwrap_or(0.0),
        head_px: a.head_px.unwrap_or(0.0),
        text: a.text.clone(),
        text_color: a.color.clone().unwrap_or_else(|| "#FFFFFF".to_string()),
        font_px: a.font_px.unwrap_or(h_px as f64 * 0.05),
    };
    let mut spec = cut_media::title::build_shape(
        &a.shape,
        x,
        y,
        w,
        h,
        x2,
        y2,
        &params,
        w_px,
        h_px,
        fps_u,
        duration_ms,
    )
    .ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown shape '{}'", a.shape),
            "see edit.add_shape",
        )
    })?;

    // Animation override (default is the builder's fade envelope).
    if let Some(anim) = a.animation.as_deref() {
        let kf = cut_media::title::animation_keyframes(anim).ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown animation '{anim}'"),
                format!(
                    "animation must be one of: {}",
                    cut_media::title::TITLE_ANIMATIONS.join(", ")
                ),
            )
        })?;
        for layer in &mut spec.layers {
            layer.keyframes = kf.clone();
        }
    }
    Ok(spec)
}

pub(in crate::dispatch) async fn edit_add_shape(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let a: ShapeArgs = parse_args(args.clone())?;
    if a.range_ms[1] <= a.range_ms[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "range_ms must be non-empty (end > start)",
            format!("got {:?}", a.range_ms),
        ));
    }
    reject_overlay_too_long(a.range_ms)?;
    let duration_ms = a.range_ms[1] - a.range_ms[0];
    let (w_px, h_px, fps) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let s = &store.project.settings;
        (s.width, s.height, s.fps)
    };
    let spec = build_shape_spec(&a, w_px, h_px, fps)?;

    let (title_track, asset_id, clip_id, op_id) = place_title_overlay(
        state,
        &spec,
        a.range_ms[0],
        duration_ms,
        actor,
        a.rationale
            .clone()
            .unwrap_or_else(|| format!("shape {}", a.shape)),
        "edit.add_shape",
        args.clone(),
    )
    .await?;
    Ok(VerbResult::ok_with_ops(
        json!({
            "shape_track": title_track,
            "asset_id": asset_id,
            "clip_id": clip_id,
            "shape": a.shape,
            "range_ms": a.range_ms,
        }),
        vec![op_id],
    ))
}

/// The spec-shaping fields a `shape.update` call may override on a placed shape.
/// These map 1:1 onto [`ShapeArgs`]/`edit.add_shape` field names. `range_ms` is
/// intentionally NOT here (the re-render keeps the SAME duration, mirroring
/// title.update); `clip`/`rationale` are control fields. The centered-label text
/// is handled separately: the public field is `label`, folded onto `text`.
const SHAPE_OVERRIDE_KEYS: &[&str] = &[
    "shape",
    "color",
    "fill",
    "stroke",
    "stroke_px",
    "opacity",
    "radius_px",
    "head_px",
    "font_px",
    "animation",
    "x",
    "y",
    "w",
    "h",
    "x2",
    "y2",
];

/// Fold the spec-shaping fields PRESENT in `overrides` onto the `base`
/// shape-args object (mutating `base`, which is in `edit.add_shape` arg shape).
/// Only the keys in [`SHAPE_OVERRIDE_KEYS`] move, and only when present and
/// non-null — so a `shape.update` that passes just `{label}` changes the label
/// and keeps every other field. The public `label` field maps to `edit.add_shape`'s
/// `text` (the field [`build_shape_spec`] reads). `clip`/`range_ms`/`rationale`
/// are never folded.
pub(in crate::dispatch) fn apply_shape_overrides(base: &mut Value, overrides: &Value) {
    if let (Some(obj), Some(ov)) = (base.as_object_mut(), overrides.as_object()) {
        for k in SHAPE_OVERRIDE_KEYS {
            if let Some(v) = ov.get(*k) {
                if !v.is_null() {
                    obj.insert((*k).to_string(), v.clone());
                }
            }
        }
        // `label` is the PUBLIC shape.update field for the centered text; it maps
        // to edit.add_shape's `text` (build_shape_spec reads `text`).
        if let Some(v) = ov.get("label") {
            if !v.is_null() {
                obj.insert("text".to_string(), v.clone());
            }
        }
    }
}

/// Recover the FULL declarative shape args ([`ShapeArgs`] shape, as JSON) that a
/// shape clip currently renders from, by replaying the op-log:
///  1. find the `edit.add_shape` op that CREATED this clip (its lowered
///     `edit.insert` effect recorded `added_clip == clip_id`) → the base args;
///  2. fold every later `shape.update {clip}` op's overrides on top, in op order,
///     so a chain of edits accumulates correctly.
///
/// This is ALSO the shape-vs-title DISTINCTION: titles and shapes both live on
/// `title*` tracks, but a shape clip has an `edit.add_shape` creating op while a
/// title clip has a `title.add` one. A title clip (or any non-shape clip) reaches
/// no base here → a clean, actionable error instead of a wrong render.
fn recover_shape_args(ops: &[OpRecord], clip_id: &str) -> Result<Value, CutError> {
    let mut base: Option<Value> = None;
    for op in ops {
        if op.verb == "edit.add_shape" {
            let added = op
                .effects
                .iter()
                .find_map(|e| e.detail.get("added_clip").and_then(|v| v.as_str()));
            if added == Some(clip_id) {
                base = Some(op.args.clone());
            }
        }
    }
    let base = base.ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "can't recover this shape's spec to edit",
            "no edit.add_shape op created this clip — select a shape overlay clip (from edit.add_shape); titles use title.update",
        )
        .with_clip(clip_id)
    })?;
    // Fold any prior edits to THIS clip, in op order, so the recovered spec is the
    // CURRENT one (chained shape.update calls accumulate).
    let mut merged = base;
    for op in ops {
        if op.verb == "shape.update"
            && op.args.get("clip").and_then(|v| v.as_str()) == Some(clip_id)
        {
            apply_shape_overrides(&mut merged, &op.args);
        }
    }
    Ok(merged)
}

/// shape.update{clip, shape?, label?, color?, fill?, stroke?, stroke_px?,
/// opacity?, radius_px?, head_px?, font_px?, animation?, x?, y?, w?, h?, x2?, y2?,
/// rationale?} — edit a PLACED shape overlay clip's properties (shape kind, label
/// text, color, geometry, …) IN PLACE (shape-editing regression). `edit.add_shape`
/// only ADDS a shape; selecting one gave a generic "Video clip" inspector with no
/// way to change it. This recovers the originating `edit.add_shape` spec from the
/// op-log, merges the provided overrides (`label`/`color` are the common cases),
/// RE-RENDERS a fresh transparent overlay `.mov` at the SAME duration via the SAME
/// render path edit.add_shape uses, and swaps the clip onto the new asset — the
/// clip id, range, track, and position are all kept. Committed as ONE lowered op
/// (the `edit.set_asset` swap; the new `.mov`'s `media.import` rides ahead of it)
/// so undo reverts the whole edit and replay re-imports + re-swaps deterministically.
/// The clip must be a shape overlay clip (created by `edit.add_shape`, on a
/// `title*` track); a title clip is refused (use `title.update`).
pub(in crate::dispatch) async fn shape_update(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        shape: Option<String>,
        /// The centered label text (maps to edit.add_shape's `text`).
        label: Option<String>,
        color: Option<String>,
        fill: Option<String>,
        stroke: Option<String>,
        stroke_px: Option<f64>,
        opacity: Option<f64>,
        radius_px: Option<f64>,
        head_px: Option<f64>,
        font_px: Option<f64>,
        animation: Option<String>,
        x: Option<f64>,
        y: Option<f64>,
        w: Option<f64>,
        h: Option<f64>,
        x2: Option<f64>,
        y2: Option<f64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;

    // 1. The clip must exist AND be on a `title*` overlay track (shapes share the
    //    title overlay tracks). Capture geometry/fps for the re-render here too.
    let (track_id, w_px, h_px, fps) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (tid, _idx) = store.project.find_clip(&a.clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("clip '{}' not found", a.clip),
                "pass a shape clip id (on a `title*` overlay track, from edit.add_shape)",
            )
        })?;
        if !tid.starts_with("title") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "shape.update edits shape clips; this is not one",
                "select a shape clip (on a `title*` overlay track, from edit.add_shape) — use title.update for titles, edit.* for media clips",
            )
            .with_clip(&a.clip));
        }
        let s = &store.project.settings;
        (tid.to_string(), s.width, s.height, s.fps)
    };

    // 2. Recover the originating edit.add_shape spec from the op-log (errors
    //    honestly if it can't be reconstructed — e.g. the clip is a title, which
    //    has a title.add creating op, not an edit.add_shape one).
    let ops = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.log.read_all()?
    };
    let mut merged = recover_shape_args(&ops, &a.clip)?;

    // 3. Fold THIS call's overrides on top of the recovered (current) spec.
    apply_shape_overrides(&mut merged, &args);

    // 4. Rebuild the spec via the SAME path edit.add_shape uses (so an edited
    //    shape renders identically except for the changed field). Re-validate the
    //    range defensively (it came from a valid edit.add_shape, but never trust
    //    blindly).
    let sa: ShapeArgs = serde_json::from_value(merged.clone()).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "recovered shape spec did not parse",
            e.to_string(),
        )
        .with_clip(&a.clip)
    })?;
    if sa.range_ms[1] <= sa.range_ms[0] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "recovered shape range is empty",
            format!("got {:?}", sa.range_ms),
        )
        .with_clip(&a.clip));
    }
    reject_overlay_too_long(sa.range_ms)?;
    let duration_ms = sa.range_ms[1] - sa.range_ms[0];
    let spec = build_shape_spec(&sa, w_px, h_px, fps)?;

    // 5. Render the new overlay .mov + register it (a media.import op).
    let new_asset = render_title_asset(state, &spec, duration_ms).await?;

    // 6. Commit the swap as ONE lowered op named "shape.update": its single step
    //    repoints the clip at the new asset (edit.set_asset). Undo reverts the
    //    whole edit; replay re-imports the persisted .mov then re-swaps.
    let step = InverseOp {
        verb: "edit.set_asset".into(),
        args: json!({ "clip": a.clip, "asset": new_asset }),
    };
    let op = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        guard_call("shape.update", || {
            store.apply_lowered(
                "shape.update",
                args.clone(),
                actor,
                a.rationale
                    .clone()
                    .or_else(|| Some(format!("shape.update {}", sa.shape))),
                vec![step],
                vec![],
            )
        })?
    };
    state.events.publish(Event::OpApplied { op: op.clone() });

    Ok(VerbResult::ok_with_ops(
        json!({
            "clip": a.clip,
            "track": track_id,
            "asset_id": new_asset,
            "shape": sa.shape,
            "label": sa.text,
        }),
        vec![op.op_id],
    ))
}
