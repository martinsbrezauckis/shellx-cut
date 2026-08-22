//! overwrite_edit.rs — bounded, no-ripple overwrite timeline primitive.
//!
//! `edit.overwrite` replaces a fixed timeline interval and differs from insert:
//! insert still splices and shifts the target track; overwrite consumes exactly
//! the source-duration slot and preserves every later timeline coordinate.
use crate::edit::{fx, make_media_clip};
use crate::error::{codes, CutError};
use crate::ops::OpEffect;
use crate::types::{tl_off_to_src, Clip, ClipFade, GapClip, MediaClip, Project, Track, TrackKind};
use serde_json::json;
use std::collections::BTreeSet;
/// Overwrite one or more video/audio tracks with one source window. Each target
/// receives its own new media clip, but the caller records all effects in ONE
/// operation so linked A/V overwrite is one undo/replay unit.
pub fn overwrite(
    project: &mut Project,
    asset: &str,
    tracks: &[String],
    at_ms: u64,
    src_range_ms: [u64; 2],
) -> Result<Vec<OpEffect>, CutError> {
    overwrite_pinned(project, asset, tracks, at_ms, src_range_ms, None, None)
}
/// Replay-pinned form of [`overwrite`]. An overwrite creates one incoming clip
/// per target and can create a right-side split only when one original clip
/// spans both overwrite edges. Pins keep those identities stable if an earlier
/// operation is rebased out.
pub fn overwrite_pinned(
    project: &mut Project,
    asset: &str,
    tracks: &[String],
    at_ms: u64,
    src_range_ms: [u64; 2],
    pinned_added: Option<&[String]>,
    pinned_splits: Option<&[String]>,
) -> Result<Vec<OpEffect>, CutError> {
    let [src_in, src_out] = src_range_ms;
    let duration_ms = src_out.checked_sub(src_in).ok_or_else(|| {
        CutError::new(
            codes::INVALID_ARGS,
            format!("src_range_ms [{src_in}, {src_out}) is empty or inverted"),
            "source range start must be strictly less than its end",
        )
    })?;
    let overwrite_end = at_ms.checked_add(duration_ms).ok_or_else(|| {
        CutError::new(
            codes::INVALID_ARGS,
            "overwrite end overflows the timeline",
            "at_ms plus the source duration must fit in u64 milliseconds",
        )
    })?;
    validate_overwrite(
        project,
        asset,
        tracks,
        at_ms,
        overwrite_end,
        duration_ms,
        src_out,
    )?;
    let mut effects = Vec::with_capacity(tracks.len());
    let mut split_cursor = 0usize;
    for (target_index, track_id) in tracks.iter().enumerate() {
        let mut reserved_ids = BTreeSet::new();
        let added_id = pinned_added
            .and_then(|pins| pins.get(target_index))
            .cloned()
            .unwrap_or_else(|| next_clip_id(project, &reserved_ids));
        reserved_ids.insert(added_id.clone());
        let snapshot = project.track(track_id).expect("validated target").clone();
        let needs_split = needs_right_split(&snapshot, at_ms, overwrite_end);
        let split_id = needs_split.then(|| {
            let pinned = pinned_splits
                .and_then(|pins| pins.get(split_cursor))
                .cloned();
            split_cursor += 1;
            pinned.unwrap_or_else(|| next_clip_id(project, &reserved_ids))
        });
        effects.push(overwrite_track(
            project,
            &snapshot,
            asset,
            at_ms,
            src_range_ms,
            added_id,
            split_id,
        ));
    }
    Ok(effects)
}
fn next_clip_id(project: &Project, reserved: &BTreeSet<String>) -> String {
    let mut max_id = project
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .chain(
            project
                .nests
                .iter()
                .flat_map(|nest| nest.tracks.iter())
                .flat_map(|track| track.clips.iter()),
        )
        .filter_map(Clip::id)
        .filter_map(|id| id.strip_prefix('c').and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    loop {
        max_id += 1;
        let candidate = format!("c{max_id}");
        if !reserved.contains(&candidate) {
            return candidate;
        }
    }
}
fn validate_overwrite(
    project: &Project,
    asset: &str,
    tracks: &[String],
    at_ms: u64,
    overwrite_end: u64,
    duration_ms: u64,
    src_out: u64,
) -> Result<(), CutError> {
    if tracks.is_empty() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "edit.overwrite needs at least one destination track",
            "pass video_track, audio_track, or both",
        ));
    }
    let source = project.assets.get(asset).ok_or_else(|| {
        CutError::new(
            codes::NOT_FOUND,
            format!("no asset '{asset}'"),
            "import the source asset before overwriting",
        )
    })?;
    if let Some(duration_ms) = source
        .probe
        .as_ref()
        .and_then(|probe| probe.get("duration_ms"))
        .and_then(|value| value.as_u64())
    {
        if src_out > duration_ms {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("src_out_ms {src_out} exceeds asset duration {duration_ms}ms"),
                format!("asset '{asset}' is only {duration_ms}ms long"),
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for track_id in tracks {
        if !seen.insert(track_id) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("destination track '{track_id}' was supplied more than once"),
                "each chosen video or audio track may be overwritten once per operation",
            ));
        }
        let track = project.track(track_id).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no track '{track_id}'"),
                "destination tracks must exist in the open project",
            )
        })?;
        if !matches!(track.kind, TrackKind::Video | TrackKind::Audio) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("cannot overwrite caption track '{track_id}'"),
                "caption timing is managed through captions.* verbs",
            ));
        }
        if track
            .clips
            .iter()
            .any(|clip| matches!(clip, Clip::Caption(_)))
        {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("track '{track_id}' contains a caption clip on a media track"),
                "the project timeline is malformed; caption clips belong on caption tracks",
            ));
        }
        for edge in [at_ms, overwrite_end] {
            if let Some((clip_id, feature)) = edge_splits_time_dependent_clip(track, edge) {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!(
                        "overwrite edge {edge}ms cuts through {feature} clip '{clip_id}' on '{track_id}'"
                    ),
                    format!(
                        "{feature} is defined across the original clip window; v1 will not split it without an exact remap"
                    ),
                )
                .with_clip(clip_id)
                .with_suggested_action(format!(
                    "place the overwrite edge at a clip boundary, or clear {feature} on '{clip_id}' first"
                ))
                .with_at_ms(edge));
            }
        }
        if let Some((clip_id, xfade_ms, head_end)) =
            overwrite_starts_inside_right_owned_crossfade(track, at_ms)
        {
            return Err(CutError::new(
                codes::CONFLICT,
                format!(
                    "overwrite start {at_ms}ms cuts through the first {xfade_ms}ms of right-owned crossfade clip '{clip_id}' on '{track_id}'"
                ),
                "splitting that transition head would shorten its realized overlap and move later rendered timeline coordinates",
            )
            .with_clip(clip_id)
            .with_at_ms(at_ms)
            .with_suggested_action(format!(
                "place the overwrite start at or after editorial {head_end}ms, or rebuild the transition deliberately in a separate edit"
            )));
        }
        if let Some((clip_id, xfade_ms)) =
            overwrite_fully_consumes_right_owned_crossfade(track, at_ms, overwrite_end)
        {
            return Err(CutError::new(
                codes::CONFLICT,
                format!(
                    "overwrite [{at_ms}, {overwrite_end})ms fully consumes right-owned crossfade clip '{clip_id}' on '{track_id}'"
                ),
                format!(
                    "removing that {xfade_ms}ms transition would lengthen the rendered timeline and move later coordinates",
                ),
            )
            .with_clip(clip_id)
            .with_at_ms(at_ms)
            .with_suggested_action(
                "place the overwrite edges outside the transition-owning clip, or rebuild the transition deliberately in a separate edit",
            ));
        }
        if let Some((clip_id, xfade_ms)) =
            truncated_right_owned_crossfade(track, at_ms, overwrite_end, duration_ms)
        {
            return Err(CutError::new(
                codes::CONFLICT,
                format!(
                    "overwrite end {overwrite_end}ms would truncate the {xfade_ms}ms right-owned crossfade on '{clip_id}' in '{track_id}'"
                ),
                "keeping that transition would require a longer inserted source and surviving right clip; shortening it would move later rendered timeline coordinates",
            )
            .with_clip(clip_id)
            .with_at_ms(overwrite_end)
            .with_suggested_action(
                "place the overwrite edge at a clip boundary, or choose a source range that leaves enough material on both sides of the transition",
            ));
        }
    }
    Ok(())
}

