//! Edit, title, asset-tool, plugin, and audio dispatch handlers.
//!
//! Kept as a child module of `dispatch` so this extraction is behavior-preserving:
//! handlers still share central commit, render, media, transcript, and event helpers.

use super::*;

mod linked_move;
use linked_move::resolve_linked_media;

mod timeline;
pub(super) use timeline::{
    captions_set_range, captions_set_text, edit_crossfade, edit_cut_to_beat, edit_detach_audio,
    edit_duplicate, edit_fade, edit_fit_to_fill, edit_gain, edit_insert, edit_mark_scenes,
    edit_move, edit_move_marker, edit_mute_range, edit_nest, edit_paste, edit_replace,
    edit_ripple_delete, edit_seek_marker, edit_split, edit_split_at_scenes, edit_split_edit,
    edit_trim, edit_trim_edges, edit_update_marker,
};

mod visual;
pub(super) use visual::{
    edit_add_mask, edit_adjustment, edit_animate, edit_auto_balance, edit_auto_zoom,
    edit_color_match, edit_color_space, edit_crop, edit_effect, edit_freeze, edit_grade,
    edit_grade_stack, edit_grade_window, edit_keyframe, edit_matte, edit_redact, edit_reverse,
    edit_stabilize, edit_transform, grade_apply, grade_list, grade_save,
};

mod multicam;
pub(super) use multicam::{edit_multicam_switch, edit_multicam_sync};

mod track_markers;
pub(super) use track_markers::{
    edit_add_marker, edit_add_track, edit_blend, edit_mute, edit_pan, edit_remove_marker,
    edit_remove_track, edit_reorder_track, edit_restore, edit_roll, edit_slide_edit, edit_slip,
    edit_solo, edit_track, edit_track_lock, edit_track_visible,
};

mod audio;
pub(super) use audio::{assemble_broll, audio_add_music, audio_cleanup_voice, edit_duck, edit_eq};

mod speed;
pub(super) use speed::{edit_speed, edit_speed_ramp};

mod title_shape;
pub(super) use title_shape::apply_shape_overrides;
use title_shape::place_title_overlay;
#[cfg(test)]
pub(super) use title_shape::resolve_slide;
pub(crate) use title_shape::{build_shape_spec, build_title_spec, ShapeArgs, TitleArgs};
pub(super) use title_shape::{
    edit_add_shape, edit_slide, shape_update, title_add, title_templates, title_update,
};

mod assets_plugins;
pub(super) use assets_plugins::{
    agent_chat, assets_fetch, assets_generate, assets_providers, assets_search, effects_list,
    media_index, media_index_status, media_search, plugins_call, plugins_enable, plugins_list,
    transitions_list,
};

