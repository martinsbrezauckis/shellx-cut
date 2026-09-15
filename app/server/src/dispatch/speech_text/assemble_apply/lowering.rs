//! Deterministic lower-level timeline steps for one Assemble materialization.

use super::super::*;
use super::binding::{Materialization, ASSEMBLE_CAPTION_TRACK_ID};
use super::captions::{added_clip_id, append_short_captions};
use super::planning::{
    crop_pixels, destination_track, ensure_short_geometry, locked_track_error, source_spans,
};

pub(super) struct MaterializationPlan {
    pub(super) steps: Vec<InverseOp>,
    pub(super) effects: Vec<OpEffect>,
    pub(super) receipt: MaterializationReceipt,
}

pub(super) struct MaterializationReceipt {
    pub(super) spans_placed: usize,
    pub(super) video_track: String,
    pub(super) audio_track: Option<String>,
    pub(super) video_clip_ids: Vec<String>,
    pub(super) audio_clip_ids: Vec<String>,
    pub(super) caption_track: Option<String>,
    pub(super) caption_clip_ids: Vec<String>,
    pub(super) total_ms: u64,
}

pub(super) fn build_materialization_plan(
    store: &ProjectStore,
    asset: &str,
    words: &[cut_perception::WordSpan],
    selected_ranges: &[[usize; 2]],
    materialization: &Materialization,
) -> Result<MaterializationPlan, CutError> {
    let source = store.project.assets.get(asset).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{asset}'"),
            "the reviewed source is no longer in this project",
        )
    })?;
    let source_duration = source
        .probe
        .as_ref()
        .and_then(|probe| probe.get("duration_ms"))
        .and_then(Value::as_u64);
    let has_audio = source
        .probe
        .as_ref()
        .and_then(|probe| probe.get("has_audio"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let spans = source_spans(
        words,
        selected_ranges,
        &store.project.transcript_ignores,
        asset,
        source_duration,
    )?;

    let video_track = destination_track(&store.project, cut_core::TrackKind::Video, "video")?;
    let audio_track = if has_audio {
        Some(destination_track(
            &store.project,
            cut_core::TrackKind::Audio,
            "audio",
        )?)
    } else {
        None
    };
    if matches!(materialization, Materialization::Shorts { .. }) {
        ensure_short_geometry(&store.project, materialization)?;
        if let Some(track) = store.project.track(ASSEMBLE_CAPTION_TRACK_ID) {
            if track.locked {
                return Err(locked_track_error(ASSEMBLE_CAPTION_TRACK_ID));
            }
        }
    }

    // Appending after the later A/V end preserves existing clips and guarantees
    // no overlapping placement on either target lane.
    let video_end = store
        .project
        .track(&video_track)
        .map(cut_core::Track::duration_ms)
        .unwrap_or(0);
    let audio_end = audio_track
        .as_deref()
        .and_then(|track| store.project.track(track))
        .map(cut_core::Track::duration_ms)
        .unwrap_or(0);
    let start = video_end.max(audio_end);

    let crop = match materialization {
        Materialization::Reel => None,
        Materialization::Shorts {
            crop: Some(crop), ..
        } => Some(crop_pixels(source, *crop)?),
        Materialization::Shorts { crop: None, .. } => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "the reviewed short plan has no usable source crop",
                "run media.probe, review the short plan again, then apply it",
            ));
        }
    };
    let mut next = store.project.clone();
    let mut steps = Vec::new();
    let mut video_clip_ids = Vec::new();
    let mut audio_clip_ids = Vec::new();
    let mut caption_words = Vec::new();
    let mut at = start;

    for span in &spans {
        let insert = json!({
            "asset": asset,
            "track": video_track,
            "at_ms": at,
            "src_range_ms": span.source_range,
            "ripple": false,
        });
        let effects = cut_core::apply_edit_verb(&mut next, "edit.insert", &insert)?;
        let clip_id = added_clip_id(&effects, &video_track)?;
        steps.push(InverseOp {
            verb: "edit.insert".into(),
            args: insert,
        });
        video_clip_ids.push(clip_id.clone());

        if let Some([x, y, w, h]) = crop {
            let crop_args = json!({"clip": clip_id, "x": x, "y": y, "w": w, "h": h});
            cut_core::apply_edit_verb(&mut next, "edit.crop", &crop_args)?;
            steps.push(InverseOp {
                verb: "edit.crop".into(),
                args: crop_args,
            });
        }

        if let Some(audio_track) = &audio_track {
            let insert = json!({
                "asset": asset,
                "track": audio_track,
                "at_ms": at,
                "src_range_ms": span.source_range,
                "ripple": false,
            });
            let effects = cut_core::apply_edit_verb(&mut next, "edit.insert", &insert)?;
            let clip_id = added_clip_id(&effects, audio_track)?;
            steps.push(InverseOp {
                verb: "edit.insert".into(),
                args: insert,
            });
            audio_clip_ids.push(clip_id);
        }

        if matches!(materialization, Materialization::Shorts { .. }) {
            for word in &words[span.word_range[0]..=span.word_range[1]] {
                let start_ms =
                    at.saturating_add(word.start_ms.saturating_sub(span.source_range[0]));
                let end_ms = at.saturating_add(
                    word.end_ms
                        .min(span.source_range[1])
                        .saturating_sub(span.source_range[0]),
                );
                if start_ms < end_ms {
                    caption_words.push((start_ms, end_ms, word.word.clone()));
                }
            }
        }
        at += span.source_range[1] - span.source_range[0];
    }

    let (caption_track, caption_clip_ids) =
        if matches!(materialization, Materialization::Shorts { .. }) {
            caption_words.sort();
            caption_words.dedup();
            let (tracks, caption_ids) = append_short_captions(&next, caption_words)?;
            let track = ASSEMBLE_CAPTION_TRACK_ID.to_string();
            steps.push(InverseOp {
                verb: "edit._set_timeline".into(),
                args: json!({
                    "tracks": tracks,
                    "markers": next.markers,
                    "caption_styles": next.caption_styles,
                }),
            });
            (Some(track), caption_ids)
        } else {
            (None, Vec::new())
        };

    let effects = vec![effect(
        Some(&video_track),
        json!({
            "assemble_materialized": materialization.label(),
            "spans_placed": spans.len(),
            "video_clip_ids": video_clip_ids,
            "audio_clip_ids": audio_clip_ids,
            "caption_clip_ids": caption_clip_ids,
            "range_ms": [start, at],
        }),
    )];
    Ok(MaterializationPlan {
        steps,
        effects,
        receipt: MaterializationReceipt {
            spans_placed: spans.len(),
            video_track,
            audio_track,
            video_clip_ids,
            audio_clip_ids,
            caption_track,
            caption_clip_ids,
            total_ms: at - start,
        },
    })
}