/// `xfade_in_ms` belongs to the right clip's head. When overwrite starts in
/// that head, `left_piece` would retain the transition on a fragment shorter
/// than its realized overlap. The EDL then clamps the overlap and shifts every
/// later rendered position, violating overwrite's no-ripple guarantee. The
/// full-head boundary is safe: the retained left piece is long enough to carry
/// the transition exactly.
fn overwrite_starts_inside_right_owned_crossfade(
    track: &Track,
    at_ms: u64,
) -> Option<(String, u64, u64)> {
    let mut cursor = 0u64;
    for (index, clip) in track.clips.iter().enumerate() {
        let end = cursor + clip.timeline_duration_ms();
        if let (Clip::Media(right), Some(Clip::Media(left))) =
            (clip, index.checked_sub(1).and_then(|i| track.clips.get(i)))
        {
            let xfade_ms = resolved_right_owned_xfade_ms(left, right);
            let head_end = cursor.saturating_add(xfade_ms);
            if xfade_ms > 0 && at_ms > cursor && at_ms < head_end {
                return Some((right.id.clone(), xfade_ms, head_end));
            }
        }
        cursor = end;
    }
    None
}

/// A right-owned crossfade cannot survive after its owner is removed. Dropping
/// it changes the EDL's overlap by the full realized transition length, so an
/// overwrite that covers the owner must fail before it mutates this or another
/// selected track.
fn overwrite_fully_consumes_right_owned_crossfade(
    track: &Track,
    at_ms: u64,
    overwrite_end: u64,
) -> Option<(String, u64)> {
    let mut cursor = 0u64;
    for (index, clip) in track.clips.iter().enumerate() {
        let end = cursor + clip.timeline_duration_ms();
        if at_ms <= cursor && overwrite_end >= end {
            if let (Clip::Media(right), Some(Clip::Media(left))) =
                (clip, index.checked_sub(1).and_then(|i| track.clips.get(i)))
            {
                let xfade_ms = resolved_right_owned_xfade_ms(left, right);
                if xfade_ms > 0 {
                    return Some((right.id.clone(), xfade_ms));
                }
            }
        }
        cursor = end;
    }
    None
}