/// captions.kinetic — ANIMATED, transcript-synced captions: each caption line
/// pops in (fade + scale) and fades out in sync with speech, rendered as a
/// native title overlay (the motion-graphics counterpart to captions.generate's
/// STATIC burn-in). Reads the cap1 caption cues (run captions.generate first),
/// optionally narrowed to range_ms, builds a kinetic TitleSpec, and places it on
/// the top-most title1 overlay. Built entirely in-house (resvg + ffmpeg).
pub(super) async fn captions_kinetic(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        range_ms: Option<[u64; 2]>,
        color: Option<String>,
        font_px: Option<f64>,
        position: Option<String>,
        /// When true, REMOVE the cap1 static cues this call animates so the
        /// kinetic overlay shows ALONE (kinetic READS cap1 to animate it, so
        /// without this every animated line ALSO burns in statically = doubled).
        /// Range-aware: cues outside an explicit range_ms stay static. Default
        /// false keeps the existing kinetic-over-static agent contract.
        #[serde(default)]
        replace_static: bool,
        /// PER-WORD ("karaoke") mode: animate each spoken WORD as its own centred
        /// cue for a word-by-word karaoke style instead of one cue per caption
        /// LINE. Reads the word
        /// timings straight from the transcript mapped through the EDL (NOT the
        /// cap1 line cues), so it does not require captions.generate first. Pairs
        /// naturally with replace_static for a pure word-by-word look. Default
        /// false = the existing per-LINE kinetic.
        #[serde(default)]
        per_word: bool,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let color = a.color.as_deref().unwrap_or("#FFFFFF").to_string();
    let pos_y = match a.position.as_deref() {
        Some("top") => 0.08,
        Some("center") | Some("centre") => 0.45,
        _ => 0.78,
    };

    // Read geometry + the cues to animate. PER-WORD: harvest word-level timeline
    // cues from the transcript (one cue per spoken word). PER-LINE (default):
    // read the cap1 caption-line cues. Both feed the SAME kinetic() builder (one
    // layer per cue, each visible only in its window → word-by-word reveal).
    let (w, h, fps, mut cues) = if a.per_word {
        let words = harvest_timeline_words(state).await?;
        let (width, height, fps) = {
            let guard = state.project.read().await;
            let store = guard.as_ref().ok_or_else(no_project)?;
            let s = &store.project.settings;
            (s.width, s.height, s.fps)
        };
        (width, height, fps, words)
    } else {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let s = &store.project.settings;
        let cap = store
            .project
            .tracks
            .iter()
            .find(|t| t.kind == cut_core::TrackKind::Caption)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    "no caption track to animate",
                    "run captions.generate first (kinetic captions animate the existing cues), or pass per_word:true to animate the transcript word-by-word",
                )
            })?;
        let cues: Vec<(u64, u64, String)> = cap
            .clips
            .iter()
            .filter_map(|c| match c {
                cut_core::Clip::Caption(cc) => {
                    Some((cc.range_ms[0], cc.range_ms[1], cc.text.clone()))
                }
                _ => None,
            })
            .collect();
        (s.width, s.height, s.fps, cues)
    };
    if let Some([r0, r1]) = a.range_ms {
        cues.retain(|(cs, ce, _)| *ce > r0 && *cs < r1);
    }
    if cues.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            if a.per_word {
                "no transcribed words in range"
            } else {
                "no caption cues in range"
            },
            if a.per_word {
                "transcribe the footage first (per_word animates transcript words), or widen range_ms"
            } else {
                "captions.generate first, or widen range_ms"
            },
        ));
    }
    cues.sort_by_key(|(cs, _, _)| *cs);
    let t0 = cues.first().unwrap().0;
    let t1 = cues.iter().map(|(_, ce, _)| *ce).max().unwrap();
    let dur = (t1 - t0).max(1);
    let fps_u = (fps.round() as i64).max(1) as u32;
    // Single words read larger than lines — bump the default font in per-word mode
    // (a caller-supplied font_px still wins).
    let font_px = a
        .font_px
        .unwrap_or(h as f64 * if a.per_word { 0.075 } else { 0.05 });
    // Normalize cue times to the title's local [0,1].
    let norm: Vec<(f64, f64, String)> = cues
        .iter()
        .map(|(cs, ce, text)| {
            (
                (*cs - t0) as f64 / dur as f64,
                (*ce - t0) as f64 / dur as f64,
                text.clone(),
            )
        })
        .collect();
    let spec = cut_media::title::kinetic(&norm, w, h, fps_u, dur, &color, font_px, pos_y);
    let group_id = if a.replace_static {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        Some(format!("grp-captions_kinetic-{}", store.log.next_id()?))
    } else {
        None
    };
    let mut placement_args = args.clone();
    if let Some(g) = &group_id {
        if let Some(obj) = placement_args.as_object_mut() {
            obj.insert("group_id".into(), json!(g));
        }
    }
    let (title_track, asset_id, clip_id, place_op_id) = place_title_overlay(
        state,
        &spec,
        t0,
        dur,
        actor.clone(),
        a.rationale.clone().unwrap_or_else(|| {
            format!(
                "kinetic captions ({} {})",
                cues.len(),
                if a.per_word {
                    "words, per-word"
                } else {
                    "cues"
                }
            )
        }),
        "captions.kinetic",
        placement_args,
    )
    .await?;
    // op_ids: the overlay placement op + (if replace_static cleared cues) the
    // cap1-clear op. When both land, they share a group_id so one Undo removes
    // the overlay and restores the static cues together.
    let mut op_ids = vec![place_op_id];

    // replace_static: drop the cap1 STATIC cues this call just animated so the
    // kinetic overlay shows ALONE. Committed as one lowered edit._set_timeline
    // step — the same replayable path captions.reflow/shift use — so it undoes
    // cleanly alongside the kinetic placement. Range-aware: only the cues that
    // overlap an explicit range_ms are cleared (cues outside stay static); no
    // range = the whole track was animated, so all static cues clear.
    let mut cleared_static = 0usize;
    if a.replace_static {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        if let Some(cap_idx) = store
            .project
            .tracks
            .iter()
            .position(|t| t.kind == cut_core::TrackKind::Caption && t.id == "cap1")
        {
            let mut tracks = store.project.tracks.clone();
            let before = tracks[cap_idx].clips.len();
            tracks[cap_idx].clips.retain(|c| match c {
                cut_core::Clip::Caption(cc) => match a.range_ms {
                    Some([r0, r1]) => !(cc.range_ms[1] > r0 && cc.range_ms[0] < r1),
                    None => false,
                },
                _ => true,
            });
            cleared_static = before - tracks[cap_idx].clips.len();
            if cleared_static > 0 {
                let steps = vec![InverseOp {
                    verb: "edit._set_timeline".into(),
                    args: json!({
                        "tracks": tracks,
                        "markers": store.project.markers,
                        "caption_styles": store.project.caption_styles,
                    }),
                }];
                let extra = vec![effect(
                    Some("cap1"),
                    json!({ "cleared_static_cues": cleared_static }),
                )];
                let op = guard_call("captions.kinetic", || {
                    let mut clear_args = json!({ "replace_static": true });
                    if let Some(g) = &group_id {
                        clear_args["group_id"] = json!(g);
                    }
                    store.apply_lowered(
                        "captions.kinetic",
                        clear_args,
                        actor,
                        a.rationale.clone(),
                        steps,
                        extra,
                    )
                })?;
                op_ids.push(op.op_id.clone());
                state.events.publish(Event::OpApplied { op: op.clone() });
            }
        }
    }

    Ok(VerbResult::ok_with_ops(
        json!({
            "title_track": title_track,
            "asset_id": asset_id,
            "clip_id": clip_id,
            "cue_count": cues.len(),
            "range_ms": [t0, t1],
            "cleared_static": cleared_static,
        }),
        op_ids,
    ))
}