/// An overwrite changes the predecessor of any right-owned transition at its
/// end. If it ends exactly at the right clip's head, that transition survives
/// in place; if it ends inside, the surviving right piece must carry it onto
/// the new inserted predecessor. In both cases the full realized overlap must
/// still fit the replacement; the inside case also needs enough surviving right
/// material. Otherwise the transition shortens and every later laid position
/// drifts, so reject before mutation.
fn truncated_right_owned_crossfade(
    track: &Track,
    at_ms: u64,
    overwrite_end: u64,
    replacement_duration_ms: u64,
) -> Option<(String, u64)> {
    let mut cursor = 0u64;
    for (index, clip) in track.clips.iter().enumerate() {
        let end = cursor + clip.timeline_duration_ms();
        let ends_at_right_head = at_ms < cursor && overwrite_end == cursor;
        // The right-piece helper preserves an incoming transition only when
        // this overwrite removed the clip's head. If its left piece survives,
        // that left piece remains the transition owner and the right piece must
        // stay hard-joined (the normal split behavior).
        let ends_inside_right = cursor >= at_ms && overwrite_end > cursor && overwrite_end < end;
        if ends_at_right_head || ends_inside_right {
            let (Clip::Media(right), Some(Clip::Media(left))) =
                (clip, index.checked_sub(1).and_then(|i| track.clips.get(i)))
            else {
                return None;
            };
            let xfade_ms = resolved_right_owned_xfade_ms(left, right);
            if xfade_ms > 0
                && (xfade_ms > replacement_duration_ms
                    || (ends_inside_right && xfade_ms > end - overwrite_end))
            {
                return Some((right.id.clone(), xfade_ms));
            }
            return None;
        }
        cursor = end;
    }
    None
}

/// The same effective incoming overlap the EDL realizes for an adjacent plain
/// media pair. Keeping this local to overwrite makes its no-ripple validation
/// use the exact clip durations that `right_piece` will leave behind.
fn resolved_right_owned_xfade_ms(left: &MediaClip, right: &MediaClip) -> u64 {
    right
        .xfade_in_ms
        .min(Clip::Media(left.clone()).timeline_duration_ms())
        .min(Clip::Media(right.clone()).timeline_duration_ms())
}
/// Return the state that makes a mid-clip overwrite unsafe. Each item is
/// expressed in the source clip's *whole* duration or its clip-local timeline;
/// cloning it onto shortened pieces would silently change playback. Constant
/// speed, fades, and source-time mute ranges are handled correctly by the split
/// helpers below, so they deliberately remain allowed.
fn edge_splits_time_dependent_clip(track: &Track, edge: u64) -> Option<(&str, &'static str)> {
    let mut cursor = 0u64;
    for clip in &track.clips {
        let end = cursor + clip.timeline_duration_ms();
        if edge > cursor && edge < end {
            if let Clip::Media(media) = clip {
                let feature = if media.speed_ramp.is_some() {
                    Some("a variable-speed ramp")
                } else if media.reverse {
                    Some("reverse playback")
                } else if media.freeze.is_some() {
                    Some("a freeze-frame")
                } else if media.animation.is_some() {
                    Some("a Ken Burns animation")
                } else if !media.keyframes.is_empty() {
                    Some("parameter keyframes")
                } else {
                    None
                };
                if let Some(feature) = feature {
                    return Some((media.id.as_str(), feature));
                }
            }
        }
        cursor = end;
    }
    None
}
fn needs_right_split(track: &Track, at_ms: u64, overwrite_end: u64) -> bool {
    let mut cursor = 0u64;
    for clip in &track.clips {
        let end = cursor + clip.timeline_duration_ms();
        if cursor < at_ms && end > overwrite_end {
            return matches!(clip, Clip::Media(_));
        }
        cursor = end;
    }
    false
}
#[allow(clippy::too_many_arguments)]
fn overwrite_track(
    project: &mut Project,
    original: &Track,
    asset: &str,
    at_ms: u64,
    src_range_ms: [u64; 2],
    added_id: String,
    split_id: Option<String>,
) -> OpEffect {
    let overwrite_end = at_ms + (src_range_ms[1] - src_range_ms[0]);
    let old_end = original.duration_ms();
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut cursor = 0u64;
    let mut overlapped_clip_ids = Vec::new();
    let mut removed_clip_ids = Vec::new();
    let mut overwritten_gap_ms = 0u64;
    let mut used_split = None;
    for clip in &original.clips {
        let end = cursor + clip.timeline_duration_ms();
        if end <= at_ms {
            before.push(clip.clone());
        } else if cursor >= overwrite_end {
            after.push(clip.clone());
        } else {
            if let Some(id) = clip.id() {
                overlapped_clip_ids.push(id.to_string());
                if cursor >= at_ms && end <= overwrite_end {
                    removed_clip_ids.push(id.to_string());
                }
            }
            if let Clip::Gap(_) = clip {
                overwritten_gap_ms += end.min(overwrite_end) - cursor.max(at_ms);
            }
            if cursor < at_ms {
                before.push(left_piece(clip, at_ms - cursor));
            }
            if end > overwrite_end {
                let right_id = if cursor < at_ms {
                    let id = split_id.clone().expect("single-clip split was planned");
                    used_split = Some(id.clone());
                    Some(id)
                } else {
                    None
                };
                // If the overwrite consumed this clip's head, carry its
                // right-owned transition to the surviving piece. Validation
                // proved its full realized overlap still fits. If the left
                // piece survives instead, it remains the sole owner and the
                // right piece must be hard-joined like every normal split.
                after.push(right_piece(
                    clip,
                    overwrite_end - cursor,
                    right_id,
                    cursor >= at_ms,
                ));
            }
        }
        cursor = end;
    }
    let tail_gap_ms = at_ms.saturating_sub(old_end);
    if tail_gap_ms > 0 {
        before.push(Clip::Gap(GapClip::new(tail_gap_ms)));
    }
    before.push(Clip::Media(make_media_clip(
        &added_id,
        asset,
        src_range_ms[0],
        src_range_ms[1],
    )));
    before.extend(after);
    project
        .track_mut(&original.id)
        .expect("validated target")
        .clips = before;

    let existing_end = old_end.min(overwrite_end).max(at_ms);
    let mut detail = json!({
        "added_clip": added_id,
        "added_ms": [at_ms, overwrite_end],
        "src_range_ms": src_range_ms,
        "overwrite": true,
        "overwritten_existing_ms": [at_ms, existing_end],
        "overlapped_clip_ids": overlapped_clip_ids,
        "removed_clip_ids": removed_clip_ids,
        "overwritten_gap_ms": overwritten_gap_ms,
        "tail_gap_ms": tail_gap_ms,
        "tail_extended_ms": overwrite_end.saturating_sub(old_end),
    });
    if let Some(id) = used_split {
        detail["split_clip"] = json!(id);
    }
    fx(Some(&original.id), detail)
}

fn left_piece(clip: &Clip, timeline_ms: u64) -> Clip {
    match clip {
        Clip::Gap(gap) => Clip::Gap(GapClip::new(timeline_ms.min(gap.duration_ms))),
        Clip::Media(media) => {
            let mut left = media.clone();
            left.src_out_ms = media.src_in_ms + tl_off_to_src(timeline_ms, media.speed);
            let (left_fade, _) = split_fade(&media.fade);
            left.fade = left_fade;
            Clip::Media(left)
        }
        Clip::Caption(_) => unreachable!("caption tracks are rejected before overwrite"),
    }
}

fn right_piece(
    clip: &Clip,
    timeline_offset_ms: u64,
    replacement_id: Option<String>,
    preserve_incoming_xfade: bool,
) -> Clip {
    match clip {
        Clip::Gap(gap) => Clip::Gap(GapClip::new(gap.duration_ms - timeline_offset_ms)),
        Clip::Media(media) => {
            let mut right: MediaClip = media.clone();
            if let Some(id) = replacement_id {
                right.id = id;
            }
            right.src_in_ms = media.src_in_ms + tl_off_to_src(timeline_offset_ms, media.speed);
            let (_, right_fade) = split_fade(&media.fade);
            right.fade = right_fade;
            if !preserve_incoming_xfade {
                right.xfade_in_ms = 0;
                right.xfade_kind = None;
            }
            Clip::Media(right)
        }
        Clip::Caption(_) => unreachable!("caption tracks are rejected before overwrite"),
    }
}

fn split_fade(fade: &Option<ClipFade>) -> (Option<ClipFade>, Option<ClipFade>) {
    match fade {
        None => (None, None),
        Some(fade) => (
            (fade.in_ms > 0).then_some(ClipFade {
                in_ms: fade.in_ms,
                out_ms: 0,
                kind: fade.kind,
            }),
            (fade.out_ms > 0).then_some(ClipFade {
                in_ms: 0,
                out_ms: fade.out_ms,
                kind: fade.kind,
            }),
        ),
    }
}

#[cfg(test)]
mod tests;
