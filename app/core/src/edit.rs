//! edit.rs — timeline mutation primitives (public verb contract "edit" domain).
//!
//! Role: the ONLY code allowed to mutate a Project's timeline. Each function
//! mutates in place and returns the OpEffects describing what changed; the
//! caller (store::ProjectStore::apply / store::apply_record) wraps them into
//! an OpRecord and appends to the log — every mutation goes
//! through a verb, every verb emits an op record.
//!
//! Time model invariants relied on throughout (timeline/op-log contract):
//! - video/audio tracks: clips are ordered + non-overlapping; timeline
//!   position is CUMULATIVE (sum of preceding clip durations). Removing or
//!   inserting duration automatically ripples everything after it.
//! - caption tracks: clip `range_ms` is ABSOLUTE timeline time, so ripple
//!   operations must remap caption ranges explicitly.
//! - markers: absolute timeline time; remapped on timeline-wide ripples only
//!   (single-track ripples don't move the other tracks the markers annotate).
//!
//! Undo model: current records are recomputed from the immutable log. A tip
//! restore rebuilds the prefix before its target, applies that timeline, and
//! records the materialized result in its own effect; this is why it refuses a
//! non-tip target that would discard later edits. Selective non-tip restore
//! uses the guarded id-pinned rebase path. `restore()` below remains solely for
//! replaying historic snapshot-era restore records.
//!
//! Dependencies: types.rs, ops.rs, error.rs. Primary callers: store.rs,
//! server verb dispatch, transcript verbs (cut_words lowers to ripple_delete).

use crate::error::{codes, CutError};
use crate::ops::OpEffect;
use crate::types::{
    Clip, ClipFade, FadeKind, GainWindow, GapClip, Marker, MediaClip, Nest, Project, Track,
    TrackKind,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/// Build an OpEffect from a track id + a JSON object of op-specific detail.
/// Non-object detail becomes an empty map (callers always pass json!({..})).
pub(crate) fn fx(track: Option<&str>, detail: serde_json::Value) -> OpEffect {
    let map = match detail {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    OpEffect {
        track: track.map(String::from),
        detail: map,
    }
}

/// Highest N across existing "cN" clip ids (0 if none). Id allocation during
/// mutation precomputes `max_clip_n + 1, +2, ...` so we never need an
/// immutable &Project while holding a &mut Track borrow.
fn max_clip_n(project: &Project) -> u64 {
    // Scan BOTH the live tracks AND every nest's sub-timeline: a clip moved into a
    // nest (edit.nest) lives only in `project.nests`, so excluding it would let a
    // later allocation re-mint a buried clip's id and collide on un-nest. Pre-nest
    // projects have no nests → this chain is a no-op (byte-identical id allocation).
    let track_clips = project.tracks.iter().flat_map(|t| t.clips.iter());
    let nest_clips = project
        .nests
        .iter()
        .flat_map(|n| n.tracks.iter())
        .flat_map(|t| t.clips.iter());
    track_clips
        .chain(nest_clips)
        .filter_map(|c| c.id())
        .filter_map(|id| id.strip_prefix('c').and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
}

/// Next free "nestN" id (max existing nest index + 1). Deterministic (pure function
/// of project state) so replay allocates the exact same id the live path did.
fn next_nest_id(project: &Project) -> String {
    let n = project
        .nests
        .iter()
        .filter_map(|nx| {
            nx.id
                .strip_prefix("nest")
                .and_then(|x| x.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("nest{}", n + 1)
}

/// Helper used by insert/split implementations: next free "cN" clip id.
/// Deterministic (pure function of project state) — replay allocates the
/// exact same ids the live path did. Kept public so transcript verbs reuse it.
pub fn new_clip_id(project: &Project) -> String {
    format!("c{}", max_clip_n(project) + 1)
}

/// Convenience constructor for a media clip (used by insert + tests).
pub fn make_media_clip(id: &str, asset: &str, src_in_ms: u64, src_out_ms: u64) -> MediaClip {
    MediaClip {
        id: id.to_string(),
        asset: asset.to_string(),
        src_in_ms,
        src_out_ms,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
    }
}

/// Distribute a clip's fade across its two halves when the clip is split:
/// the LEFT half owns the original beginning (keeps the fade-in), the RIGHT
/// half owns the original ending (keeps the fade-out); the new boundary is a
/// hard cut. A half whose fade collapses to 0/0 carries None. Shared by
/// split(), ripple_delete() and splice_into_track() — every site that cuts a
/// media clip in two (a PiP transform is copied to both halves; a fade must
/// NOT be, or a mid-clip split would invent a dip to black/silence).
fn split_fade(fade: &Option<ClipFade>) -> (Option<ClipFade>, Option<ClipFade>) {
    match fade {
        None => (None, None),
        Some(f) => (
            (f.in_ms > 0).then_some(ClipFade {
                in_ms: f.in_ms,
                out_ms: 0,
                kind: f.kind,
            }),
            (f.out_ms > 0).then_some(ClipFade {
                in_ms: 0,
                out_ms: f.out_ms,
                kind: f.kind,
            }),
        ),
    }
}

/// Ripple time-map: where does absolute time `t` land after removing [a, b)?
/// Points inside the removed range clamp to `a`.
fn ripple_map(t: u64, a: u64, b: u64) -> u64 {
    if t <= a {
        t
    } else if t < b {
        a
    } else {
        t - (b - a)
    }
}

/// Insert time-map (the mirror of `ripple_map`): where does absolute time `t`
/// land after inserting `d` ms at `at`? A point exactly AT `at` shifts right —
/// it annotates the content that begins there, and that content moves. Both
/// maps follow content, not coordinates (content-relative-marker contract).
fn insert_map(t: u64, at: u64, d: u64) -> u64 {
    if t < at {
        t
    } else {
        t + d
    }
}

/// Timeline interval [t0, t1) of clip index `i` on a cumulative track.
fn clip_span(track: &Track, i: usize) -> (u64, u64) {
    let t0: u64 = track.clips[..i]
        .iter()
        .map(|c| c.timeline_duration_ms())
        .sum();
    (t0, t0 + track.clips[i].timeline_duration_ms())
}

// ---------------------------------------------------------------------------
// edit verbs
// ---------------------------------------------------------------------------

/// Split the clip under `at_ms` on `track` into two clips at that point.
/// Returns effects naming both resulting clip ids. Errors `not_found` if the
/// position is in a gap, on a boundary, or past the end (with at_ms context).
pub fn split(project: &mut Project, track: &str, at_ms: u64) -> Result<Vec<OpEffect>, CutError> {
    split_pinned(project, track, at_ms, None)
}

/// Id-pinning variant of [`split`]: `pinned_right` forces the right-half clip
/// id instead of allocating positionally (`new_clip_id`). The LIVE path calls
/// `split` (pinned = None ⇒ positional alloc, recorded in the effect); the
/// REPLAY / skip-replay path passes the id the op RECORDED so the timeline's
/// id graph stays stable even when an earlier op was skipped (rebase.rs).
/// When pinned is Some, allocation order no longer matters — the recorded id is
/// authoritative. Determinism note: in the NO-SKIP case the pinned id EQUALS
/// what positional alloc would mint, so replay stays byte-identical.
pub fn split_pinned(
    project: &mut Project,
    track: &str,
    at_ms: u64,
    pinned_right: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let right_id = match pinned_right {
        Some(id) => id.to_string(),
        None => new_clip_id(project), // allocate before the &mut borrow
    };
    let t = project
        .track_mut(track)
        .ok_or_else(|| track_not_found(track))?;
    if t.kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "cannot split a caption track",
            "caption clips are managed via captions.* verbs",
        ));
    }
    let mut cursor: u64 = 0;
    for i in 0..t.clips.len() {
        let dur = t.clips[i].timeline_duration_ms();
        let (t0, t1) = (cursor, cursor + dur);
        if at_ms > t0 && at_ms < t1 {
            return match &mut t.clips[i] {
                Clip::Media(c) => {
                    // A variable-speed ramp makes the timeline→source map NON-LINEAR
                    // (`tl_off_to_src` below assumes a single scalar speed), and the
                    // ramp's source-offset control points would be invalidated by the
                    // new window — refuse the split rather than mis-cut it.
                    if c.speed_ramp.is_some() {
                        return Err(ramp_conflict(&c.id, "split"));
                    }
                    let off = at_ms - t0;
                    // `off` is a TIMELINE offset into the clip; the SOURCE split
                    // point is that offset scaled by the clip's speed (a 2× clip
                    // covers 2 ms of source per 1 ms of timeline). Identity at
                    // speed 1.0, so unsped splits are byte-identical.
                    let src_off = crate::types::tl_off_to_src(off, c.speed);
                    let (left_fade, right_fade) = split_fade(&c.fade);
                    let right = MediaClip {
                        id: right_id.clone(),
                        asset: c.asset.clone(),
                        src_in_ms: c.src_in_ms + src_off,
                        src_out_ms: c.src_out_ms,
                        effects: c.effects.clone(),
                        gain_db: c.gain_db,
                        transform: c.transform.clone(),
                        // Crop is a source-frame property: both halves keep
                        // the same kept rectangle (a split changes the time
                        // range, not which source pixels are visible).
                        crop: c.crop.clone(),
                        fade: right_fade,
                        // Crossfade-in belongs to the clip's original START
                        // (the cut point it dissolves from), so it stays on the
                        // LEFT half — the right half starts with a hard cut from
                        // its left sibling (the original mid-clip split point).
                        xfade_in_ms: 0,
                        xfade_kind: None,
                        // Both halves of a split play at the parent's speed.
                        speed: c.speed,
                        // ...and keep the same color grade (a split changes the
                        // time range, not the look).
                        grade: c.grade.clone(),
                        // ...and the same matte (same source pixels → same alpha;
                        // both halves reuse the parent's baked cache).
                        matte: c.matte.clone(),
                        mask: c.mask.clone(),
                        // Both halves play in the same direction as the parent.
                        reverse: c.reverse,
                        freeze: c.freeze.clone(),
                        animation: c.animation.clone(),
                        keyframes: c.keyframes.clone(),
                        eq: c.eq.clone(),
                        // Mute ranges are SOURCE-time: both halves keep the parent's
                        // list verbatim; each renders only the overlap with its own
                        // visible window — the muted words stay muted, unsplit.
                        mute_ranges: c.mute_ranges.clone(),
                        stabilize: c.stabilize.clone(),
                        // A speed ramp's points are SOURCE offsets relative to the
                        // ORIGINAL window; a split moves the window, invalidating
                        // them — so split refuses a ramped clip (guarded above) and
                        // the halves never carry a ramp.
                        speed_ramp: None,
                        // Both halves are the same source pixels → same input color
                        // space tag (a split changes the time range, not the source).
                        input_color_space: c.input_color_space,
                        nest: c.nest.clone(),
                        grade_stack: c.grade_stack.clone(),
                        grade_windows: c.grade_windows.clone(),
                    };
                    let left_id = c.id.clone();
                    c.src_out_ms = c.src_in_ms + src_off;
                    c.fade = left_fade;
                    // left half keeps c.xfade_in_ms unchanged (its start is the
                    // original crossfade boundary).
                    t.clips.insert(i + 1, Clip::Media(right));
                    Ok(vec![fx(
                        Some(track),
                        json!({"split_at_ms": at_ms, "left": left_id, "right": right_id}),
                    )])
                }
                Clip::Gap(_) => Err(CutError::new(
                    codes::NOT_FOUND,
                    format!("position {at_ms}ms is inside a gap on '{track}'"),
                    "gaps have no content to split",
                )
                .with_at_ms(at_ms)),
                Clip::Caption(_) => Err(caption_on_non_caption_track(track)),
            };
        }
        cursor = t1;
    }
    Err(CutError::new(
        codes::NOT_FOUND,
        format!("no clip under {at_ms}ms on '{track}'"),
        "position is on a clip boundary or past the end of the track",
    )
    .with_at_ms(at_ms)
    .with_suggested_action("use project.state to inspect clip positions"))
}

/// Remove timeline range `range_ms` — on one track, or all tracks when `track`
/// is None (keeps AV in sync). `ripple` selects the NLE edit kind:
/// - `ripple = true` (DEFAULT): close the gap (ripple/extract) — later content
///   shifts left, captions/markers/duck-windows remap. This is the legacy behavior, so
///   ops logged without the flag replay byte-identical.
/// - `ripple = false`: LEAVE a gap of equal length where the content was (lift)
///   — nothing downstream moves, captions/markers/duck-windows stay put. The
///   "remove this clip, keep everyone else's timing" edit.
///
/// Caption clips overlapping the range are trimmed/shifted (ripple) or left in
/// place (lift). Markers ripple only on the all-tracks ripple form (a single-
/// track ripple leaves the rest of the timeline — which markers annotate — in
/// place; a lift never moves them).
pub fn ripple_delete(
    project: &mut Project,
    track: Option<&str>,
    range_ms: [u64; 2],
    ripple: bool,
) -> Result<Vec<OpEffect>, CutError> {
    ripple_delete_pinned(project, track, range_ms, ripple, None)
}

/// Id-pinning variant of [`ripple_delete`]. A range EDGE can cut a clip whose
/// BOTH halves survive (the range sits strictly inside one clip); the right
/// half gets a freshly allocated `cN` id. `pinned_split_clips` supplies those
/// ids IN ALLOCATION ORDER (one per surviving-both-halves split, in track order
/// then clip order) so a skip-replay keeps them stable (rebase.rs); each minted
/// split is recorded in its track's effect as `split_clip`. Live path passes
/// None ⇒ positional `c{next_n}` allocation exactly as before. In the no-skip
/// case the pinned ids equal the positional ones (byte-identical replay).
pub fn ripple_delete_pinned(
    project: &mut Project,
    track: Option<&str>,
    range_ms: [u64; 2],
    ripple: bool,
    pinned_split_clips: Option<&[String]>,
) -> Result<Vec<OpEffect>, CutError> {
    let [a, b] = range_ms;
    if a >= b {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("range_ms [{a}, {b}) is empty or inverted"),
            "range start must be strictly less than range end",
        ));
    }
    // Cursor into pinned_split_clips: each surviving-both-halves split consumes
    // the next pinned id (replay path) or allocates positionally (live path).
    let mut pinned_cursor = 0usize;
    // Validate target track up front so we never half-mutate on bad input.
    let target_idx: Vec<usize> = match track {
        Some(id) => {
            let i = project
                .tracks
                .iter()
                .position(|t| t.id == id)
                .ok_or_else(|| track_not_found(id))?;
            vec![i]
        }
        None => (0..project.tracks.len()).collect(),
    };
    let mut next_n = max_clip_n(project) + 1; // ids for clips split by the range edge
    let mut effects = Vec::new();
    // Refuse non-linear retimes before draining/rebuilding any track.
    for &ti in &target_idx {
        let t = &project.tracks[ti];
        if t.kind == TrackKind::Caption {
            continue;
        }
        let mut cursor: u64 = 0;
        for clip in &t.clips {
            let dur = clip.timeline_duration_ms();
            let (t0, t1) = (cursor, cursor + dur);
            cursor = t1;
            if t1 <= a || t0 >= b {
                continue;
            }
            if let Clip::Media(c) = clip {
                if c.speed_ramp.is_some() {
                    return Err(ramp_conflict(&c.id, "edit.ripple_delete"));
                }
            }
        }
    }
    for ti in target_idx {
        let t = &mut project.tracks[ti];
        let track_id = t.id.clone();
        if t.kind == TrackKind::Caption {
            // Absolute ranges. RIPPLE: remap both edges, drop captions that
            // collapse. LIFT (ripple=false): caption timings are absolute and
            // a lift moves nothing — leave every caption clip untouched (the
            // hole is on the media tracks; captions over it simply stay).
            let mut removed_ids = Vec::new();
            if ripple {
                t.clips.retain_mut(|c| {
                    if let Clip::Caption(cc) = c {
                        let ns = ripple_map(cc.range_ms[0], a, b);
                        let ne = ripple_map(cc.range_ms[1], a, b);
                        if ne <= ns {
                            removed_ids.push(cc.id.clone());
                            return false;
                        }
                        cc.range_ms = [ns, ne];
                    }
                    true
                });
            }
            effects.push(fx(
                Some(&track_id),
                json!({"removed_ms": [a, b], "clips_removed": removed_ids}),
            ));
            continue;
        }
        // Cumulative tracks: rebuild the clip list, cutting out [a, b).
        // RIPPLE: shrinking earlier durations ripples later clips left
        // automatically (gap closes). LIFT (ripple=false): a GapClip of the
        // removed overlap is pushed in place of the cut content, so every
        // later clip keeps its timeline position (the hole stays open).
        let mut cursor: u64 = 0;
        let mut new_clips: Vec<Clip> = Vec::new();
        let mut removed_ids: Vec<String> = Vec::new();
        // The range-edge split id minted on THIS track (at most one per track:
        // only a clip strictly containing [a,b) survives as both halves). None
        // when no clip was split here. Recorded as `split_clip` in the effect.
        let mut split_clip_id: Option<String> = None;
        for clip in t.clips.drain(..) {
            let dur = clip.timeline_duration_ms();
            let (t0, t1) = (cursor, cursor + dur);
            cursor = t1;
            if t1 <= a || t0 >= b {
                new_clips.push(clip); // untouched (before or after the range)
                continue;
            }
            // The portion of THIS clip that the cut removes (for the lift gap).
            let cut_overlap = t1.min(b) - t0.max(a);
            match clip {
                Clip::Media(c) => {
                    let keeps_left = t0 < a;
                    let keeps_right = t1 > b;
                    // Fade ownership (edit.fade): the left remnant keeps the
                    // fade-in, the right remnant the fade-out — a removed
                    // beginning/ending takes its fade with it.
                    let (left_fade, right_fade) = split_fade(&c.fade);
                    if keeps_left {
                        let mut left = c.clone();
                        left.src_out_ms =
                            c.src_in_ms + crate::types::tl_off_to_src(a - t0, c.speed);
                        left.fade = left_fade;
                        new_clips.push(Clip::Media(left));
                    }
                    // LIFT: backfill the removed span with a gap so positions
                    // downstream don't shift. (RIPPLE drops the span entirely.)
                    if !ripple && cut_overlap > 0 {
                        new_clips.push(Clip::Gap(GapClip::new(cut_overlap)));
                    }
                    if keeps_right {
                        let mut right = c.clone();
                        right.src_in_ms =
                            c.src_in_ms + crate::types::tl_off_to_src(b - t0, c.speed);
                        right.fade = right_fade;
                        if keeps_left {
                            // Both halves survive → the right half is a new clip.
                            // Pin the id from pinned_split_clips (replay path) or
                            // allocate positionally (live path); record it so a
                            // future skip-replay can pin it (rebase.rs).
                            let new_id = match pinned_split_clips {
                                Some(ids) if pinned_cursor < ids.len() => {
                                    let id = ids[pinned_cursor].clone();
                                    pinned_cursor += 1;
                                    id
                                }
                                _ => {
                                    let id = format!("c{next_n}");
                                    next_n += 1;
                                    id
                                }
                            };
                            right.id = new_id.clone();
                            split_clip_id = Some(new_id);
                        }
                        // A crossfade-in on the right remnant survived from the
                        // original clip; but its left neighbour is now a gap
                        // (lift) or different content — clear it so we never
                        // dissolve from a gap (the renderer would skip it, but
                        // keep the data model honest).
                        if !ripple {
                            right.xfade_in_ms = 0;
                            right.xfade_kind = None;
                        }
                        new_clips.push(Clip::Media(right));
                    }
                    if !keeps_left && !keeps_right {
                        removed_ids.push(c.id);
                    }
                }
                Clip::Gap(g) => {
                    if ripple {
                        // Gaps just lose the overlapped duration (merge freely).
                        let remain = g.duration_ms - cut_overlap;
                        if remain > 0 {
                            new_clips.push(Clip::Gap(GapClip::new(remain)));
                        }
                    } else {
                        // LIFT over a gap: the gap was already empty time —
                        // keep its full length (nothing to remove, positions
                        // must not shift).
                        new_clips.push(Clip::Gap(g));
                    }
                }
                Clip::Caption(_) => return Err(caption_on_non_caption_track(&t.id)),
            }
        }
        t.clips = new_clips;
        // Gain windows (edit.duck) carry ABSOLUTE timeline ranges. RIPPLE
        // remaps both edges with the track's content and drops collapsed
        // windows; LIFT moves nothing (positions unchanged), so windows stay.
        if ripple {
            t.gain_windows.retain_mut(|w| {
                let ns = ripple_map(w.range_ms[0], a, b);
                let ne = ripple_map(w.range_ms[1], a, b);
                if ne <= ns {
                    return false;
                }
                w.range_ms = [ns, ne];
                true
            });
        }
        let mut detail =
            json!({"removed_ms": [a, b], "clips_removed": removed_ids, "ripple": ripple});
        if let Some(sc) = &split_clip_id {
            detail["split_clip"] = json!(sc);
        }
        effects.push(fx(Some(&track_id), detail));
    }
    // Timeline-wide ripple moves the markers with the content they annotate;
    // a lift leaves the hole and every marker where it was.
    if track.is_none() && ripple {
        for m in &mut project.markers {
            m.at_ms = ripple_map(m.at_ms, a, b);
        }
    }
    Ok(effects)
}

/// Adjust a clip's source in/out points (either or both). Timeline ripples
/// to match the new duration (cumulative model). Errors if the new range is
/// empty/inverted or outside the asset duration (when the asset is probed).
pub fn trim(
    project: &mut Project,
    clip_id: &str,
    src_in_ms: Option<u64>,
    src_out_ms: Option<u64>,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    let (old, new_in, new_out, asset) = match &mut t.clips[idx] {
        Clip::Media(c) => {
            // A trim moves the clip's source window; a speed ramp's control points
            // are offsets INTO that window, so trimming would silently invalidate
            // them — refuse it (clear the ramp, trim, then re-ramp).
            if c.speed_ramp.is_some() {
                return Err(ramp_conflict(clip_id, "trim"));
            }
            let old = [c.src_in_ms, c.src_out_ms];
            let new_in = src_in_ms.unwrap_or(c.src_in_ms);
            let new_out = src_out_ms.unwrap_or(c.src_out_ms);
            if new_in >= new_out {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("trim would make clip '{clip_id}' empty ([{new_in}, {new_out}))"),
                    "src_in_ms must be strictly less than src_out_ms",
                )
                .with_clip(clip_id));
            }
            c.src_in_ms = new_in;
            c.src_out_ms = new_out;
            (old, new_in, new_out, c.asset.clone())
        }
        _ => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("'{clip_id}' is not a media clip"),
                "trim adjusts media source ranges; captions are edited via captions.* verbs",
            )
            .with_clip(clip_id))
        }
    };
    // Validate against probe AFTER the (cheap) set; on violation the caller's
    // transactional apply (clone-then-swap in store.rs) discards the mutation.
    if let Some(d) = project
        .assets
        .get(&asset)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("duration_ms"))
        .and_then(|v| v.as_u64())
    {
        if new_out > d {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("src_out_ms {new_out} exceeds asset duration {d}ms"),
                format!("asset '{asset}' is only {d}ms long"),
            )
            .with_clip(clip_id));
        }
    }
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_src_ms": old, "new_src_ms": [new_in, new_out]}),
    )])
}

/// Set a media clip's playback speed factor (edit.speed; retime / slow-mo).
/// The source range [src_in,src_out) is UNCHANGED — only how it lays onto the
/// timeline (`Clip::timeline_duration_ms` divides by speed, so later clips
/// ripple automatically) and how the renderer time-stretches it. factor 1.0
/// CLEARS the retime (back to normal, serde-skipped). Range (0.25–4.0) is
/// validated by the dispatch layer before commit; core trusts the value so
/// replay reproduces it verbatim. Errors if the target is not a media clip
/// (gaps/captions have no source to stretch).
pub fn speed(project: &mut Project, clip_id: &str, factor: f64) -> Result<Vec<OpEffect>, CutError> {
    if !(factor.is_finite() && factor > 0.0) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("speed factor must be finite and > 0, got {factor}"),
            "edit.speed retimes by dividing the source duration by factor",
        )
        .with_clip(clip_id)
        .with_suggested_action("pass a positive finite speed factor, e.g. 0.5, 1.0, or 2.0"));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    let (old_factor, new_dur, keyframes_rescaled) = match &mut t.clips[idx] {
        Clip::Media(c) => {
            // Constant speed and a variable-speed ramp both claim the clip's
            // timeline length — they are mutually exclusive. Clear the ramp first.
            if c.speed_ramp.is_some() {
                return Err(ramp_conflict(clip_id, "edit.speed (constant retime)"));
            }
            let new_dur =
                crate::types::src_off_to_tl(c.src_out_ms.saturating_sub(c.src_in_ms), factor);
            let old_dur =
                crate::types::src_off_to_tl(c.src_out_ms.saturating_sub(c.src_in_ms), c.speed);
            let mut keyframes_rescaled = 0usize;
            if old_dur > 0 && old_dur != new_dur {
                for track in &mut c.keyframes {
                    for point in &mut track.points {
                        point.t_ms = (((point.t_ms as u128) * (new_dur as u128)
                            + (old_dur as u128 / 2))
                            / old_dur as u128)
                            .min(new_dur as u128) as u64;
                        keyframes_rescaled += 1;
                    }
                }
            }
            let old_factor = c.speed;
            c.speed = factor;
            (old_factor, new_dur, keyframes_rescaled)
        }
        _ => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("'{clip_id}' is not a media clip"),
                "edit.speed retimes media clips; gaps and captions have no source to stretch",
            )
            .with_clip(clip_id))
        }
    };
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip": clip_id,
            "old_factor": old_factor,
            "factor": factor,
            "new_timeline_duration_ms": new_dur,
            "keyframes_rescaled": keyframes_rescaled,
        }),
    )])
}
/// Set (or CLEAR) a media clip's VARIABLE speed ramp (edit.speed_ramp) — the
/// variable-speed curve. `points` (≥ 2, source-offset/factor; see [`crate::types::SpeedRampPoint`]) describe a piecewise-linear speed curve over
/// the clip's source window; `segments` is the sampling granularity (resolved + clamped by the dispatch layer, recorded so replay is default-independent). An
/// EMPTY `points` list CLEARS the ramp (back to constant speed). The realized timeline length is the integral of (1/speed) over the source = the sum of the
/// expanded sub-segment durations (`speed_ramp_segments`), so later clips ripple automatically (the cursor reads `timeline_duration_ms`).
///
/// The clip must be PLAIN enough for the sub-segmentation to be correct: no
/// constant `speed` ≠ 1 (mutually exclusive retime), and none of the time-warping
/// / per-frame-baked features whose semantics the split would break — reverse,
/// freeze, animation, keyframes, matte, mask, stabilize. (Time-INVARIANT looks —
/// gain, grade, effects, eq, crop, transform — are fine and are carried onto every
/// sub-segment by the EDL.) The values themselves (≥2 points, factor range, sorted
/// offsets) are validated by the dispatch layer before commit; core trusts them so
/// replay reproduces the curve verbatim.
pub fn speed_ramp(
    project: &mut Project,
    clip_id: &str,
    points: Vec<crate::types::SpeedRampPoint>,
    segments: usize,
    timebase_fps: Option<f64>,
    timebase_audio_rate: Option<u32>,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    // New ops persist their grid; absent fields retain historical project-fps clamping and ms semantics.
    let timebase_fps = timebase_fps.filter(|fps| fps.is_finite() && *fps > 0.0);
    let timebase_audio_rate = timebase_fps.and(timebase_audio_rate.filter(|rate| *rate > 0));
    let fps = timebase_fps.unwrap_or(project.settings.fps.max(1.0));
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut t.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "edit.speed_ramp retimes media clips; gaps and captions have no source to remap",
        )
        .with_clip(clip_id));
    };
    // CLEAR path: empty points removes the ramp (always allowed, even on a clip
    // that otherwise carries conflicting features — clearing can only help).
    if points.is_empty() {
        let old = c.speed_ramp.take();
        return Ok(vec![fx(
            Some(&track_id),
            json!({
                "clip": clip_id,
                "cleared": true,
                "had_ramp": old.is_some(),
            }),
        )]);
    }
    // SET path: the clip must be plain enough for sub-segmentation to be correct.
    if (c.speed - 1.0).abs() > f64::EPSILON {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "clip '{clip_id}' already has a constant speed ({:.3}×); a ramp replaces it",
                c.speed
            ),
            "constant speed and a variable-speed ramp both set the clip's timeline length",
        )
        .with_clip(clip_id)
        .with_suggested_action(
            "reset to normal speed first (edit.speed{clip, factor:1}), then add the ramp",
        ));
    }
    let blocker = if c.reverse {
        Some("reverse")
    } else if c.freeze.is_some() {
        Some("a freeze-frame")
    } else if c.animation.is_some() {
        Some("a Ken Burns animation")
    } else if !c.keyframes.is_empty() {
        Some("parameter keyframes")
    } else if c.matte.is_some() {
        Some("a background matte")
    } else if c.mask.is_some() {
        Some("a region mask/redaction")
    } else if c.stabilize.is_some() {
        Some("stabilization")
    } else {
        None
    };
    if let Some(feature) = blocker {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("clip '{clip_id}' has {feature}, which is incompatible with a speed ramp"),
            "a speed ramp re-renders the clip as constant-speed sub-segments; that breaks \
             time-warping / per-frame-baked features keyed to the whole clip",
        )
        .with_clip(clip_id)
        .with_suggested_action(format!(
            "clear {feature} on '{clip_id}' first, then add the ramp (or ramp a copy without it)"
        )));
    }
    let old = c.speed_ramp.clone();
    let preferred_segments = segments.clamp(
        crate::types::MIN_RAMP_SEGMENTS,
        crate::types::MAX_RAMP_SEGMENTS,
    );
    let mut ramp = crate::types::SpeedRamp {
        points,
        segments: preferred_segments,
        preferred_segments: timebase_fps.map(|_| preferred_segments),
        timebase_fps,
        timebase_audio_rate,
    };
    ramp.segments =
        crate::speed_ramp_timing::clamp_speed_ramp_segments(c.src_in_ms, c.src_out_ms, &ramp, fps);
    let new_dur: u64 = crate::types::speed_ramp_segments(c.src_in_ms, c.src_out_ms, &ramp)
        .iter()
        .map(|s| s.dur_ms)
        .sum();
    let n_points = ramp.points.len();
    c.speed_ramp = Some(ramp.clone());
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip": clip_id,
            "cleared": false,
            "points": ramp.points,
            "segments": ramp.segments,
            "n_points": n_points,
            "new_timeline_duration_ms": new_dur,
            "old_ramp": old,
        }),
    )])
}

/// Remove trailing GAP clips from a track — empty space AFTER the last real clip.
///
/// A gap BETWEEN clips holds the later clips in position and must stay; a gap at the
/// END is pure black tail. `Track::duration_ms` sums EVERY clip (gaps included), so a
/// dangling trailing gap inflates the track's length → the EDL extent runs past the
/// content and playback/render show a black tail. Ops that vacate a slot
/// (move/delete) gap-fill it; when that slot was the track's LAST clip, the gap is left
/// dangling at the end — e.g. move a clip far right then back leaves the far slot as a
/// trailing gap. Returns the ms removed (0 if the track didn't end in a gap).
fn trim_trailing_gaps(track: &mut Track) -> u64 {
    let mut removed = 0;
    while matches!(track.clips.last(), Some(Clip::Gap(_))) {
        if let Some(Clip::Gap(g)) = track.clips.pop() {
            removed += g.duration_ms;
        }
    }
    removed
}

/// Move a clip to `to_track` at timeline position `at_ms`. Gap-fills the
/// source position (so the source track does NOT ripple) and splices into
/// the destination (splitting whatever clip is under `at_ms`).
///
/// `ripple`: when TRUE, the destination splice also opens a gap of
/// the clip's duration in every OTHER video/audio track at `at_ms` (and remaps
/// captions/markers/duck-windows) — an AV-sync-preserving move, the mirror of
/// `edit.insert{ripple:true}`. When FALSE (the DEFAULT, legacy behavior),
/// only the destination track changes — the float/overlay move. ops logged
/// before the flag replay as `ripple:false`, byte-identical.
pub fn move_clip(
    project: &mut Project,
    clip_id: &str,
    to_track: &str,
    at_ms: u64,
    ripple: bool,
) -> Result<Vec<OpEffect>, CutError> {
    move_clip_pinned(project, clip_id, to_track, at_ms, ripple, None)
}

/// Id-pinning variant of [`move_clip`]. `pinned_split` forces the destination
/// splice-split id (effect `split_clip`) so a skip-replay keeps a move's
/// destination-split clip id stable (rebase.rs). The moved clip itself keeps
/// its id, so only the splice-split needs pinning. Live path passes None.
pub fn move_clip_pinned(
    project: &mut Project,
    clip_id: &str,
    to_track: &str,
    at_ms: u64,
    ripple: bool,
    pinned_split: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let (from_track, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let from_kind = project.track(&from_track).unwrap().kind;
    let to_kind = project
        .track(to_track)
        .ok_or_else(|| track_not_found(to_track))?
        .kind;
    if from_kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "caption clips cannot be moved with edit.move",
            "caption clips carry absolute ranges; use captions.* verbs",
        )
        .with_clip(clip_id));
    }
    if from_kind != to_kind {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("cannot move a {from_kind:?} clip onto {to_kind:?} track '{to_track}'"),
            "source and destination track kinds must match",
        )
        .with_clip(clip_id));
    }
    // Remove from source + gap-fill its slot so later clips keep position.
    let (old_t0, old_t1) = clip_span(project.track(&from_track).unwrap(), idx);
    let clip = {
        let t = project.track_mut(&from_track).unwrap();
        let c = t.clips.remove(idx);
        t.clips
            .insert(idx, Clip::Gap(GapClip::new(old_t1 - old_t0)));
        c
    };
    let dur = clip.timeline_duration_ms();
    // The destination splice may mint a right-half id when the move lands inside
    // a clip; record it as `split_clip` for replay/skip-replay pinning. `pinned`
    // is None on the live path; move's replay/skip path passes the recorded id.
    let split_clip = splice_into_track(project, to_track, at_ms, clip, pinned_split)?;
    let mut dest_detail = json!({"moved_clip": clip_id, "added_ms": [at_ms, at_ms + dur]});
    if let Some(sc) = &split_clip {
        dest_detail["split_clip"] = json!(sc);
    }
    let mut effects = vec![
        fx(
            Some(&from_track),
            json!({"moved_clip": clip_id, "gap_filled_ms": [old_t0, old_t1], "ripple": ripple}),
        ),
        fx(Some(to_track), dest_detail),
    ];
    // AV-sync ripple at the destination: shift sibling tracks +
    // captions/markers/duck-windows right by the clip's duration at at_ms, the
    // same machinery edit.insert{ripple:true} uses.
    if ripple {
        ripple_open_gap_at(project, to_track, at_ms, dur, &mut effects);
    }
    // Drop any trailing gap the move left dangling (the vacated far slot when a
    // clip is moved right then back). A trailing gap is pure black tail that inflates
    // the track length → playback/render past the content. Gaps BETWEEN clips stay.
    trim_trailing_gaps(project.track_mut(&from_track).unwrap());
    if to_track != from_track {
        trim_trailing_gaps(project.track_mut(to_track).unwrap());
    }
    Ok(effects)
}

/// Insert a new clip of `asset` on `track` at `at_ms`. `src_range_ms` selects
/// a sub-range of the asset (defaults to the full asset duration from probe).
/// Returns the new clip id in effects.
///
/// RIPPLE SEMANTICS (ripple-sync and content-relative-marker contracts, mirroring ripple_delete):
/// - `ripple = true`: a GAP of the inserted duration is spliced into every
///   OTHER video/audio track at `at_ms` (tracks ending at/before `at_ms` have
///   nothing after the point and are untouched), caption-clip ranges and ALL
///   tracks' duck gain-windows are remapped through `insert_map`, and markers
///   at/after `at_ms` shift right by the inserted duration. Everything that
///   was aligned stays aligned — AV sync preserved.
/// - `ripple = false`: only the target track shifts (deliberate overlay /
///   replace workflows). Markers and sibling tracks stay put — consistent
///   with the single-track form of ripple_delete.
/// - In BOTH modes the TARGET track's own gain windows are remapped (its
///   content after `at_ms` moves regardless), like single-track ripple_delete
///   remaps the deleted track's windows.
///
/// The caller (server dispatch) resolves the DEFAULT (base tracks ripple,
/// overlay/extra tracks float) and records `ripple` explicitly on the op —
/// core treats it as a plain bool so legacy ops without the key replay as
/// `false` (their original behavior).
pub fn insert(
    project: &mut Project,
    asset: &str,
    track: &str,
    at_ms: u64,
    src_range_ms: Option<[u64; 2]>,
    ripple: bool,
) -> Result<Vec<OpEffect>, CutError> {
    insert_pinned(
        project,
        asset,
        track,
        at_ms,
        src_range_ms,
        ripple,
        None,
        None,
    )
}

/// Id-pinning variant of [`insert`]. `pinned_clip` forces the inserted clip id
/// (effect `added_clip`); `pinned_split` forces the splice-split right-half id
/// (effect `split_clip`) for the case the insert lands inside an existing clip.
/// The LIVE path calls `insert` (both None ⇒ positional alloc + record); the
/// REPLAY / skip-replay path passes the recorded ids so the timeline id graph
/// stays stable across a rebased-out earlier op (rebase.rs). In the no-skip
/// case the pinned ids equal the positional ones, so replay is byte-identical.
#[allow(clippy::too_many_arguments)]
pub fn insert_pinned(
    project: &mut Project,
    asset: &str,
    track: &str,
    at_ms: u64,
    src_range_ms: Option<[u64; 2]>,
    ripple: bool,
    pinned_clip: Option<&str>,
    pinned_split: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let a = project.assets.get(asset).ok_or_else(|| {
        CutError::new(
            codes::NOT_FOUND,
            format!("no asset '{asset}'"),
            "asset id must come from media.import",
        )
        .with_suggested_action("call media.import first, or project.state to list assets")
    })?;
    let probed_duration_ms = a
        .probe
        .as_ref()
        .and_then(|p| p.get("duration_ms"))
        .and_then(|v| v.as_u64());
    let (s_in, s_out) = match src_range_ms {
        Some([i, o]) if i < o => (i, o),
        Some([i, o]) => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("src_range_ms [{i}, {o}) is empty or inverted"),
                "range start must be strictly less than range end",
            ))
        }
        None => {
            let d = probed_duration_ms.ok_or_else(|| {
                CutError::new(
                    codes::INVALID_ARGS,
                    format!(
                        "asset '{asset}' has no probe yet — full-length insert needs a duration"
                    ),
                    "src_range_ms was omitted and probe.duration_ms is unavailable",
                )
                .with_suggested_action("run media.probe first or pass src_range_ms")
            })?;
            (0, d)
        }
    };
    if let Some(d) = probed_duration_ms {
        if s_out > d {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("src_out_ms {s_out} exceeds asset duration {d}ms"),
                format!("asset '{asset}' is only {d}ms long"),
            ));
        }
    }
    match project.track(track).map(|t| t.kind) {
        None => return Err(track_not_found(track)),
        Some(TrackKind::Caption) => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "cannot insert media on a caption track",
                "caption clips are created via captions.* verbs",
            ))
        }
        Some(_) => {}
    }
    let id = match pinned_clip {
        Some(p) => p.to_string(),
        None => new_clip_id(project),
    };
    let d = s_out - s_in; // inserted timeline duration
    let clip = Clip::Media(make_media_clip(&id, asset, s_in, s_out));
    // The splice may mint a right-half id when the insert lands inside a clip;
    // record it as `split_clip` so replay/skip-replay can pin it (rebase.rs).
    let split_clip = splice_into_track(project, track, at_ms, clip, pinned_split)?;
    let mut detail = json!({"added_clip": id, "added_ms": [at_ms, at_ms + d], "src_range_ms": [s_in, s_out], "ripple": ripple});
    if let Some(sc) = &split_clip {
        detail["split_clip"] = json!(sc);
    }
    let mut effects = vec![fx(Some(track), detail)];
    // Target track's duck windows follow its own shifted content in BOTH
    // modes (the gain-window analog of the content-relative-marker contract — a window left behind would
    // duck the wrong moment after the insert).
    let target_id = track.to_string();
    {
        let t = project
            .track_mut(&target_id)
            .expect("target validated above");
        for w in &mut t.gain_windows {
            w.range_ms = [
                insert_map(w.range_ms[0], at_ms, d),
                insert_map(w.range_ms[1], at_ms, d),
            ];
        }
    }
    if ripple {
        ripple_open_gap_at(project, &target_id, at_ms, d, &mut effects);
    }
    Ok(effects)
}

/// Shared "insert `d` ms of empty time at `at_ms`, shifting everything after it
/// right" ripple, used by `edit.insert{ripple:true}` and `edit.move{ripple:
/// true}`. Gap-inserts into every cumulative (video/audio) track
/// EXCEPT `exclude_track` that has content after `at_ms`, remaps those tracks'
/// duck windows, stretches caption ranges over the gap, and shifts markers — so
/// AV sync and overlay/caption/marker alignment survive a sibling-rippling
/// insert or move. The CALLER has already mutated the originating track (placed
/// the clip / its gap there); this only touches the siblings + absolute-time
/// metadata. Appends per-sibling + caption + marker effects to `effects`.
fn ripple_open_gap_at(
    project: &mut Project,
    exclude_track: &str,
    at_ms: u64,
    d: u64,
    effects: &mut Vec<OpEffect>,
) {
    ripple_open_gap_at_excluding(project, &[exclude_track], at_ms, d, effects);
}

/// Multi-track form used by one logical linked A/V move. Both destination
/// tracks already received the moved clips, so neither may receive the sibling
/// ripple gap. This stays an internal core primitive; the public contract is
/// still `edit.move{linked:true,ripple:true}`.
pub(crate) fn ripple_open_gap_at_excluding(
    project: &mut Project,
    exclude_tracks: &[&str],
    at_ms: u64,
    d: u64,
    effects: &mut Vec<OpEffect>,
) {
    // Gap-insert into every OTHER cumulative (video/audio) track that has
    // content at/after the point; remap its gain windows with it.
    let sibling_ids: Vec<String> = project
        .tracks
        .iter()
        .filter(|t| {
            !exclude_tracks.contains(&t.id.as_str())
                && matches!(t.kind, TrackKind::Video | TrackKind::Audio)
                && t.duration_ms() > at_ms // nothing after the point ⇒ nothing to shift
        })
        .map(|t| t.id.clone())
        .collect();
    for sid in sibling_ids {
        // splice_into_track only fails on a missing track; sid was just listed.
        // Splicing a GAP never mints a clip id (gaps carry none), so there is
        // nothing to pin here — pass None and ignore the (always-None) result.
        let _ = splice_into_track(project, &sid, at_ms, Clip::Gap(GapClip::new(d)), None);
        let t = project.track_mut(&sid).expect("sibling listed above");
        for w in &mut t.gain_windows {
            w.range_ms = [
                insert_map(w.range_ms[0], at_ms, d),
                insert_map(w.range_ms[1], at_ms, d),
            ];
        }
        effects.push(fx(
            Some(&sid),
            json!({"rippled_gap_ms": [at_ms, at_ms + d]}),
        ));
    }
    // Caption ranges are ABSOLUTE: remap both edges. A caption spanning the
    // point stretches over the inserted content (edge-wise mapping — the same
    // convention ripple_delete uses for trims).
    for t in project
        .tracks
        .iter_mut()
        .filter(|t| t.kind == TrackKind::Caption)
    {
        let mut shifted = 0u64;
        for c in &mut t.clips {
            if let Clip::Caption(cc) = c {
                let mapped = [
                    insert_map(cc.range_ms[0], at_ms, d),
                    insert_map(cc.range_ms[1], at_ms, d),
                ];
                if mapped != cc.range_ms {
                    cc.range_ms = mapped;
                    shifted += 1;
                }
            }
        }
        if shifted > 0 {
            let tid = t.id.clone();
            effects.push(fx(Some(&tid), json!({"captions_shifted": shifted})));
        }
    }
    // Markers move with the content they annotate (the content-relative-marker contract — the insert mirror of
    // ripple_delete's all-tracks marker remap).
    let mut shifted = 0u64;
    for m in &mut project.markers {
        let mapped = insert_map(m.at_ms, at_ms, d);
        if mapped != m.at_ms {
            m.at_ms = mapped;
            shifted += 1;
        }
    }
    if shifted > 0 {
        effects.push(fx(None, json!({"markers_shifted": shifted})));
    }
}

/// Splice `clip` into `track_id` at absolute `at_ms`: pad with a gap when
/// past the end, insert at a boundary, or split the clip under the point
/// (later clips ripple right automatically — cumulative model).
///
/// Returns `Some(split_id)` when the splice landed INSIDE an existing MEDIA
/// clip and minted a fresh id for its right half — the caller records that id
/// in its effect (`split_clip`) so replay/skip-replay can pin it (rebase.rs).
/// Returns `None` for boundary inserts, past-the-end pads, and gap splits
/// (gaps carry no id). `pinned_split` forces the split id (replay path); when
/// None the id is allocated positionally (live path) exactly as before.
fn splice_into_track(
    project: &mut Project,
    track_id: &str,
    at_ms: u64,
    clip: Clip,
    pinned_split: Option<&str>,
) -> Result<Option<String>, CutError> {
    // Reserve a split id that can't collide with the incoming clip's id
    // (the incoming clip may not be in the project yet, e.g. during move).
    let incoming_n = clip
        .id()
        .and_then(|id| id.strip_prefix('c'))
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(0);
    let split_id = match pinned_split {
        Some(id) => id.to_string(),
        None => format!("c{}", max_clip_n(project).max(incoming_n) + 1),
    };
    let t = project
        .track_mut(track_id)
        .ok_or_else(|| track_not_found(track_id))?;
    let end = t.duration_ms();
    if at_ms >= end {
        if at_ms > end {
            t.clips.push(Clip::Gap(GapClip::new(at_ms - end)));
        }
        t.clips.push(clip);
        return Ok(None);
    }
    let mut cursor: u64 = 0;
    for i in 0..t.clips.len() {
        let dur = t.clips[i].timeline_duration_ms();
        if at_ms == cursor {
            t.clips.insert(i, clip);
            return Ok(None);
        }
        if at_ms < cursor + dur {
            let off = at_ms - cursor;
            // Track whether THIS splice minted a media right-half id (Some) or
            // split a gap / nothing (None) — the caller records the id so a
            // future skip-replay can pin it.
            let minted = match &mut t.clips[i] {
                Clip::Media(c) => {
                    // A variable-speed ramp makes the timeline→source map non-linear
                    // and its source-offset points window-relative — refuse splicing
                    // through it (insert/move at a mid-ramp point) rather than mis-cut.
                    if c.speed_ramp.is_some() {
                        return Err(ramp_conflict(&c.id, "split at this position"));
                    }
                    // `off` is timeline; the source split point scales by speed
                    // (identity at 1.0). The Gap arm below keeps the raw `off`
                    // (gaps are pure timeline, no source / no speed).
                    let src_off = crate::types::tl_off_to_src(off, c.speed);
                    let (left_fade, right_fade) = split_fade(&c.fade);
                    let right = MediaClip {
                        id: split_id.clone(),
                        asset: c.asset.clone(),
                        src_in_ms: c.src_in_ms + src_off,
                        src_out_ms: c.src_out_ms,
                        effects: c.effects.clone(),
                        gain_db: c.gain_db,
                        transform: c.transform.clone(),
                        // Crop survives a splice-split: same source rectangle.
                        crop: c.crop.clone(),
                        fade: right_fade,
                        // Crossfade-in stays on the LEFT half (its start is the
                        // original boundary); the right half is a hard cut.
                        xfade_in_ms: 0,
                        xfade_kind: None,
                        // The split inherits the parent's playback speed.
                        speed: c.speed,
                        // ...and the parent's color grade (look is unchanged).
                        grade: c.grade.clone(),
                        // ...and the parent's matte (same alpha cache).
                        matte: c.matte.clone(),
                        mask: c.mask.clone(),
                        // Both halves keep the parent's playback direction.
                        reverse: c.reverse,
                        freeze: c.freeze.clone(),
                        animation: c.animation.clone(),
                        keyframes: c.keyframes.clone(),
                        eq: c.eq.clone(),
                        // SOURCE-time mute ranges: both halves keep the parent's list.
                        mute_ranges: c.mute_ranges.clone(),
                        stabilize: c.stabilize.clone(),
                        // Splicing a ramped clip is refused above; halves carry none.
                        speed_ramp: None,
                        // Same source pixels → same input color space tag.
                        input_color_space: c.input_color_space,
                        nest: c.nest.clone(),
                        grade_stack: c.grade_stack.clone(),
                        grade_windows: c.grade_windows.clone(),
                    };
                    c.src_out_ms = c.src_in_ms + src_off;
                    c.fade = left_fade;
                    t.clips.insert(i + 1, Clip::Media(right));
                    Some(split_id.clone())
                }
                Clip::Gap(g) => {
                    let right = GapClip::new(g.duration_ms - off);
                    g.duration_ms = off;
                    t.clips.insert(i + 1, Clip::Gap(right));
                    None // gaps carry no id — nothing to pin
                }
                Clip::Caption(_) => return Err(caption_on_non_caption_track(track_id)),
            };
            t.clips.insert(i + 1, clip);
            return Ok(minted);
        }
        cursor += dur;
    }
    unreachable!("at_ms < track end implies an insertion point was found")
}

/// Duplicate the media clip `clip_id` (the universal NLE Ctrl+D / "Duplicate"):
/// clone it with ALL per-clip attributes — effects, gain_db, transform, crop,
/// fade, speed, speed_ramp, grade, matte, mask (redact), reverse, freeze,
/// animation, keyframes, eq, stabilize — and insert the copy on the SAME track
/// IMMEDIATELY AFTER the source clip, rippling per `ripple` like a normal insert.
///
/// Attribute fidelity: the clone is a `MediaClip { id: <new>, ..source.clone() }`
/// struct-update, so it carries EVERY current and future per-clip field
/// automatically (no per-field list to keep in sync with the struct — unlike
/// split/insert). The clone's CROSSFADE-IN is the one exception: it is reset to a
/// HARD CUT (`xfade_in_ms = 0`, `xfade_kind = None`). A crossfade-in is a property
/// of the clip's ORIGINAL left-neighbour boundary, NOT of the clip's content; the
/// clone's new left neighbour is its own source, so inheriting the crossfade would
/// dissolve two identical frames AND shorten the timeline by the overlap (the
/// clone would not lengthen the timeline by its own span). This is exactly the
/// doctrine `split`/`splice_into_track` already use when their RIGHT half starts
/// with a hard cut.
///
/// `ripple` is the insert-style sibling-track gap (the ripple-sync contract, the mirror of
/// ripple_delete): `true` splices a gap of the clone's duration into every OTHER
/// video/audio track at the insertion point (base-track AV sync); `false` leaves
/// the siblings untouched (overlay duplicate, AND the aligned linked-A/V pair —
/// the dispatch layer duplicates each half with `ripple:false` so the two new
/// clips shift their own tracks by the same amount and stay frame-locked, with no
/// leftover gap). Works on a RETIMED / ramped clip (the whole clip is copied at a
/// boundary — no timeline→source split — so split/trim's ramp refusal never
/// applies).
pub fn duplicate(
    project: &mut Project,
    clip_id: &str,
    ripple: bool,
) -> Result<Vec<OpEffect>, CutError> {
    duplicate_pinned(project, clip_id, ripple, None)
}

/// Id-pinning variant of [`duplicate`]. `pinned_clip` forces the cloned clip id
/// (effect `added_clip`) on the REPLAY / skip-replay path; the LIVE path passes
/// None ⇒ positional [`new_clip_id`] alloc, recorded in the effect. In the no-skip
/// case the pinned id EQUALS the positional one, so replay is byte-identical
/// (the same contract as insert/split). Since duplicate allocates exactly ONE id
/// per op, a single Option suffices (no queue) — the linked-A/V pair is TWO
/// separate ops, each pinning its own one id.
pub fn duplicate_pinned(
    project: &mut Project,
    clip_id: &str,
    ripple: bool,
    pinned_clip: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    // Allocate the clone id BEFORE the &mut borrow (positional alloc scans the
    // whole project). Replay pins the recorded id.
    let new_id = match pinned_clip {
        Some(p) => p.to_string(),
        None => new_clip_id(project),
    };

    // Locate the source clip + its track.
    let (track_id, idx) = project.find_clip(clip_id).ok_or_else(|| {
        CutError::new(
            codes::NOT_FOUND,
            format!("no clip '{clip_id}' on the timeline"),
            "clip must be an existing clip id (project.state lists clips)",
        )
        .with_clip(clip_id)
    })?;
    let track_id = track_id.to_string();
    let kind = project
        .track(&track_id)
        .expect("find_clip returned a real track id")
        .kind;
    if kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("clip '{clip_id}' is a caption clip"),
            "caption clips are managed via captions.* verbs, not edit.duplicate",
        )
        .with_clip(clip_id));
    }

    // Build the full-attribute clone + compute the insertion point under the
    // immutable borrow, then drop it before the &mut splice.
    let (clone, at_ms, d) = {
        let track = project.track(&track_id).expect("track validated above");
        let Clip::Media(src) = &track.clips[idx] else {
            // Video/audio tracks hold only Media/Gap; a gap carries no id so
            // find_clip never returns it — defensive (a future clip kind).
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("clip '{clip_id}' is not a media clip"),
                "edit.duplicate copies a media (video/audio) clip",
            )
            .with_clip(clip_id));
        };
        // Whole clip minus its id (struct-update carries every per-clip field).
        let mut clone = src.clone();
        clone.id = new_id.clone();
        // A duplicate starts with a HARD CUT from its source (see fn docs).
        clone.xfade_in_ms = 0;
        clone.xfade_kind = None;
        // The clone's TIMELINE span equals the source clip's (same source window
        // + same speed/ramp); read it off the source Clip (a Clip method).
        let d = track.clips[idx].timeline_duration_ms();
        // Insertion point = the cumulative timeline END of the source clip (the
        // boundary right after it). Splicing AT a boundary inserts the clone at
        // the next index with NO split.
        let at_ms: u64 = track.clips[..=idx]
            .iter()
            .map(|c| c.timeline_duration_ms())
            .sum();
        (clone, at_ms, d)
    };

    // Boundary insert ⇒ splice_into_track returns Ok(None) (no clip is split).
    let _ = splice_into_track(project, &track_id, at_ms, Clip::Media(clone), None)?;

    let mut effects = vec![fx(
        Some(&track_id),
        json!({
            "added_clip": new_id,
            "source_clip": clip_id,
            "added_ms": [at_ms, at_ms + d],
            "ripple": ripple,
        }),
    )];
    // The target track's own duck windows follow its shifted content in BOTH
    // modes (its content after at_ms moves regardless — the gain-window analog of
    // the content-relative-marker contract), mirroring insert.
    {
        let t = project
            .track_mut(&track_id)
            .expect("target validated above");
        for w in &mut t.gain_windows {
            w.range_ms = [
                insert_map(w.range_ms[0], at_ms, d),
                insert_map(w.range_ms[1], at_ms, d),
            ];
        }
    }
    if ripple {
        ripple_open_gap_at(project, &track_id, at_ms, d, &mut effects);
    }
    Ok(effects)
}

/// Collapse a CONTIGUOUS run of clips on ONE track into a single NEST / COMPOUND
/// CLIP (a nested compound clip). The selected clips are MOVED off the
/// parent track into a new [`Nest`] sub-timeline stored on the project (rebased so
/// the first starts at sub-time 0, with EVERY per-clip attribute — grade/effects/
/// fade/speed/timing — preserved losslessly) and REPLACED in place on the parent
/// track by ONE nest clip occupying their combined `[at_ms, at_ms+span)` span, so the
/// parent timeline LENGTH is unchanged.
///
/// CONTIGUITY RULE (documented): the selection must be MEDIA clips, all on the SAME
/// track, forming a CONTIGUOUS run of adjacent indices with NOTHING (no gap, no
/// caption) between them. A cross-track selection, a non-contiguous selection, a
/// gap/caption inside the run, or an already-nested clip is REFUSED with a clear
/// error rather than silently reordering the timeline.
///
/// LINKED AUDIO (scope limit, documented): v1 nests a SINGLE track's run only. When a
/// base-video run is nested, its muxed audio (a sibling clip on a1t) is NOT pulled in
/// — it stays on the parent. The nest BAKE renders whatever audio the nested clips
/// carry in-source; sibling-track audio is a multi-track-nest follow-up.
///
/// The nest clip references the nest by id in BOTH `nest` (the marker) and `asset`
/// (its "source" IS the nest); at render time the server bakes the sub-timeline to a
/// content-addressed file and feeds it in (server `nest::bake_and_flatten`), so the
/// main renderer is nest-blind and a project with NO nest renders byte-identical.
///
/// Replay-safe (ONE op): the new nest id is allocated deterministically (max nest
/// index + 1) and the nest CLIP id is pinned via `added_clip`, so rebuild_from_log
/// reproduces the timeline byte-for-byte. Undo is recompute-by-replay (edit.nest
/// `mutates_timeline`) — replaying without this op restores the original clips.
pub fn nest(
    project: &mut Project,
    clip_ids: &[String],
    name: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    nest_pinned(project, clip_ids, name, None)
}

/// Id-pinning variant of [`nest`]. `pinned_clip` forces the nest clip id (effect
/// `added_clip`) on the REPLAY / skip-replay path; the LIVE path passes None ⇒
/// positional [`new_clip_id`] alloc, recorded in the effect. In the no-skip case the
/// pinned id EQUALS the positional one, so replay is byte-identical (same contract as
/// insert/duplicate). One id per op ⇒ a single Option (no queue).
pub fn nest_pinned(
    project: &mut Project,
    clip_ids: &[String],
    name: Option<&str>,
    pinned_clip: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    if clip_ids.is_empty() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "edit.nest needs at least one clip",
            "pass clips:[...] — the clips to collapse into a compound clip / nest",
        ));
    }
    // Locate every clip; they must be unique and all on the SAME track.
    let mut seen = std::collections::BTreeSet::new();
    let mut located: Vec<usize> = Vec::new();
    let mut track_id: Option<String> = None;
    for cid in clip_ids {
        if !seen.insert(cid.clone()) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("clip '{cid}' is listed twice"),
                "each clip may appear once in a nest selection",
            )
            .with_clip(cid));
        }
        let (tid, idx) = project.find_clip(cid).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no clip '{cid}' on the timeline"),
                "clips must be existing clip ids (project.state lists clips)",
            )
            .with_clip(cid)
        })?;
        match &track_id {
            None => track_id = Some(tid.to_string()),
            Some(t) if t != tid => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("clips span two tracks ('{t}' and '{tid}')"),
                    "a nest collapses a contiguous run on ONE track; nest each track separately",
                )
                .with_clip(cid));
            }
            _ => {}
        }
        located.push(idx);
    }
    let track_id = track_id.expect("non-empty clips ⇒ a track");
    let kind = project
        .track(&track_id)
        .expect("find_clip returned a real track id")
        .kind;
    if kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "cannot nest caption clips",
            "captions are managed via captions.* verbs, not edit.nest",
        ));
    }
    // Require a CONTIGUOUS run of adjacent indices (last-first+1 == count).
    located.sort_unstable();
    let first = located[0];
    let last = *located.last().unwrap();
    if last - first + 1 != located.len() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "the selected clips are not contiguous on the track",
            "a nest collapses ADJACENT clips with nothing between them; select a contiguous run",
        ));
    }
    // Every clip in the run must be a MEDIA clip (no gap/caption) and not already a
    // nest (v1: no nest-of-nest, so the bake never recurses).
    {
        let t = project.track(&track_id).expect("track validated above");
        for c in &t.clips[first..=last] {
            match c {
                Clip::Media(m) if m.is_nest() => {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        format!("clip '{}' is already a nest", m.id),
                        "v1 does not support nesting a nest inside another nest",
                    )
                    .with_clip(&m.id));
                }
                Clip::Media(_) => {}
                Clip::Gap(_) => {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        "the selected run contains a gap",
                        "select a contiguous run of MEDIA clips with no gap between them",
                    ));
                }
                Clip::Caption(_) => {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        "the selected run contains a caption clip",
                        "nest media clips only",
                    ));
                }
            }
        }
    }
    // Allocate ids BEFORE the &mut borrow. Nest id is deterministic (max +1); the
    // nest clip id is pinned on replay.
    let nest_id = next_nest_id(project);
    let clip_id = match pinned_clip {
        Some(p) => p.to_string(),
        None => new_clip_id(project),
    };
    // Captured BEFORE the &mut track borrow (the realized-span probe below needs the
    // settings, but project can't be read while track_mut holds it).
    let settings = project.settings.clone();
    // Move the run out, splice the single nest clip in its place, and build the
    // sub-track. The nest is registered AFTER the track borrow drops (separate field
    // mutation), so the borrow checker stays happy.
    let (at_ms, span_ms, nested_ids, sub_track) = {
        let t = project.track_mut(&track_id).expect("track validated above");
        // Cumulative timeline start of the first selected clip.
        let at_ms: u64 = t.clips[..first]
            .iter()
            .map(|c| c.timeline_duration_ms())
            .sum();
        // Drain the contiguous run (in track order) into the nest sub-track.
        let moved: Vec<Clip> = t
            .clips
            .splice(first..=last, std::iter::empty::<Clip>())
            .collect();
        let nested_ids: Vec<String> = moved
            .iter()
            .filter_map(|c| c.id().map(String::from))
            .collect();
        // The nest clip's slot = the REALIZED duration of the sub-timeline (the same
        // crossfade-aware derivation the renderer + the bake use), NOT the naive sum of
        // clip durations — so the declared slot exactly matches the baked file length
        // (no frozen tail) even when an internal seam crossfades. A crossfade INTO the
        // first nested clip is dropped (the nest clip hard-cuts from its predecessor),
        // exactly like edit.duplicate's reset.
        let span_ms: u64 = {
            let mut probe = Project::new("nest_probe", settings.clone());
            probe.tracks = vec![Track {
                id: track_id.clone(),
                kind,
                clips: moved.clone(),
                gain_db: 0.0,
                gain_windows: vec![],
                blend_mode: None,
                visible: true,
                locked: false,
                muted: false,
                solo: false,
                pan: 0.0,
            }];
            crate::edl::edl_from_project(&probe).duration_ms
        };
        // The single nest clip occupies the SAME span on the parent (src 0..span over
        // the baked nest). `asset` == the nest id (its source IS the nest); `nest`
        // marks it so state/diff/the bake recognise it.
        let mut nclip = make_media_clip(&clip_id, &nest_id, 0, span_ms);
        nclip.nest = Some(nest_id.clone());
        t.clips.insert(first, Clip::Media(nclip));
        // Sub-track reuses the parent track id (namespaced inside the nest) + kind.
        let sub_track = Track {
            id: track_id.clone(),
            kind,
            clips: moved,
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        };
        (at_ms, span_ms, nested_ids, sub_track)
    };
    project.nests.push(Nest {
        id: nest_id.clone(),
        name: name.map(String::from),
        tracks: vec![sub_track],
    });
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "added_clip": clip_id,
            "added_nest": nest_id,
            "nested_clips": nested_ids,
            "added_ms": [at_ms, at_ms + span_ms],
            "span_ms": span_ms,
        }),
    )])
}

/// Replace the SOURCE of media clip `clip_id` IN PLACE with new footage, PRESERVING
/// the clip's id, timeline position, and slot duration — the 3-point "replace edit"
/// (a three-point replace edit). The clip is repointed at `new_asset` over the window
/// `[src_in, used_out)`; it KEEPS its look (effects, grade, transform, crop, fade,
/// gain, eq, mask, matte, stabilize, reverse, transitions, keyframes, animation) and
/// RESETS the three source-timing fields tied to the OLD footage — constant `speed`→
/// 1.0, `speed_ramp`→None, `freeze`→None (the replacement plays at NORMAL speed; use
/// `fit_to_fill` to speed-match a clip into a slot instead).
///
/// SLOT PRESERVATION + CLAMP/HOLD behaviour: let `slot` = the target's CURRENT
/// timeline span (already accounting for any old constant speed / speed-ramp). At
/// normal speed the replacement needs `slot` ms of source to fill the slot, so it
/// uses `[src_in, used_out)` where `used_out = min(src_in + slot, cap)` and `cap` is
/// the explicit `src_out` (if given) else the asset's PROBED duration. When that
/// usable window is SHORTER than `slot` (the source ran out — "insufficient media"),
/// the replacement is placed at the slot start and the REMAINDER of the slot is left
/// as a GAP (black / silence) — a CLAMP-and-pad, NOT a ripple — so the slot's total
/// duration and ALL downstream timing are preserved exactly. When the window covers
/// `slot` the replacement fills it precisely (any extra source is trimmed off).
///
/// The op allocates NO new clip id (the target keeps its id; the optional pad gap is
/// anonymous), so it is trivially replay-safe — replay re-derives `slot`/`used_out`/
/// `pad` from the same deterministic inputs and reproduces the mutation byte-for-byte.
/// Operates on ONE clip; the dispatch layer also replaces a muxed video clip's linked
/// audio sibling (each an independent in-place equal-duration swap, so NEITHER track
/// ripples — the linked-paste C-1 double-ripple trap cannot arise here).
pub fn replace(
    project: &mut Project,
    clip_id: &str,
    new_asset: &str,
    src_in: Option<u64>,
    src_out: Option<u64>,
) -> Result<Vec<OpEffect>, CutError> {
    // The replacement asset must be registered (defensive: on the live path
    // media.import ran first; on replay the import op replays before this step).
    if !project.assets.contains_key(new_asset) {
        return Err(CutError::new(
            codes::NOT_FOUND,
            format!("no asset '{new_asset}'"),
            "edit.replace swaps a clip onto a REGISTERED asset; import it first",
        )
        .with_clip(clip_id));
    }
    // Locate the target clip + its track.
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let kind = project
        .track(&track_id)
        .expect("find_clip returned a real track id")
        .kind;
    if kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("clip '{clip_id}' is a caption clip"),
            "caption clips are managed via captions.* verbs, not edit.replace",
        )
        .with_clip(clip_id));
    }
    // The slot to preserve = the target's CURRENT realized timeline span.
    let slot = project
        .track(&track_id)
        .expect("track validated above")
        .clips[idx]
        .timeline_duration_ms();
    if slot == 0 {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("clip '{clip_id}' has a zero-length timeline slot"),
            "there is no slot to fill",
        )
        .with_clip(clip_id));
    }
    let src_in = src_in.unwrap_or(0);
    // Usable-source upper bound: an explicit src_out caps it, the probed asset
    // duration caps it, and when both are known the SMALLER wins (you can't read
    // past either). At least one must be known to fill the slot deterministically.
    let probe_dur = project
        .assets
        .get(new_asset)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("duration_ms"))
        .and_then(|v| v.as_u64());
    let cap = match (src_out, probe_dur) {
        (Some(o), Some(d)) => o.min(d),
        (Some(o), None) => o,
        (None, Some(d)) => d,
        (None, None) => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("asset '{new_asset}' has no probe and no source_out_ms was given"),
                "the replacement source's available length is unknown",
            )
            .with_clip(clip_id)
            .with_suggested_action("run media.probe on the asset, or pass source_out_ms"));
        }
    };
    if src_in >= cap {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("source window [{src_in}, {cap}) is empty (source_in_ms is past the available source)"),
            "source_in_ms must be strictly less than the source's available end",
        )
        .with_clip(clip_id));
    }
    // 3-point fill: take exactly enough source to fill the slot at normal speed,
    // CLAMPED to the available source (the short-source case pads with a gap below).
    let used_out = (src_in + slot).min(cap);
    let used_span = used_out - src_in; // ≤ slot
    let pad = slot - used_span; // remainder padded with an anonymous gap (≥ 0)

    let (old_asset, old_src) = {
        let t = project.track_mut(&track_id).expect("track validated above");
        let Clip::Media(c) = &mut t.clips[idx] else {
            // Video/audio tracks hold only Media/Gap; a gap carries no id so
            // find_clip never returns it — defensive (a future clip kind).
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("clip '{clip_id}' is not a media clip"),
                "edit.replace swaps a media (video/audio) clip's source",
            )
            .with_clip(clip_id));
        };
        let old_asset = c.asset.clone();
        let old_src = [c.src_in_ms, c.src_out_ms];
        c.asset = new_asset.to_string();
        c.src_in_ms = src_in;
        c.src_out_ms = used_out;
        // RESET the source-timing fields tied to the OLD footage (see fn docs):
        // the new footage plays at normal speed and fills the slot directly.
        c.speed = 1.0;
        c.speed_ramp = None;
        c.freeze = None;
        // Pad the slot's remainder with a gap when the source ran short, so the
        // slot's total span (and all downstream timing) is preserved exactly.
        if pad > 0 {
            t.clips.insert(idx + 1, Clip::Gap(GapClip::new(pad)));
        }
        (old_asset, old_src)
    };
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip": clip_id,
            "old_asset": old_asset,
            "asset": new_asset,
            "old_src_ms": old_src,
            "new_src_ms": [src_in, used_out],
            "slot_ms": slot,
            "gap_ms": pad,
        }),
    )])
}

/// Place media clip `[src_in, src_out)` of `asset` into the EMPTY timeline slot
/// `[at_ms, at_ms + slot_ms)` on `track_id`, SPEED-ADJUSTING it so the retimed clip
/// EXACTLY fills the slot ("Fit to Fill"): `speed = source_span / slot_ms`,
/// so `timeline_duration_ms()` (= `source_span / speed`) lands back on `slot_ms` and
/// NO downstream content moves — the gap is consumed in place, not rippled.
///
/// The target span MUST be EMPTY: a single GAP that covers it, or at/after the track
/// end (which is padded). fit_to_fill FILLS space; it does not overwrite media (delete
/// the clip first — that leaves a gap — or use `edit.replace`). The caller (dispatch)
/// resolves `slot_ms` (from an explicit duration or the gap at `at_ms`) and the source
/// window, and validates `speed` against the [0.25, 4.0] retime range BEFORE commit, so
/// the logged op is self-contained and replay re-derives the same `speed` verbatim. The
/// placed clip's id is recorded as `added_clip` (pinned on replay, like insert).
pub fn fit_to_fill(
    project: &mut Project,
    track_id: &str,
    at_ms: u64,
    slot_ms: u64,
    asset: &str,
    src_in: u64,
    src_out: u64,
) -> Result<Vec<OpEffect>, CutError> {
    fit_to_fill_pinned(
        project, track_id, at_ms, slot_ms, asset, src_in, src_out, None,
    )
}

/// Id-pinning variant of [`fit_to_fill`]. `pinned_clip` forces the placed clip id
/// (effect `added_clip`) on the REPLAY / skip-replay path; the LIVE path passes None
/// ⇒ positional [`new_clip_id`] alloc, recorded in the effect. In the no-skip case the
/// pinned id EQUALS the positional one, so replay is byte-identical (insert's contract).
#[allow(clippy::too_many_arguments)]
pub fn fit_to_fill_pinned(
    project: &mut Project,
    track_id: &str,
    at_ms: u64,
    slot_ms: u64,
    asset: &str,
    src_in: u64,
    src_out: u64,
    pinned_clip: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let asset_record = project.assets.get(asset).ok_or_else(|| {
        CutError::new(
            codes::NOT_FOUND,
            format!("no asset '{asset}'"),
            "edit.fit_to_fill fills a slot from a REGISTERED asset; import it first",
        )
    })?;
    let probed_duration_ms = asset_record
        .probe
        .as_ref()
        .and_then(|p| p.get("duration_ms"))
        .and_then(|v| v.as_u64());
    if src_in >= src_out {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("source window [{src_in}, {src_out}) is empty or inverted"),
            "source range start must be strictly less than its end",
        ));
    }
    if let Some(d) = probed_duration_ms {
        if src_out > d {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("source window [{src_in}, {src_out}) exceeds asset duration {d}ms"),
                "source range must stay inside the probed asset duration",
            )
            .with_suggested_action("pick a shorter source range or re-run media.probe"));
        }
    }
    if slot_ms == 0 {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "fit_to_fill slot duration is zero",
            "the slot to fill must be a positive number of ms",
        )
        .with_at_ms(at_ms));
    }
    match project.track(track_id).map(|t| t.kind) {
        None => return Err(track_not_found(track_id)),
        Some(TrackKind::Caption) => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "cannot fit_to_fill onto a caption track",
                "caption clips are created via captions.* verbs",
            ))
        }
        Some(_) => {}
    }
    let new_id = match pinned_clip {
        Some(p) => p.to_string(),
        None => new_clip_id(project),
    };
    let source_span = src_out - src_in;
    // The defining equation: speed chosen so the retimed clip fills the slot.
    let speed = source_span as f64 / slot_ms as f64;
    // The realized timeline span (= source_span / speed, rounded to ms) — equals
    // slot_ms in all practical cases (the round-trip recovers the integer); the
    // gap-fill consumes exactly THIS many ms so the invariant holds regardless.
    let realized = crate::types::src_off_to_tl(source_span, speed);
    let mut clip = make_media_clip(&new_id, asset, src_in, src_out);
    clip.speed = speed;
    fill_empty_span(project, track_id, at_ms, Clip::Media(clip))?;
    Ok(vec![fx(
        Some(track_id),
        json!({
            "added_clip": new_id,
            "at_ms": at_ms,
            "added_ms": [at_ms, at_ms + realized],
            "slot_ms": slot_ms,
            "src_range_ms": [src_in, src_out],
            "speed": speed,
            "source_span_ms": source_span,
        }),
    )])
}

/// Place `clip` into EMPTY timeline space at `at_ms` on `track_id`, consuming the
/// clip's realized timeline span from the gap there WITHOUT rippling later content
/// (overwrite-into-gap — the mechanism behind `fit_to_fill`). The span
/// `[at_ms, at_ms + realized)` must be covered by a SINGLE gap clip, or lie at/after
/// the track end (which is padded with a leading gap). The covering gap is split into
/// `[left_gap] + clip + [right_gap]` (zero-length edges omitted), so the track's total
/// length — and every later clip's position — is unchanged. Returns `CONFLICT` when the
/// span is occupied by media or overruns the gap it lands in.
fn fill_empty_span(
    project: &mut Project,
    track_id: &str,
    at_ms: u64,
    clip: Clip,
) -> Result<(), CutError> {
    let realized = clip.timeline_duration_ms();
    let t = project
        .track_mut(track_id)
        .ok_or_else(|| track_not_found(track_id))?;
    let end = t.duration_ms();
    // Past-the-end: pad to at_ms then append. No later content to preserve.
    if at_ms >= end {
        if at_ms > end {
            t.clips.push(Clip::Gap(GapClip::new(at_ms - end)));
        }
        t.clips.push(clip);
        return Ok(());
    }
    // Find the clip covering at_ms; it MUST be a gap wide enough for the fill.
    let mut cursor: u64 = 0;
    for i in 0..t.clips.len() {
        let dur = t.clips[i].timeline_duration_ms();
        let span_end = cursor + dur;
        if at_ms < span_end {
            if !matches!(t.clips[i], Clip::Gap(_)) {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!("the slot at {at_ms}ms on '{track_id}' is occupied by media"),
                    "fit_to_fill fills empty space (a gap or the track tail), it does not overwrite a clip",
                )
                .with_at_ms(at_ms)
                .with_suggested_action(
                    "delete the clip there first (that leaves a gap to fill), or use edit.replace",
                ));
            }
            let free = span_end - at_ms; // gap room from at_ms to the gap's end
            if realized > free {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!(
                        "the {realized}ms fill at {at_ms}ms overruns the gap (only {free}ms free)"
                    ),
                    "the speed-fit clip is longer than the free space at this position",
                )
                .with_at_ms(at_ms)
                .with_suggested_action("widen the gap, or fit a shorter slot"));
            }
            // Split the covering gap into [left] + clip + [right] (drop 0-length).
            let left = at_ms - cursor;
            let right = span_end - (at_ms + realized);
            let mut repl: Vec<Clip> = Vec::new();
            if left > 0 {
                repl.push(Clip::Gap(GapClip::new(left)));
            }
            repl.push(clip);
            if right > 0 {
                repl.push(Clip::Gap(GapClip::new(right)));
            }
            t.clips.splice(i..=i, repl);
            return Ok(());
        }
        cursor = span_end;
    }
    unreachable!("at_ms < track end implies a covering clip was found")
}

/// Target selector for `gain`: a single clip or a whole track (public verb contract
/// `gain{clip|track,db}`).
#[derive(Debug, Clone, PartialEq)]
pub enum GainTarget {
    Clip(String),
    Track(String),
}

/// Set audio gain (dB) on a clip or track. Absolute set, not delta — the op
/// effect records old + new values so the change is fully auditable.
pub fn gain(project: &mut Project, target: GainTarget, db: f64) -> Result<Vec<OpEffect>, CutError> {
    match target {
        GainTarget::Clip(id) => {
            let (track_id, idx) = project
                .find_clip(&id)
                .map(|(t, i)| (t.to_string(), i))
                .ok_or_else(|| clip_not_found(&id))?;
            let track_kind = project
                .track(&track_id)
                .map(|track| track.kind)
                .ok_or_else(|| track_not_found(&track_id))?;
            if track_kind != TrackKind::Audio {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("clip '{id}' is not on an audio track"),
                    "the audio render graph reads audio-track clips only; target the linked audio clip from project.state",
                )
                .with_clip(&id));
            }
            let t = project
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
                .unwrap();
            match &mut t.clips[idx] {
                Clip::Media(c) => {
                    let old = c.gain_db;
                    c.gain_db = db;
                    Ok(vec![fx(
                        Some(&track_id),
                        json!({"target": "clip", "id": id, "old_db": old, "new_db": db}),
                    )])
                }
                _ => Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("'{id}' is not a media clip"),
                    "gain applies to media clips or whole tracks",
                )
                .with_clip(&id)),
            }
        }
        GainTarget::Track(id) => {
            let t = audio_track_mut(project, &id, "gain")?;
            let old = t.gain_db;
            t.gain_db = db;
            Ok(vec![fx(
                Some(&id),
                json!({"target": "track", "id": id, "old_db": old, "new_db": db}),
            )])
        }
    }
}

fn audio_track_mut<'a>(
    project: &'a mut Project,
    track_id: &str,
    action: &str,
) -> Result<&'a mut Track, CutError> {
    let track = project
        .track_mut(track_id)
        .ok_or_else(|| track_not_found(track_id))?;
    if track.kind != TrackKind::Audio {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("track '{track_id}' is not an audio track"),
            format!(
                "{action} changes the audio mix, which reads TrackKind::Audio only; target the linked audio track from project.state"
            ),
        ));
    }
    Ok(track)
}

/// Target selector for `fade`: one clip, or a whole track (resolved to its
/// first/last media clips — see `fade`).
#[derive(Debug, Clone, PartialEq)]
pub enum FadeTarget {
    Clip(String),
    Track(String),
}

/// Set linear fades (verb `edit.fade {clip|track, in_ms?, out_ms?, kind}` —
/// fade-edit contract). Provided fields are SET, omitted fields keep their current
/// value, explicit 0 clears that side; a fade collapsing to 0/0 clears
/// entirely. `kind` always replaces the clip's fade kind (one kind per clip).
///
/// TRACK form: a convenience that resolves NOW — `in_ms` lands on the track's
/// FIRST media clip, `out_ms` on its LAST (the "fade the music bed in/out"
/// gesture). It records plain clip fades; later timeline edits do not re-aim
/// it (re-run after restructuring, same doctrine as edit.duck).
///
/// HONEST SCOPE: linear fades only, per-clip, video fades go to black (alpha
/// on overlay tracks); CROSSFADES between adjacent clips are v2. A `kind`
/// that cannot do anything on the target's track (audio fade on a video
/// track — video tracks contribute no audio to the mix; video fade on an
/// audio track) is refused rather than silently recorded as a no-op.
pub fn fade(
    project: &mut Project,
    target: FadeTarget,
    in_ms: Option<u64>,
    out_ms: Option<u64>,
    kind: FadeKind,
) -> Result<Vec<OpEffect>, CutError> {
    if in_ms.is_none() && out_ms.is_none() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "edit.fade needs at least one of in_ms|out_ms",
            "neither was given — there is nothing to set",
        ));
    }
    // Resolve target clips: [(clip_id, set_in?, set_out?)].
    let assignments: Vec<(String, Option<u64>, Option<u64>)> = match &target {
        FadeTarget::Clip(id) => vec![(id.clone(), in_ms, out_ms)],
        FadeTarget::Track(track_id) => {
            let t = project
                .track(track_id)
                .ok_or_else(|| track_not_found(track_id))?;
            let media_ids: Vec<String> = t
                .clips
                .iter()
                .filter_map(|c| match c {
                    Clip::Media(m) => Some(m.id.clone()),
                    _ => None,
                })
                .collect();
            let (Some(first), Some(last)) = (media_ids.first(), media_ids.last()) else {
                return Err(CutError::new(
                    codes::NOT_FOUND,
                    format!("track '{track_id}' has no media clips to fade"),
                    "the track form fades the first clip in and the last clip out",
                ));
            };
            if first == last {
                vec![(first.clone(), in_ms, out_ms)]
            } else {
                let mut v = Vec::new();
                if in_ms.is_some() {
                    v.push((first.clone(), in_ms, None));
                }
                if out_ms.is_some() {
                    v.push((last.clone(), None, out_ms));
                }
                v
            }
        }
    };
    let mut effects = Vec::new();
    for (clip_id, set_in, set_out) in assignments {
        let (track_id, idx) = project
            .find_clip(&clip_id)
            .map(|(t, i)| (t.to_string(), i))
            .ok_or_else(|| clip_not_found(&clip_id))?;
        let track_kind = project.track(&track_id).unwrap().kind;
        // Kind-vs-track sanity: refuse fades that cannot render on this track.
        match (kind, track_kind) {
            (FadeKind::Audio, TrackKind::Video) => return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("kind 'audio' cannot fade '{clip_id}' on video track '{track_id}'"),
                "video tracks contribute no audio to the mix — an audio fade there renders nothing",
            )
            .with_clip(&clip_id)
            .with_suggested_action("use kind 'video' (or 'both') on video tracks")),
            (FadeKind::Video, TrackKind::Audio) => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("kind 'video' cannot fade '{clip_id}' on audio track '{track_id}'"),
                    "audio clips have no pixels",
                )
                .with_clip(&clip_id)
                .with_suggested_action("use kind 'audio' (or 'both') on audio tracks"))
            }
            (_, TrackKind::Caption) => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "caption clips cannot be faded with edit.fade",
                    "caption styling is captions.set_style territory",
                )
                .with_clip(&clip_id))
            }
            _ => {}
        }
        let t = project
            .tracks
            .iter_mut()
            .find(|t| t.id == track_id)
            .unwrap();
        let Clip::Media(c) = &mut t.clips[idx] else {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("'{clip_id}' is not a media clip"),
                "fades apply to media clips (gaps are already silence/black)",
            )
            .with_clip(&clip_id));
        };
        let dur = Clip::Media(c.clone()).timeline_duration_ms();
        let old = c.fade.clone();
        let new_in = set_in.unwrap_or_else(|| old.as_ref().map_or(0, |f| f.in_ms));
        let new_out = set_out.unwrap_or_else(|| old.as_ref().map_or(0, |f| f.out_ms));
        if new_in + new_out > dur {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "fade in {new_in}ms + out {new_out}ms exceeds clip '{clip_id}' duration {dur}ms"
                ),
                "the ramps would overlap — the clip would never reach full level",
            )
            .with_clip(&clip_id)
            .with_suggested_action("shorten the fades or trim the clip longer"));
        }
        let new = (new_in > 0 || new_out > 0).then_some(ClipFade {
            in_ms: new_in,
            out_ms: new_out,
            kind,
        });
        c.fade = new.clone();
        effects.push(fx(
            Some(&track_id),
            json!({"clip": clip_id, "old_fade": old, "new_fade": new}),
        ));
    }
    Ok(effects)
}

/// Set (or clear) a clip's overlay geometry (verb `edit.transform`).
/// Normalized values (types.rs ClipTransform): x/y = top-left as a fraction
/// of frame width/height, scale = overlay width as a fraction of frame width
/// (height proportional). Identity (0, 0, 1) CLEARS the transform. Applies
/// to media clips on VIDEO tracks; the renderer reads it for clips on
/// overlay tracks (any video track after the first non-empty one). On the base track the
/// renderer places/scales the clip over black and applies opacity against black.
pub fn transform(
    project: &mut Project,
    clip_id: &str,
    t: crate::types::ClipTransform,
) -> Result<Vec<OpEffect>, CutError> {
    if !(t.scale > 0.0 && t.scale <= 1.0) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("scale must be in (0, 1], got {}", t.scale),
            "scale is the overlay's width as a fraction of the frame; >1 zoom is out of scope for compositing v1",
        ));
    }
    if !(0.0..=1.0).contains(&t.x) || !(0.0..=1.0).contains(&t.y) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("x/y must be within [0, 1], got ({}, {})", t.x, t.y),
            "positions are normalized to the frame; partially-offscreen placement is out of scope for compositing v1, so out-of-range x/y are rejected",
        ));
    }
    if !(0.0..=1.0).contains(&t.opacity) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("opacity must be within [0, 1], got {}", t.opacity),
            "opacity is the overlay's alpha — 1 = fully opaque, 0 = invisible",
        ));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    if track.kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "transform positions/scales video pixels — audio/caption clips have none",
        )
        .with_clip(clip_id));
    }
    match &mut track.clips[idx] {
        Clip::Media(c) => {
            let old = c.transform.clone();
            let new = if t.is_identity() { None } else { Some(t) };
            c.transform = new.clone();
            Ok(vec![fx(
                Some(&track_id),
                json!({"clip": clip_id, "old_transform": old, "new_transform": new}),
            )])
        }
        _ => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "transform applies to media clips",
        )
        .with_clip(clip_id)),
    }
}

/// Set (or clear) a clip's SOURCE crop rectangle (verb `edit.crop`). Values are
/// source pixels (types.rs ClipCrop):
/// `crop` is the rectangle of source frame to KEEP. The canonical use is
/// removing a baked-in letterbox/pillarbox detected as the import's
/// `content_bbox` perception fact (an OBS canvas/window mismatch bakes black
/// bands into the source pixels — the editor must crop them off before the
/// frame is conformed, or they ship in the render).
///
/// COMPOSE ORDER: crop happens in SOURCE space, BEFORE conform/transform
/// (render.rs documents + implements this). A crop on the BASE track removes
/// the bands and the conform scale/pads the cropped picture to fill the
/// project frame; a crop on an overlay clip crops the source before the PiP
/// transform places it.
///
/// VALIDATION: when the asset is probed, the crop must lie fully inside the
/// source geometry (x+w ≤ width, y+h ≤ height) — an out-of-bounds crop is a
/// hard error (ffmpeg would clamp silently and the receipt could not reason
/// about it). An identity crop (origin + full source size) CLEARS the crop
/// (stored as None) so the no-op replays byte-identically. Applies to media
/// clips on video tracks only (audio/caption clips have no pixels).
pub fn crop(
    project: &mut Project,
    clip_id: &str,
    rect: crate::types::ClipCrop,
) -> Result<Vec<OpEffect>, CutError> {
    if rect.w == 0 || rect.h == 0 {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("crop w/h must be > 0, got {}x{}", rect.w, rect.h),
            "a zero-size crop would keep no pixels",
        )
        .with_clip(clip_id));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "crop selects source pixels — audio/caption clips have none",
        )
        .with_clip(clip_id));
    }
    // Look up the source geometry from the clip's asset probe (when probed)
    // to bounds-check and to recognise an identity (full-frame) crop.
    let asset_id = match &project.track(&track_id).unwrap().clips[idx] {
        Clip::Media(c) => c.asset.clone(),
        _ => {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("'{clip_id}' is not a media clip"),
                "crop applies to media clips",
            )
            .with_clip(clip_id))
        }
    };
    let src_geom: Option<(u32, u32)> = project
        .assets
        .get(&asset_id)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| {
            let w = p.get("width").and_then(|v| v.as_u64())? as u32;
            let h = p.get("height").and_then(|v| v.as_u64())? as u32;
            Some((w, h))
        });
    if let Some((sw, sh)) = src_geom {
        let out_of_bounds = rect.x.checked_add(rect.w).is_none_or(|right| right > sw)
            || rect.y.checked_add(rect.h).is_none_or(|bottom| bottom > sh);
        if out_of_bounds {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "crop {}x{}+{}+{} exceeds source geometry {sw}x{sh}",
                    rect.w, rect.h, rect.x, rect.y
                ),
                "the crop rectangle must lie fully inside the source frame",
            )
            .with_clip(clip_id)
            .with_suggested_action(
                "use the asset's content_bbox perception fact, or a sub-rectangle of the source",
            ));
        }
    }
    // Identity crop (full source frame) clears — but only when we KNOW the
    // source size; without a probe an origin crop of any size is kept as-is
    // (we cannot prove it is full-frame, and replay must be deterministic).
    let new = match src_geom {
        Some((sw, sh)) if rect.is_full_frame(sw, sh) => None,
        _ => Some(rect),
    };
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        unreachable!("media clip confirmed above");
    };
    let old = c.crop.clone();
    c.crop = new.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_crop": old, "new_crop": new}),
    )])
}

/// Set the color grade on a media clip (verb `edit.grade`). Stores a
/// `ClipGrade` (parametric eq contrast/brightness/saturation/gamma + optional
/// white-balance temperature + optional `.cube` LUT path). An IDENTITY grade
/// clears it to None (renders byte-identical to ungraded — so re-applying the
/// defaults is the "remove grade" gesture). Grade is a color operation on
/// pixels — audio/caption clips are refused. The LUT path (if any) is stored
/// verbatim; its existence/extension is validated at the DISPATCH layer (live
/// only), so replay stays filesystem-independent.
pub fn grade(
    project: &mut Project,
    clip_id: &str,
    grade: crate::types::ClipGrade,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "grade is a color operation on video — audio/caption clips have no pixels",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "grade applies to media clips",
        )
        .with_clip(clip_id));
    };
    // Identity → None: no filter is emitted and the clip renders byte-identical
    // to ungraded (this is also how a grade is cleared — pass the defaults).
    let new = if grade.is_identity() {
        None
    } else {
        Some(grade)
    };
    let old = c.grade.clone();
    c.grade = new.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_grade": old, "new_grade": new}),
    )])
}

/// Set a LAYERED color-grade STACK on a media clip (verb `edit.grade_stack`). Stores a
/// `Vec<ClipGrade>` applied IN ORDER by the renderer (layer N grades layer N-1's output)
/// — a serial grading node-stack workflow, vs the single grade `edit.grade`
/// stores. The stack is the AUTHORITY when present: the single `grade` field is cleared
/// (set None) so the renderer reads only the stack.
///
/// Identity layers are dropped (each would emit no filter — exactly as `edit.grade`
/// collapses an identity grade to None), so an empty / all-identity `grades` clears BOTH
/// the stack and the single grade → the clip renders byte-identical to ungraded. A
/// SINGLE non-identity layer is stored as a one-element stack that emits the EXACT same
/// filter chain as the equivalent `edit.grade`, so it renders byte-identical to a plain
/// per-clip grade. Color op on pixels — audio/caption clips are refused. Pure replay
/// (clip fields only; no filesystem/network touch). Any `.cube` LUT path inside a layer
/// is stored verbatim and fenced (existence/extension) at the DISPATCH layer, like
/// `edit.grade`.
pub fn grade_stack(
    project: &mut Project,
    clip_id: &str,
    grades: Vec<crate::types::ClipGrade>,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "grade is a color operation on video — audio/caption clips have no pixels",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "grade applies to media clips",
        )
        .with_clip(clip_id));
    };
    // Drop identity layers: each emits no filter, so keeping them would only bloat the
    // stored stack. An empty result == the clip is fully ungraded (byte-identical).
    let stack: Vec<crate::types::ClipGrade> =
        grades.into_iter().filter(|g| !g.is_identity()).collect();
    let old_stack = c.grade_stack.clone();
    let old_grade = c.grade.clone();
    c.grade_stack = stack.clone();
    // The stack supersedes the single grade — clear it so the renderer reads ONLY the
    // stack (and an empty stack leaves the clip ungraded, byte-identical to never-graded).
    c.grade = None;
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip": clip_id,
            "old_grade": old_grade,
            "old_grade_stack": old_stack,
            "new_grade_stack": stack,
        }),
    )])
}

/// Add a GEOMETRIC POWER WINDOW to a media clip (verb `edit.grade_window`), REMOVE one by
/// index, or CLEAR all windows. A power window grades ONLY a REGION of the frame — the
/// geometric grade-window, the region-scoped-grade gap vs `edit.grade`'s whole-frame look. `window` = a
/// [`crate::types::WindowShape`] region (rect/ellipse/polygon, reusing the `edit.add_mask`
/// geometry vocabulary); `grade` = the SAME [`crate::types::ClipGrade`] params `edit.grade`
/// takes, applied INSIDE the region (outside is untouched).
///
/// APPEND semantics: each call PUSHES one window onto `MediaClip::grade_windows`, so
/// windows STACK (the renderer composites them in order — e.g. warm a face in one window,
/// cool the background in another). `remove_index = Some(i)` removes exactly one window;
/// `window = None` with no removal (verb `enabled:false`) CLEARS every window on the clip
/// (the `edit.add_mask` clear idiom). An IDENTITY grade is refused (a
/// window that grades nothing is a no-op — pass real grade params, or clear). Geometric
/// region only — HSL/luma QUALIFIERS (key by colour) are a documented follow-up.
///
/// v1 renders on the BASE (first) video track (the proven mask-composite path); an
/// overlay-track clip is refused with a clear message (overlay-clip windows are a
/// follow-up). A speed-RAMPED clip is refused (the multi-segment retime conflicts with the
/// region composite, exactly as `edit.add_mask`). Pure replay (clip fields only; no
/// filesystem/network touch).
pub fn grade_window(
    project: &mut Project,
    clip_id: &str,
    window: Option<crate::types::WindowShape>,
    grade: crate::types::ClipGrade,
    remove_index: Option<usize>,
) -> Result<Vec<OpEffect>, CutError> {
    use crate::types::MaskShape;
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "a power window is a color operation on video — audio/caption clips have no pixels",
        )
        .with_clip(clip_id));
    }
    // v1 SCOPE: windows render on the BASE (first) video track — the same proven
    // region-composite path edit.add_mask uses. An overlay clip would render the window
    // as a silent no-op, so refuse it (overlay-clip windows are a follow-up). Clearing
    // Clearing/removing is always allowed (lets you remove windows regardless).
    let base_video = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .map(|t| t.id.as_str());
    if window.is_some() && remove_index.is_some() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "a power window append cannot also remove an existing window",
            "pass shape + grade params to append, remove_index to remove one, or enabled:false to clear all",
        )
        .with_clip(clip_id));
    }
    if window.is_some() && base_video != Some(track_id.as_str()) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is on an overlay video track; power windows currently render only on the BASE (first) video track"),
            "move the clip to the base video track to add a power window (overlay-clip windows are not yet supported)",
        )
        .with_clip(clip_id));
    }
    // Validate geometry + grade BEFORE mutating (a bad window leaves state untouched).
    if let Some(w) = &window {
        let need = match w.shape {
            MaskShape::Rect => "rect needs 2 points (opposite corners)",
            MaskShape::Ellipse => "ellipse needs 2 points (centre, radii)",
            MaskShape::Polygon => "polygon needs at least 3 points (vertices)",
        };
        let ok = match w.shape {
            MaskShape::Rect | MaskShape::Ellipse => w.points.len() == 2,
            MaskShape::Polygon => w.points.len() >= 3,
        };
        if !ok {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "window shape '{}' has {} points — {need}",
                    w.shape.as_str(),
                    w.points.len()
                ),
                need,
            )
            .with_clip(clip_id));
        }
        if w.feather < 0.0 || !w.feather.is_finite() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "window feather must be a non-negative fraction of frame height",
                "feather 0 = hard edge; 0.02 ≈ a soft 2%-of-height edge",
            )
            .with_clip(clip_id));
        }
        // A window with an identity grade composites nothing — refuse it (the renderer
        // would emit no grade filter, so the window would be invisible). Keeps the stored
        // stack meaningful + the no-window render byte-identical.
        if grade.is_identity() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "power window grade is identity (nothing to grade inside the region)",
                "pass grade params (contrast/brightness/saturation/gamma/temperature_k/lut), or enabled:false to clear",
            )
            .with_clip(clip_id));
        }
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "power windows apply to media clips",
        )
        .with_clip(clip_id));
    };
    if window.is_some() && c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.grade_window"));
    }
    let old = c.grade_windows.clone();
    match (remove_index, window) {
        (Some(i), None) => {
            if i >= c.grade_windows.len() {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!(
                        "power window index {i} is out of range (clip has {} windows)",
                        c.grade_windows.len()
                    ),
                    "refresh project state and pass a zero-based remove_index for an existing window",
                )
                .with_clip(clip_id));
            }
            c.grade_windows.remove(i);
        }
        // Clear ALL windows (enabled:false) — byte-identical to a never-windowed clip.
        (None, None) => c.grade_windows.clear(),
        // APPEND one window — windows stack, composited in order by the renderer.
        (None, Some(w)) => c
            .grade_windows
            .push(crate::types::GradeWindow { window: w, grade }),
        (Some(_), Some(_)) => unreachable!("append/remove conflict was validated above"),
    }
    let new = c.grade_windows.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_grade_windows": old, "new_grade_windows": new}),
    )])
}

/// Tag (or clear) a media clip's INPUT color space (verb `edit.color_space`). Stores
/// `input_color_space = Some(space)` so the renderer converts the source INTO the
/// project working space (and on to output) before grade/effects; `None` clears the
/// tag (renders byte-identical to an untagged clip — the source is then assumed
/// already in the working space). A color operation on pixels → audio/caption clips
/// are refused. Pure replay (just a clip field) — no filesystem/network touch.
pub fn set_color_space(
    project: &mut Project,
    clip_id: &str,
    input: Option<crate::types::ColorSpace>,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "color space tags VIDEO pixels — audio/caption clips have none",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "color space applies to media clips",
        )
        .with_clip(clip_id));
    };
    let old = c.input_color_space;
    c.input_color_space = input;
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip": clip_id,
            "old_input": old.map(|s| s.as_str()),
            "new_input": input.map(|s| s.as_str()),
        }),
    )])
}

/// Set (or clear) a media clip's AI background MATTE (verb `edit.matte`). `matte`
/// = `Some(..)` stores the intent (the renderer composites the baked alpha);
/// `None` clears it (renders byte-identical to un-matted — the "remove background
/// removal" gesture). Matte is a pixel operation on video — audio/caption clips
/// are refused. `mode = remove` reveals a LOWER track, so it is refused on a
/// BASE-track clip (nothing under the canvas) exactly like chroma key; `replace`
/// fills its own background, so it is allowed on any video clip.
///
/// The ALPHA itself is a content-addressed cache artifact baked at the DISPATCH
/// layer (live-only, network) — core only stores the intent, so replay stays
/// pure + filesystem-independent. Returns the op effect (old + new) for the
/// receipt/undo.
pub fn matte(
    project: &mut Project,
    clip_id: &str,
    matte: Option<crate::types::ClipMatte>,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "matte cuts the subject out of VIDEO — audio/caption clips have no pixels",
        )
        .with_clip(clip_id));
    }
    // mode=remove reveals a LOWER track; the base canvas has nothing under it.
    // (replace fills its own bg, so it is allowed on the base.)
    if let Some(m) = &matte {
        if m.mode == crate::types::MatteMode::Remove {
            let is_base = project
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Video && !t.clips.is_empty())
                .map(|t| t.id.as_str())
                == Some(track_id.as_str());
            if is_base {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("'{clip_id}' needs a layer below it"),
                    "matte remove drops the background to reveal a LOWER track; the base track has nothing under it",
                )
                .with_clip(clip_id)
                .with_suggested_action(
                    "place the background on the base track and this subject clip on a video track ABOVE it, then apply matte remove — or use mode=replace to fill its own background",
                ));
            }
        }
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "matte applies to media clips",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.matte"));
    }
    let old = c.matte.clone();
    c.matte = matte.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_matte": old, "new_matte": matte}),
    )])
}

/// Set a media clip's REVERSE-playback flag (verb `edit.reverse`). When enabled
/// the renderer plays the clip BACKWARD (`reverse` video / `areverse` audio);
/// the timeline duration is UNCHANGED (a reversed N-frame clip is still N
/// frames). Applies to MEDIA clips on a video OR audio track (a caption clip has
/// no frames/samples to reverse). The clip-size RAM fence (reverse buffers the
/// whole clip in memory) lives at the DISPATCH layer (live-only) so replay stays
/// deterministic and filesystem-independent; core just records the flag.
/// Returns the op effect (old + new) for the receipt/undo.
pub fn reverse(
    project: &mut Project,
    clip_id: &str,
    enabled: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "reverse applies to media clips (video/audio); captions have nothing to reverse",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.reverse"));
    }
    let old = c.reverse;
    c.reverse = enabled;
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_reverse": old, "reverse": enabled}),
    )])
}

/// Set (or clear) a media clip's VIDEO STABILIZATION (verb `edit.stabilize`).
/// enabled:true smooths camera shake (the renderer runs a `vidstabdetect` analysis
/// pre-pass + `vidstabtransform`); enabled:false clears it. `smoothing` is the
/// look-ahead/behind window in FRAMES (clamped 1..=100; higher = steadier but more
/// locked-down). VIDEO-track media clips only (stabilization is a picture
/// operation). CPU-only + the detect pre-pass → opts the timeline out of the GPU
/// fast-track. Returns the op effect (old + new) for the receipt/undo.
pub fn stabilize(
    project: &mut Project,
    clip_id: &str,
    smoothing: f64,
    enabled: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "stabilize smooths camera shake in the picture — audio/caption clips have no frame",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "stabilize applies to media clips",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.stabilize"));
    }
    let new = if enabled {
        Some(crate::types::ClipStabilize {
            smoothing: smoothing.clamp(1.0, 100.0),
        })
    } else {
        None
    };
    let old = c.stabilize.clone();
    c.stabilize = new.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_stabilize": old, "stabilize": new}),
    )])
}

/// Set (or clear) a media clip's FREEZE-FRAME (verb `edit.freeze`). enabled:true
/// HOLDS the single source frame at `at_ms` (offset into the clip's visible
/// range, clamped to the last available frame) for the clip's whole timeline
/// slot; enabled:false clears it. VIDEO-track media clips only (freezing is a
/// picture operation — the audio plays through untouched). The timeline duration
/// is UNCHANGED (the held frame fills the existing slot). Returns the op effect
/// (old + new) for the receipt/undo.
pub fn freeze(
    project: &mut Project,
    clip_id: &str,
    at_ms: u64,
    enabled: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "freeze holds a picture frame — audio/caption clips have no frame to hold",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "freeze applies to media clips",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.freeze"));
    }
    let new = if enabled {
        // Clamp the freeze point into the clip's visible source span so it always
        // names a real frame (the renderer also clamps to the last frame).
        let span = c.src_out_ms.saturating_sub(c.src_in_ms);
        let clamped = at_ms.min(span.saturating_sub(1));
        Some(crate::types::ClipFreeze { at_ms: clamped })
    } else {
        None
    };
    let old = c.freeze.clone();
    c.freeze = new.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_freeze": old, "freeze": new}),
    )])
}

/// Set (or clear) a media clip's Ken Burns ANIMATION (verb `edit.animate`). The
/// resolved `anim` (from/to zoom + focal centre) is CLAMPED (zoom into [1,10],
/// x/y into [0,1]) and stored; an identity animation (no zoom, centred at both
/// ends) clears it (stored None → byte-identical replay). VIDEO-track media clips
/// only. Returns the op effect (old + new) for the receipt/undo.
pub fn animate(
    project: &mut Project,
    clip_id: &str,
    anim: crate::types::ClipAnimation,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "animate pans/zooms the picture — audio/caption clips have no frame",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "animate applies to media clips",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.animate"));
    }
    // Clamp both ends into valid ranges (belt-and-suspenders over the verb layer;
    // the renderer also clamps). zoom >= 1 (zoompan requires it); x/y are fractions.
    let clamp = |s: &crate::types::AnimState| crate::types::AnimState {
        zoom: s.zoom.clamp(1.0, 10.0),
        x: s.x.clamp(0.0, 1.0),
        y: s.y.clamp(0.0, 1.0),
    };
    let anim = crate::types::ClipAnimation {
        from: clamp(&anim.from),
        to: clamp(&anim.to),
    };
    // Identity (no zoom, centred both ends) → None: emits nothing, byte-identical.
    let new = if anim.is_identity() { None } else { Some(anim) };
    // Mutually exclusive with a SCALE keyframe (both drive the same zoompan; the
    // keyframe channel is the multi-point eased form). Reject a non-clearing animate
    // when scale keyframes exist — the symmetric guard to edit::keyframe's. Clearing
    // (new=None) is always allowed. Keeps the "mutually exclusive" contract honest
    // from both directions, not just relying on the render's scale-takes-precedence.
    if new.is_some()
        && c.keyframes
            .iter()
            .any(|k| k.param == crate::types::KfParam::Scale)
    {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' already has a scale keyframe track; edit.animate and scale keyframes are mutually exclusive (the keyframe channel is the multi-point eased form)"),
            "clear the scale keyframes first: edit.keyframe {clip, param:'scale', points:[]}, then set edit.animate",
        )
        .with_clip(clip_id));
    }
    let old = c.animation.clone();
    c.animation = new.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_animation": old, "animation": new}),
    )])
}

/// Set (or clear) a media clip's KEYFRAMES for ONE parameter (verb `edit.keyframe`).
/// `points` REPLACES that param's keyframe track (SET semantics); an empty list
/// removes it. The renderer animates the param over the clip via an ffmpeg time-
/// expression (opacity → overlay alpha, volume → audio gain) — a keyframed param
/// OVERRIDES its static counterpart. Track validation: `opacity` needs a VIDEO-track
/// clip (animated on the overlay alpha), `volume` needs an AUDIO-track clip. Returns
/// the op effect (old + new keyframe lists) for the receipt/undo.
pub fn keyframe(
    project: &mut Project,
    clip_id: &str,
    param: crate::types::KfParam,
    mut points: Vec<crate::types::KfPoint>,
    interp: crate::types::KfInterp,
) -> Result<Vec<OpEffect>, CutError> {
    use crate::types::KfParam;
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let kind = project.track(&track_id).unwrap().kind;
    let timeline_duration_ms = project.track(&track_id).unwrap().clips[idx].timeline_duration_ms();
    let ok = match param {
        KfParam::Opacity | KfParam::PosX | KfParam::PosY | KfParam::Scale => {
            kind == TrackKind::Video
        }
        KfParam::Volume => kind == TrackKind::Audio,
    };
    if !ok {
        let need = match param {
            KfParam::Opacity => "a video-track clip (opacity animates the overlay alpha)",
            KfParam::PosX | KfParam::PosY => {
                "a video-track clip (pos_x/pos_y animate the overlay PiP position)"
            }
            KfParam::Scale => "a video-track clip (scale animates the clip zoom)",
            KfParam::Volume => "an audio-track clip (volume animates the audio gain)",
        };
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("keyframing '{param:?}' needs {need}"),
            "opacity → video clip, volume → audio clip",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "keyframes apply to media clips",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.keyframe"));
    }
    if let Some(point) = points
        .iter()
        .find(|point| point.t_ms > timeline_duration_ms)
    {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "keyframe at {}ms is beyond '{clip_id}' timeline duration of {timeline_duration_ms}ms",
                point.t_ms
            ),
            "keyframe times are clip-local timeline milliseconds after retiming; move the point within the clip duration",
        )
        .with_clip(clip_id));
    }
    // Scale keyframes and edit.animate (Ken Burns) BOTH drive a `zoompan` on a base
    // clip — having both on one clip is ambiguous (two competing zooms). The keyframe
    // channel is the richer (multi-point, eased) form, so reject it when a static
    // animation is already set (clear the animation first). Only enforced when there
    // are actually points to set (an empty Scale list is a CLEAR — always allowed).
    if matches!(param, KfParam::Scale) && !points.is_empty() && c.animation.is_some() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' already has an edit.animate Ken Burns zoom; scale keyframes and edit.animate are mutually exclusive (the keyframe channel is the multi-point eased form)"),
            "clear the animation first: edit.animate {clip, enabled:false}, then set the scale keyframes",
        )
        .with_clip(clip_id));
    }
    // Sort the control points by time (the renderer's expression assumes ordered
    // points); clamp opacity values into [0,1] (volume is an open multiplier; scale
    // is clamped to the zoompan-legal [1,10] at render, matching edit.animate).
    points.sort_by_key(|p| p.t_ms);
    if matches!(param, KfParam::Opacity) {
        for p in &mut points {
            p.value = p.value.clamp(0.0, 1.0);
        }
    }
    let old = c.keyframes.clone();
    // SET semantics: drop any existing track for this param, then add the new one
    // (empty points = just clear). Identity (no keyframes) → byte-identical replay.
    c.keyframes.retain(|k| k.param != param);
    if !points.is_empty() {
        c.keyframes.push(crate::types::Keyframe {
            param,
            points,
            interp,
        });
    }
    let new = c.keyframes.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "param": param, "old_keyframes": old, "keyframes": new}),
    )])
}

/// Set (or clear) a media clip's vector/freeform MASK (verb `edit.add_mask`). The
/// mask scopes an effect (blur/pixelate/black) to a shape REGION — the region-blur /
/// privacy-redaction primitive. `mask = None` CLEARS it (back to an un-masked clip,
/// byte-identical replay). Validates: VIDEO-track media clip; the right point count
/// for the shape (rect/ellipse = 2, polygon ≥ 3); non-negative feather. Returns the
/// op effect (old + new mask) for the receipt/undo.
pub fn add_mask(
    project: &mut Project,
    clip_id: &str,
    mask: Option<crate::types::ClipMask>,
) -> Result<Vec<OpEffect>, CutError> {
    use crate::types::MaskShape;
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on a video track"),
            "masks scope a picture effect — audio/caption clips have no frame",
        )
        .with_clip(clip_id));
    }
    // v1 SCOPE: masks render on the BASE (first) video track — the region-blur /
    // redaction use is the main footage. An overlay-track (PiP) clip would render
    // the mask as a silent no-op, so reject it with a clear message rather than
    // surprise the user. (Overlay-clip masks are a documented follow-up.) Clearing
    // (mask=None) is always allowed (lets you remove a mask regardless).
    let base_video = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .map(|t| t.id.as_str());
    if mask.is_some() && base_video != Some(track_id.as_str()) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is on an overlay video track; masks currently render only on the BASE (first) video track"),
            "move the clip to the base video track to mask it (overlay-clip masks are not yet supported)",
        )
        .with_clip(clip_id));
    }
    // Validate the geometry BEFORE mutating (so a bad mask leaves state untouched).
    if let Some(m) = &mask {
        let need = match m.shape {
            MaskShape::Rect => "rect needs 2 points (opposite corners)",
            MaskShape::Ellipse => "ellipse needs 2 points (centre, radii)",
            MaskShape::Polygon => "polygon needs at least 3 points (vertices)",
        };
        let ok = match m.shape {
            MaskShape::Rect | MaskShape::Ellipse => m.points.len() == 2,
            MaskShape::Polygon => m.points.len() >= 3,
        };
        if !ok {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "mask shape '{}' has {} points — {need}",
                    m.shape.as_str(),
                    m.points.len()
                ),
                need,
            )
            .with_clip(clip_id));
        }
        if m.feather < 0.0 || !m.feather.is_finite() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "mask feather must be a non-negative fraction of frame height",
                "feather 0 = hard edge; 0.02 ≈ a soft 2%-of-height edge",
            )
            .with_clip(clip_id));
        }
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "masks apply to media clips",
        )
        .with_clip(clip_id));
    };
    if c.speed_ramp.is_some() {
        return Err(ramp_conflict(clip_id, "edit.add_mask"));
    }
    let old = c.mask.clone();
    c.mask = mask.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_mask": old, "mask": mask}),
    )])
}

/// Set (or clear) a media clip's parametric audio EQ (verb `edit.eq`). The EQ is
/// a high-pass (low-cut, removes rumble) + peaking bands (presence boost / mud or
/// de-ess cut) + low-pass (high-cut, tames hiss) chain on the clip's audio — the
/// audio analog of [`grade`]. Targets AUDIO-track clips: the render's audio chain
/// only processes audio tracks, and a video file's audio is a SEPARATE clip on its
/// own audio track, so EQ-ing a picture-only video clip would store a no-op. An
/// identity EQ (no high/low-pass, every band ~0 dB) clears it → byte-identical
/// replay. Returns the op effect (old + new EQ) for the receipt/undo.
pub fn eq(
    project: &mut Project,
    clip_id: &str,
    mut eq: crate::types::ClipEq,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    if project.track(&track_id).unwrap().kind != TrackKind::Audio {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not on an audio track"),
            "EQ shapes audio tone — apply it to the clip on the audio track (a video \
             file's audio is a separate clip on its own audio track)",
        )
        .with_clip(clip_id));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "EQ applies to media clips",
        )
        .with_clip(clip_id));
    };
    // Clamp into sane audible ranges (belt-and-suspenders over the verb layer; the
    // renderer also tolerates these). Drop ~0 dB bands so the stored EQ is minimal
    // and the identity check is exact → byte-identical replay when nothing changes.
    for b in &mut eq.bands {
        b.q = b.q.clamp(0.1, 20.0);
        b.freq_hz = b.freq_hz.clamp(10.0, 20_000.0);
    }
    eq.bands.retain(|b| b.gain_db.abs() >= 1e-3);
    if let Some(hp) = &mut eq.high_pass_hz {
        *hp = hp.clamp(10.0, 20_000.0);
    }
    if let Some(lp) = &mut eq.low_pass_hz {
        *lp = lp.clamp(10.0, 20_000.0);
    }
    let new = if eq.is_identity() { None } else { Some(eq) };
    let old = c.eq.clone();
    c.eq = new.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_eq": old, "eq": new}),
    )])
}

/// Set a media clip's visual EFFECTS list (verb `edit.effect`), REPLACING any
/// existing effects (an empty list clears them). Validates that an overlay-only
/// effect — chroma key — is NOT placed on a BASE-track clip: chroma reveals a
/// LOWER track, and the base canvas has nothing under it. Returns the op effect
/// (old + new lists) for the receipt/undo.
pub fn set_effects(
    project: &mut Project,
    clip_id: &str,
    effects: Vec<crate::types::ClipEffect>,
) -> Result<Vec<OpEffect>, CutError> {
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(tr, i)| (tr.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let track_kind = project.track(&track_id).unwrap().kind;
    // Each effect must match the clip's track: VISUAL effects need pixels (a video
    // track); the AUDIO effect (denoise) needs an audio-track clip (it runs in the
    // audio chain). Captions never carry effects.
    for e in &effects {
        let ok = if e.is_audio() {
            track_kind == TrackKind::Audio
        } else {
            track_kind == TrackKind::Video
        };
        if !ok {
            let (need, why) = if e.is_audio() {
                (
                    "an audio",
                    "denoise cleans a clip's AUDIO — apply it to an audio-track clip",
                )
            } else {
                (
                    "a video",
                    "visual effects need pixels — audio/caption clips have none",
                )
            };
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("'{}' needs {need}-track clip", e.kind()),
                why,
            )
            .with_clip(clip_id));
        }
        // SECURITY: a chroma-key color is interpolated into the ffmpeg
        // filtergraph (chromakey=color=…). Restrict it to a color NAME or
        // 0xRRGGBB hex so a crafted value can't break out and inject filters
        // (e.g. ",movie=/etc/passwd"). See types::is_valid_chroma_color.
        if let crate::types::ClipEffect::ChromaKey { color, .. } = e {
            if !crate::types::is_valid_chroma_color(color) {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("invalid chroma key color '{color}'"),
                    "color must be a color name (e.g. green) or a 0xRRGGBB hex literal",
                )
                .with_clip(clip_id));
            }
        }
    }
    // Base canvas = first video track with clips. Chroma key on it would key to
    // nothing (no lower layer) — refuse with the fix (move to an overlay track).
    let is_base = project
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video && !t.clips.is_empty())
        .map(|t| t.id.as_str())
        == Some(track_id.as_str());
    if is_base {
        if let Some(e) = effects.iter().find(|e| e.is_overlay_only()) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("{} needs a layer below it", e.kind()),
                "chroma key reveals a LOWER track; the base track has nothing under it",
            )
            .with_clip(clip_id)
            .with_suggested_action(
                "add a video track ABOVE the background and place this clip there, then apply chroma key",
            ));
        }
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|x| x.id == track_id)
        .unwrap();
    let Clip::Media(c) = &mut track.clips[idx] else {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "effects apply to media clips",
        )
        .with_clip(clip_id));
    };
    let old = c.effects.clone();
    c.effects = effects.clone();
    Ok(vec![fx(
        Some(&track_id),
        json!({"clip": clip_id, "old_effects": old, "effects": effects}),
    )])
}

/// Set a crossfade on the cut at `at_ms` on `track` (verb `edit.crossfade`).
/// The cut at `at_ms` is the boundary between the clip ENDING there (the
/// LEFT clip) and the clip STARTING there (the RIGHT clip); the crossfade is
/// stored as `xfade_in_ms` on the RIGHT clip (it travels with that clip through
/// ripples, like a fade). `duration_ms = 0` CLEARS an existing crossfade at the
/// boundary (back to a hard cut). Both neighbours must be MEDIA clips with at
/// least `duration_ms` of their own length on the relevant side — the overlap
/// is taken from the left clip's tail and the right clip's head, so the
/// realized timeline shortens by `duration_ms` (the EDL computes the pullback;
/// the renderer emits `xfade`/`acrossfade`).
///
/// INTERACTION WITH edit.fade (documented contract): a crossfade OWNS the cut,
/// so setting one CLEARS the left clip's fade-OUT and the right clip's fade-IN
/// (a per-clip fade there would double-dip the dissolve). The clips' OTHER ends
/// keep their fades. Clearing the crossfade does NOT restore the cleared fades
/// (they were a separate, now-discarded decision — re-set them if wanted).
pub fn crossfade(
    project: &mut Project,
    track_id: &str,
    at_ms: u64,
    duration_ms: u64,
    transition: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let t = project
        .track(track_id)
        .ok_or_else(|| track_not_found(track_id))?;
    if t.kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "cannot crossfade a caption track",
            "captions don't dissolve — use captions.* verbs",
        ));
    }
    // Walk the cumulative track to find the cut at `at_ms`: the clip whose end
    // is exactly at_ms (LEFT) and the clip whose start is at_ms (RIGHT).
    //
    // COORDINATE SPACE (agent-facing trap — see the self-correcting error below):
    // this match is in EDITORIAL
    // time — the running sum of each clip's OWN duration. Editorial time is
    // crossfade-INDEPENDENT: a clip boundary keeps its at_ms no matter how many
    // crossfades shorten the realized timeline. But render.frame / render.preview
    // / the playhead report SHORTENED (xfade-pulled) time, so after a crossfade
    // the SAME seam shows at a smaller position there than the at_ms this verb
    // needs. An agent that reads a seam position off the render and passes it
    // here would miss the boundary — so we also record each media-cut's render
    // position and, on a miss, map the requested at_ms back to the editorial cut.
    let mut cursor: u64 = 0; // editorial cursor (clip-duration sum)
    let mut render_cursor: u64 = 0; // render/playhead cursor (xfade-shortened)
    let mut left_idx: Option<usize> = None;
    let mut right_idx: Option<usize> = None;
    // (editorial_at, render_at) for every media↔media cut — the only cuts a
    // crossfade can dissolve, and exactly the set worth suggesting on a miss.
    let mut media_cuts: Vec<(u64, u64)> = Vec::new();
    for (i, c) in t.clips.iter().enumerate() {
        let dur = c.timeline_duration_ms();
        // xfade only pulls the realized timeline back for a media clip with a
        // crossfade-in; clip 0 has no left neighbour so its stored xfade (if any)
        // is inert, matching edl_from_project's prev=None clamp.
        let xf = if i == 0 {
            0
        } else {
            match c {
                Clip::Media(m) => m.xfade_in_ms,
                _ => 0,
            }
        };
        let render_start = render_cursor.saturating_sub(xf);
        if i > 0 && matches!(c, Clip::Media(_)) && matches!(t.clips[i - 1], Clip::Media(_)) {
            // `cursor` is the editorial boundary between clip i-1 and clip i;
            // `render_start` is where clip i (hence the seam) begins on playback.
            media_cuts.push((cursor, render_start));
        }
        if cursor + dur == at_ms {
            left_idx = Some(i);
        }
        if cursor == at_ms {
            right_idx = Some(i);
        }
        cursor += dur;
        render_cursor = render_start + dur;
    }
    let (Some(li), Some(ri)) = (left_idx, right_idx) else {
        let action = if media_cuts.is_empty() {
            format!(
                "'{track_id}' has no adjacent media clips to dissolve; edit.split or edit.insert to create a media-to-media cut first"
            )
        } else if let Some(&(ed, _)) = media_cuts.iter().find(|&&(_, rnd)| rnd == at_ms) {
            // The requested at_ms IS a cut's render position → the caller almost
            // certainly read it off render.frame/preview. Point at the editorial
            // position this verb actually wants.
            format!(
                "{at_ms}ms is the render/playhead position of the cut at editorial {ed}ms (a crossfade shortened the timeline); retry edit.crossfade with at_ms={ed}"
            )
        } else {
            let list = media_cuts
                .iter()
                .map(|&(ed, rnd)| {
                    if ed == rnd {
                        ed.to_string()
                    } else {
                        format!("{ed} (renders at {rnd})")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let nearest = media_cuts
                .iter()
                .min_by_key(|&&(ed, _)| ed.abs_diff(at_ms))
                .unwrap()
                .0;
            format!(
                "media cuts on '{track_id}' at editorial ms: {list}; nearest to {at_ms} is {nearest} — retry edit.crossfade with at_ms={nearest}"
            )
        };
        return Err(CutError::new(
            codes::NOT_FOUND,
            format!("no cut between two clips at {at_ms}ms on '{track_id}'"),
            "a crossfade needs a clip ending AND another starting exactly at at_ms in EDITORIAL time (clip-duration sum); render/playhead time diverges after a crossfade shortens the timeline",
        )
        .with_at_ms(at_ms)
        .with_suggested_action(action));
    };
    // Both neighbours must be media clips (gaps/captions carry no pixels/audio
    // to dissolve). Collect their durations for the clamp + clear their
    // boundary fades, then set the crossfade on the right clip.
    let left_dur = t.clips[li].timeline_duration_ms();
    let right_dur = t.clips[ri].timeline_duration_ms();
    let (left_is_media, left_id) = match &t.clips[li] {
        Clip::Media(c) => (true, c.id.clone()),
        _ => (false, String::new()),
    };
    let (right_is_media, right_id) = match &t.clips[ri] {
        Clip::Media(c) => (true, c.id.clone()),
        _ => (false, String::new()),
    };
    if !left_is_media || !right_is_media {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("the cut at {at_ms}ms on '{track_id}' is not between two media clips"),
            "a crossfade dissolves two media clips; a gap on either side has nothing to dissolve",
        )
        .with_at_ms(at_ms));
    }
    if duration_ms > 0 && (duration_ms > left_dur || duration_ms > right_dur) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "crossfade {duration_ms}ms exceeds a neighbour ('{left_id}' {left_dur}ms, '{right_id}' {right_dur}ms)"
            ),
            "the overlap is taken from each clip — it cannot be longer than either",
        )
        .with_at_ms(at_ms)
        .with_suggested_action("shorten the crossfade or lengthen the shorter clip (edit.trim)"));
    }
    // Clear the left clip's fade-OUT (the crossfade owns this boundary).
    let mut cleared: Vec<serde_json::Value> = Vec::new();
    if let Clip::Media(c) = &mut project.track_mut(track_id).unwrap().clips[li] {
        if let Some(fade) = &c.fade {
            if fade.out_ms > 0 {
                cleared
                    .push(json!({"clip": left_id, "cleared": "fade_out", "was_ms": fade.out_ms}));
                let new = (fade.in_ms > 0).then_some(ClipFade {
                    in_ms: fade.in_ms,
                    out_ms: 0,
                    kind: fade.kind,
                });
                c.fade = new;
            }
        }
    }
    // Set the crossfade on the right clip + clear its fade-IN.
    let old_xfade;
    if let Clip::Media(c) = &mut project.track_mut(track_id).unwrap().clips[ri] {
        old_xfade = c.xfade_in_ms;
        c.xfade_in_ms = duration_ms;
        // Transition STYLE: applied with the dissolve; removing it (duration 0)
        // also clears the style. Normalize "fade" → None so a plain dissolve stays
        // byte-identical to pre-transitions op logs.
        c.xfade_kind = if duration_ms > 0 {
            match transition {
                Some(t) if t != "fade" => Some(t.to_string()),
                _ => None,
            }
        } else {
            None
        };
        if let Some(fade) = &c.fade {
            if fade.in_ms > 0 {
                cleared.push(json!({"clip": right_id, "cleared": "fade_in", "was_ms": fade.in_ms}));
                let new = (fade.out_ms > 0).then_some(ClipFade {
                    in_ms: 0,
                    out_ms: fade.out_ms,
                    kind: fade.kind,
                });
                c.fade = new;
            }
        }
    } else {
        unreachable!("right clip confirmed media above");
    }
    // edit.duck windows carry absolute timeline ranges. The crossfade pulls
    // the right clip (and everything after the seam) back by `duration_ms`, so
    // duck windows on THIS track after the seam must remap or the bed ducks the
    // wrong moment. Mirror the ripple_delete remap: model the seam as removing
    // `duration_ms` of timeline at [at_ms - duration_ms, at_ms]. (Windows on a
    // sibling track aren't touched here; single-track crossfade leaving an A/V
    // pair unequal is surfaced as a warning at dispatch.)
    let mut windows_remapped = 0u64;
    if duration_ms > 0 {
        let lo = at_ms.saturating_sub(duration_ms);
        let tr = project.track_mut(track_id).unwrap();
        tr.gain_windows.retain_mut(|w| {
            let ns = ripple_map(w.range_ms[0], lo, at_ms);
            let ne = ripple_map(w.range_ms[1], lo, at_ms);
            if ne <= ns {
                return false; // window collapsed into the dissolve — drop it
            }
            if [ns, ne] != w.range_ms {
                windows_remapped += 1;
            }
            w.range_ms = [ns, ne];
            true
        });
    }
    Ok(vec![fx(
        Some(track_id),
        json!({
            "at_ms": at_ms,
            "left_clip": left_id,
            "right_clip": right_id,
            "old_xfade_ms": old_xfade,
            "xfade_ms": duration_ms,
            // Echo the applied transition style (the canonical "fade" when unset or
            // cleared) so the agent's receipt names what it got.
            "transition": if duration_ms > 0 {
                transition.filter(|t| *t != "fade").unwrap_or("fade")
            } else {
                "fade"
            },
            "fades_cleared": cleared,
            "duck_windows_remapped": windows_remapped,
        }),
    )])
}

/// Reposition an existing marker (verb `edit.move_marker`). One op,
/// the marker id is PRESERVED (a remove+add would mint a new id and break any
/// reference to it). Records the old and new positions for the audit trail.
pub fn marker_move(
    project: &mut Project,
    marker_id: &str,
    at_ms: u64,
) -> Result<Vec<OpEffect>, CutError> {
    let m = project
        .markers
        .iter_mut()
        .find(|m| m.id == marker_id)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no marker '{marker_id}'"),
                "marker ids are listed in project.state markers[]",
            )
        })?;
    let old = m.at_ms;
    m.at_ms = at_ms;
    Ok(vec![fx(
        None,
        json!({"marker_id": marker_id, "old_at_ms": old, "at_ms": at_ms}),
    )])
}

/// Re-label, re-color, and/or edit a marker note (verb `edit.update_marker`).
/// One op, id + position PRESERVED. `label`/`color`/`note`
/// are independent: pass any one or combination; `color: "none"` clears back
/// to default, and a blank note clears the note.
/// SYSTEM markers are refused: the beat grid (`beat`) and capture markers
/// (`capture:`-prefixed) are machine-written and consumed BY LABEL (markerClass
/// in the UI, audio.add_music's beat grid) — renaming one would silently
/// corrupt that contract.
pub fn marker_update(
    project: &mut Project,
    marker_id: &str,
    label: Option<&str>,
    color: Option<&str>,
    note: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    if label.is_none() && color.is_none() && note.is_none() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "edit.update_marker needs `label`, `color`, and/or `note`".to_string(),
            "pass a new label, a color (or \"none\" to clear), a note, or a combination",
        ));
    }
    if let Some(l) = label {
        if l.trim().is_empty() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                "marker label must not be empty".to_string(),
                "use edit.remove_marker to delete a marker",
            ));
        }
    }
    if let Some(c) = color {
        if c != "none" && !crate::types::MARKER_COLORS.contains(&c) {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("unknown marker color '{c}'"),
                format!(
                    "valid colors: {} (or \"none\" to clear)",
                    crate::types::MARKER_COLORS.join(", ")
                ),
            ));
        }
    }
    let m = project
        .markers
        .iter_mut()
        .find(|m| m.id == marker_id)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no marker '{marker_id}'"),
                "marker ids are listed in project.state markers[]",
            )
        })?;
    if m.label == "beat" || m.label.starts_with("capture:") {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("marker '{marker_id}' is a system marker ('{}')", m.label),
            "beat/capture markers are machine-managed; add your own marker instead",
        ));
    }
    let old_label = m.label.clone();
    let old_color = m.color.clone();
    let old_note = m.note.clone();
    if let Some(l) = label {
        m.label = l.to_string();
    }
    if let Some(c) = color {
        m.color = if c == "none" {
            None
        } else {
            Some(c.to_string())
        };
    }
    if let Some(n) = note {
        let trimmed = n.trim();
        m.note = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    Ok(vec![fx(
        None,
        json!({
            "marker_id": marker_id,
            "old_label": old_label, "label": m.label,
            "old_color": old_color, "color": m.color,
            "old_note": old_note, "note": m.note,
        }),
    )])
}

/// Set a caption clip's absolute timeline range (verb `captions.set_range`).
/// Covers both retime (shift both edges) and trim (move one edge) for
/// direct manipulation of caption clips, which `edit.move`/`edit.trim` refuse
/// (captions carry absolute ranges, not source ranges). Validates the range is
/// non-empty; the clip must be a caption clip.
pub fn caption_set_range(
    project: &mut Project,
    clip_id: &str,
    range_ms: [u64; 2],
) -> Result<Vec<OpEffect>, CutError> {
    if range_ms[0] >= range_ms[1] {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "range_ms [{}, {}) is empty or inverted",
                range_ms[0], range_ms[1]
            ),
            "range start must be strictly less than range end",
        )
        .with_clip(clip_id));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    match &mut t.clips[idx] {
        Clip::Caption(c) => {
            let old = c.range_ms;
            c.range_ms = range_ms;
            Ok(vec![fx(
                Some(&track_id),
                json!({"clip": clip_id, "old_range_ms": old, "range_ms": range_ms}),
            )])
        }
        _ => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a caption clip"),
            "captions.set_range moves caption clips; use edit.move/edit.trim for media clips",
        )
        .with_clip(clip_id)),
    }
}

/// captions.set_text{clip, text, style_ref?} — replace an EXISTING caption clip's
/// text (and optionally its style_ref) IN PLACE, by clip id. The companion to
/// [`caption_set_range`] (retime): together they make a placed caption fully
/// editable from the Inspector, closing the "select a caption → can't edit it"
/// gap (caption-editing regression — `captions.add_text` only ADDS a new
/// caption; nothing EDITED an existing one). Errors if the clip is not a caption.
pub fn caption_set_text(
    project: &mut Project,
    clip_id: &str,
    text: &str,
    style_ref: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "caption text is empty",
            "captions.set_text replaces a caption's words — pass non-empty text",
        )
        .with_clip(clip_id));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    match &mut t.clips[idx] {
        Clip::Caption(c) => {
            let old_text = c.text.clone();
            let old_style = c.style_ref.clone();
            c.text = trimmed.to_string();
            if let Some(s) = style_ref {
                c.style_ref = Some(s.to_string());
            }
            Ok(vec![fx(
                Some(&track_id),
                json!({"clip": clip_id, "old_text": old_text, "text": trimmed,
                       "old_style_ref": old_style, "style_ref": c.style_ref}),
            )])
        }
        _ => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a caption clip"),
            "captions.set_text edits caption clips; media clips have no editable text",
        )
        .with_clip(clip_id)),
    }
}

/// edit.set_asset{clip, asset} — repoint an EXISTING media clip at a DIFFERENT
/// registered asset id IN PLACE, keeping the clip's id, source range, track,
/// position, and per-clip edits (effects/transform/fade/…). It is the core step
/// behind `title.update` (title-editing regression): editing a placed title's
/// text re-renders a fresh transparent overlay `.mov`, registers it as a new
/// asset, then swaps the title clip onto that asset — the clip stays exactly
/// where it is, only its pixels change. NOT a top-level verb; it only ever rides
/// as a lowered step of `title.update` (like `edit.insert` inside `title.add`),
/// so the swap replays deterministically (the new `.mov` persists on disk and
/// its `media.import` op re-registers the asset before this step re-runs).
///
/// Errors if the clip is unknown, the clip is not a MEDIA clip (captions/gaps
/// carry no asset), or the target asset is not registered.
pub fn clip_set_asset(
    project: &mut Project,
    clip_id: &str,
    asset_id: &str,
) -> Result<Vec<OpEffect>, CutError> {
    // The asset must already be registered (record_import runs before this on
    // the live path; the media.import op replays before this lowered step on
    // the replay path). A defensive check turns any mis-ordering into a clean
    // error instead of a dangling clip→asset reference.
    if !project.assets.contains_key(asset_id) {
        return Err(CutError::new(
            codes::NOT_FOUND,
            format!("no asset '{asset_id}'"),
            "edit.set_asset repoints a clip at a REGISTERED asset; import it first",
        )
        .with_clip(clip_id));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    match &mut t.clips[idx] {
        Clip::Media(c) => {
            let old_asset = c.asset.clone();
            c.asset = asset_id.to_string();
            Ok(vec![fx(
                Some(&track_id),
                json!({"clip": clip_id, "old_asset": old_asset, "asset": asset_id}),
            )])
        }
        _ => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "edit.set_asset repoints media clips; captions/gaps have no asset",
        )
        .with_clip(clip_id)),
    }
}

/// Apply timed gain-reduction windows to an audio track (verb `edit.duck`).
/// REPLACES the track's existing windows (re-running a duck after timeline
/// changes is the documented refresh path — types.rs GainWindow note: this
/// is windowed gain, not a sidechain compressor). The windows are computed
/// by the CALLER (dispatch maps the against-track's perception silences
/// through the EDL) and arrive fully resolved so the recorded op is
/// self-contained and replay needs no perception files.
pub fn duck(
    project: &mut Project,
    track: &str,
    windows: Vec<GainWindow>,
) -> Result<Vec<OpEffect>, CutError> {
    let t = project
        .track_mut(track)
        .ok_or_else(|| track_not_found(track))?;
    if t.kind != TrackKind::Audio {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{track}' is not an audio track"),
            "ducking lowers audio gain — it applies to audio tracks only",
        ));
    }
    for w in &windows {
        if w.range_ms[0] >= w.range_ms[1] {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "gain window [{}, {}) is empty or inverted",
                    w.range_ms[0], w.range_ms[1]
                ),
                "window start must be strictly less than window end",
            ));
        }
        if w.db >= 0.0 {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("gain window db must be negative (got {})", w.db),
                "ducking is a reduction; use edit.gain for static boosts",
            ));
        }
    }
    let replaced = t.gain_windows.len();
    let detail = json!({
        "ducked_windows": windows.len(),
        "replaced_windows": replaced,
        "windows": windows,
    });
    t.gain_windows = windows;
    Ok(vec![fx(Some(track), detail)])
}

/// Add a new (empty) video or audio track (verb `edit.add_track`). Needed
/// for compositing workflows — a music bed or overlay needs a second track,
/// and project.create only makes v1/a1t (multi-track audio and compositing regressions were
/// unreachable without this verb). Ids are deterministic per the existing
/// naming convention — video "v{N}", audio "a{N}t" — so replay reproduces
/// them; an explicit `id` may override (must be unique). Caption tracks are
/// managed by captions.* verbs, not here.
pub fn add_track(
    project: &mut Project,
    kind: TrackKind,
    id: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    if kind == TrackKind::Caption {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "cannot add a caption track with edit.add_track",
            "caption tracks are created on demand by captions.generate / captions.add_text",
        ));
    }
    let track_id = match id {
        Some(explicit) if !explicit.is_empty() => {
            if project.track(explicit).is_some() {
                return Err(CutError::new(
                    codes::CONFLICT,
                    format!("track '{explicit}' already exists"),
                    "track ids must be unique",
                )
                .with_suggested_action("omit id for automatic allocation"));
            }
            explicit.to_string()
        }
        _ => {
            // Deterministic allocation: max existing index of the kind's
            // naming pattern + 1 (pure function of project state — replay
            // allocates the same id the live path did).
            let next = |prefix: &str, suffix: &str| {
                project
                    .tracks
                    .iter()
                    .filter_map(|t| {
                        t.id.strip_prefix(prefix)?
                            .strip_suffix(suffix)?
                            .parse::<u64>()
                            .ok()
                    })
                    .max()
                    .unwrap_or(0)
                    + 1
            };
            match kind {
                TrackKind::Video => format!("v{}", next("v", "")),
                TrackKind::Audio => format!("a{}t", next("a", "t")),
                TrackKind::Caption => unreachable!("guarded above"),
            }
        }
    };
    // GROUPED INSERT (track grouping): keep all VIDEO tracks contiguous, then
    // all AUDIO tracks, then any CAPTION tracks — `[Video…, Audio…, Caption…]`.
    // A naive tail `push` interleaves lanes when an overlay video and its linked
    // audio are added in turn (`[v1, a1t, v2, a2t]`), which the UI then renders
    // interleaved. Inserting at the group boundary keeps the timeline tidy.
    //
    // INVARIANT (load-bearing): VIDEO-track Vec order == compositing z-order
    // (first non-empty video = base canvas, later videos overlay on top — render.rs /
    // edl.rs base_video_track). `group_insert_index` puts a new video AFTER the
    // last existing video, so it lands on TOP of the prior videos — exactly the
    // old tail-append overlay rule, just confined to the video group. Audio-track
    // order is a render/EDL no-op (all audio mixes regardless), so an audio
    // insert position is cosmetic only.
    let at = group_insert_index(&project.tracks, kind);
    project.tracks.insert(
        at,
        Track {
            id: track_id.clone(),
            kind,
            clips: vec![],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        },
    );
    Ok(vec![fx(
        Some(&track_id),
        json!({"added_track": track_id, "kind": kind}),
    )])
}

/// Kind ordering rank for track grouping: VIDEO(0) < AUDIO(1) < CAPTION(2). The
/// target Vec layout is `[Video…, Audio…, Caption…]`. Used by both the grouped
/// insert (`group_insert_index`) and the load-time heal (`normalize_track_order`)
/// so they agree on one canonical order.
fn track_kind_rank(kind: TrackKind) -> u8 {
    match kind {
        TrackKind::Video => 0,
        TrackKind::Audio => 1,
        TrackKind::Caption => 2,
    }
}

/// Absolute Vec index at which to insert a new track of `kind` so the result
/// stays grouped `[Video…, Audio…, Caption…]`. Returns the position just AFTER
/// the last track whose kind-rank is ≤ this kind's rank:
///   • Video → after the last Video track (front-of-everything if no videos);
///     a new video lands on TOP of existing videos (preserves the overlay rule).
///   • Audio → after the last Audio track, or after the last Video track when no
///     audio exists yet (end of the audio group, BEFORE any caption tracks).
///   • Caption → after the last Caption track, i.e. the very end.
/// One rule covers every "no tracks of my kind yet" case correctly, so audio
/// always slots in after video and before captions even on a video-only project.
fn group_insert_index(tracks: &[Track], kind: TrackKind) -> usize {
    let rank = track_kind_rank(kind);
    // Walk to the first track that belongs AFTER this kind's group; insert there.
    tracks
        .iter()
        .position(|t| track_kind_rank(t.kind) > rank)
        .unwrap_or(tracks.len())
}

/// Remove a track entirely (verb `edit.remove_track`). Closes the "you can add
/// tracks but never delete them" gap — an emptied overlay/music lane left a dead
/// row on the timeline with no way to clear it
/// like overlay" after deleting the clip). Guards timeline validity: refuses to
/// remove the LAST video or LAST audio track (the renderer needs a base of each),
/// and refuses a NON-EMPTY track unless `force` (so a stray click can't silently
/// drop content). A recompute-by-replay undo restores the pre-removal timeline,
/// so the removed track and its clips come back together.
pub fn remove_track(
    project: &mut Project,
    track_id: &str,
    force: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let idx = project
        .tracks
        .iter()
        .position(|t| t.id == track_id)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("track '{track_id}' not found"),
                "project.state lists track ids",
            )
        })?;
    let kind = project.tracks[idx].kind;
    let kind_name = match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Caption => "caption",
    };
    let clip_count = project.tracks[idx].clips.len();
    // Keep at least one VIDEO and one AUDIO track — the renderer composes a base
    // of each kind. (Caption tracks have no base requirement: zero is valid.)
    if matches!(kind, TrackKind::Video | TrackKind::Audio) {
        let same_kind = project.tracks.iter().filter(|t| t.kind == kind).count();
        if same_kind <= 1 {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("cannot remove '{track_id}' — it is the only {kind_name} track"),
                "the timeline needs at least one video and one audio track",
            ));
        }
    }
    if clip_count > 0 && !force {
        return Err(CutError::new(
            codes::CONFLICT,
            format!("track '{track_id}' has {clip_count} clip(s)"),
            "removing the track would drop them",
        )
        .with_suggested_action(
            "remove the clips first, or pass force:true to remove the track and its clips",
        ));
    }
    project.tracks.remove(idx);
    Ok(vec![fx(
        Some(track_id),
        json!({"removed_track": track_id, "kind": kind, "clips_dropped": clip_count}),
    )])
}

/// Move a track to a new STACKING position WITHIN ITS OWN KIND (verb
/// `edit.reorder_track`). Video track order in `project.tracks` IS the
/// compositing z-order — the first video track with clips is the base canvas;
/// each later video track overlays on top. This verb changes a track's position
/// so the user can bring a layer FORWARD (a higher index) or send it BACK.
///
/// CONTRACT: `to_index` is GROUP-RELATIVE — it is the target position among
/// tracks of the SAME kind (0 = first track of that kind), NOT an absolute
/// `project.tracks` index. We translate it to the absolute Vec index internally.
/// This keeps track grouping intact: a video can never be reordered into the
/// audio group (which would corrupt `[Video…, Audio…, Caption…]` and the z-order
/// invariant). The clamp is to `[0, same_kind_count-1]`. For the only current
/// caller — the Layer panel, which reorders VIDEO tracks and where the video
/// group is the Vec prefix — group index == absolute index, so this is a no-op
/// translation there. Reordering audio/caption tracks is a render no-op (the
/// renderer filters by kind) but allowed and kept deterministic. Records the
/// resolved ABSOLUTE from/to so replay reproduces the exact move.
pub fn reorder_track(
    project: &mut Project,
    track_id: &str,
    to_index: usize,
) -> Result<Vec<OpEffect>, CutError> {
    let from = project
        .tracks
        .iter()
        .position(|t| t.id == track_id)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("track '{track_id}' not found"),
                "project.state lists track ids",
            )
        })?;
    let kind = project.tracks[from].kind;
    // Translate the group-relative target into an absolute Vec index by walking
    // the same-kind tracks. Clamp the group index to the size of this kind's
    // group, then map it to the absolute index of the same-kind track currently
    // occupying that group slot — so the moved track stays inside its group and
    // the other kinds' tracks never shift.
    let same_kind: Vec<usize> = project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == kind)
        .map(|(i, _)| i)
        .collect();
    // same_kind is non-empty (`track_id` itself is in it) → group_len >= 1.
    let group_target = to_index.min(same_kind.len() - 1);
    let to = same_kind[group_target];
    if from != to {
        let t = project.tracks.remove(from);
        project.tracks.insert(to, t);
    }
    Ok(vec![fx(
        Some(track_id),
        // Do NOT put a `track` key in the detail: OpEffect.track is a real field
        // and `detail` is #[serde(flatten)], so a `track` detail key serializes a
        // DUPLICATE top-level `track` and breaks every path that re-reads the op
        // log ("duplicate field `track`", e.g. the composed-frame rebuild). The
        // effect's own `track` field already carries the moved track id.
        json!({"from": from, "to": to}),
    )])
}

/// Set (or clear) a video track's LAYER blend mode (verb `edit.blend`). The whole
/// overlay track composites onto everything below it with `mode`
/// (multiply/screen/overlay/…); `"normal"` (or omitted) clears it to the default
/// alpha-over composite. VIDEO tracks only (audio/caption tracks have nothing to
/// blend; the first non-empty video track is the base canvas, so a blend there is
/// a no-op but allowed). Returns the op effect (old + new mode) for the receipt/undo.
pub fn set_track_blend(
    project: &mut Project,
    track_id: &str,
    mode: &str,
) -> Result<Vec<OpEffect>, CutError> {
    if !crate::types::is_valid_blend_mode(mode) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("unknown blend mode '{mode}'"),
            "mode must be one of: normal, multiply, screen, overlay, darken, lighten, \
             difference, addition, subtract, softlight, hardlight",
        ));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("track '{track_id}' not found"),
                "project.state lists track ids",
            )
        })?;
    if track.kind != TrackKind::Video {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("track '{track_id}' is not a video track"),
            "blend modes composite video layers — audio/caption tracks have nothing to blend",
        ));
    }
    let old = track.blend_mode.clone();
    // "normal"/empty → None (default composite); else store the mode.
    track.blend_mode = if mode == "normal" || mode.is_empty() {
        None
    } else {
        Some(mode.to_string())
    };
    let new = track.blend_mode.clone();
    Ok(vec![fx(
        Some(track_id),
        json!({"old_blend_mode": old, "blend_mode": new}),
    )])
}

/// Set a visual track's output visibility (verb `edit.track_visible {track, on}`).
///
/// `on = false` hides a video/caption track from preview/export without deleting
/// clips or changing layer order. Audio tracks intentionally use `edit.mute`
/// instead, so the UI does not expose two different controls for one audio
/// outcome. The flag is persisted on `Track.visible`; old projects default true.
pub fn set_track_visible(
    project: &mut Project,
    track_id: &str,
    on: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let track = project
        .track_mut(track_id)
        .ok_or_else(|| track_not_found(track_id))?;
    if track.kind == TrackKind::Audio {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("track '{track_id}' is an audio track — use mute instead"),
            "visibility applies to video/caption output; audio tracks use edit.mute",
        ));
    }
    let old = track.visible;
    track.visible = on;
    Ok(vec![fx(
        Some(track_id),
        json!({"old_visible": old, "visible": on}),
    )])
}

/// Set a track's UI edit lock (verb `edit.track_lock {track, on}`).
///
/// Locking is persisted project state that the timeline UI uses to block
/// accidental drag/trim/drop edits on the track. It does not remove selection or
/// hide output; it is an edit guard, not a render flag.
pub fn set_track_locked(
    project: &mut Project,
    track_id: &str,
    on: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let track = project
        .track_mut(track_id)
        .ok_or_else(|| track_not_found(track_id))?;
    let old = track.locked;
    track.locked = on;
    Ok(vec![fx(
        Some(track_id),
        json!({"old_locked": old, "locked": on}),
    )])
}

/// Set a track's NON-DESTRUCTIVE MUTE flag (verb `edit.mute {track, on}`).
///
/// `on = true` silences the track in the audio mix; `on = false` un-mutes it. The
/// track's `gain_db` is NEVER touched — mute is a flag, so a dialed-in level
/// survives mute/unmute and a reload (this REPLACES the old -100 dB-gain-write
/// mute, which destroyed the level on reload). Audibility is resolved at mix time
/// by [`crate::types::Project::audio_track_audible`]. Idempotent: setting the flag
/// to its current value still records an op (old == new) so the op-log/undo rail
/// stays consistent. Only audio tracks contribute to the render mix; video and
/// caption tracks are refused instead of storing a no-op flag.
pub fn set_track_muted(
    project: &mut Project,
    track_id: &str,
    on: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let track = audio_track_mut(project, track_id, "mute")?;
    let old = track.muted;
    track.muted = on;
    // NOTE: the track id rides on OpEffect.track (the fx() arg) — it must NOT also
    // go in the detail map, which serde-FLATTENS onto OpEffect: a `track` detail key
    // collides with OpEffect.track and corrupts the op record (duplicate field on
    // re-read → op-log replay fails). Same rule edit.reorder_track follows.
    Ok(vec![fx(
        Some(track_id),
        json!({"old_muted": old, "muted": on}),
    )])
}

/// Set a track's NON-DESTRUCTIVE SOLO flag (verb `edit.solo {track, on}`).
///
/// `on = true` solos the track; `on = false` clears its solo. When ANY track is
/// soloed, only soloed tracks are audible in the mix (everything else contributes
/// silence) — resolved at mix time by [`crate::types::Project::audio_track_audible`],
/// never by writing gain. Like mute, `gain_db` is untouched and the flag survives a
/// reload. Non-audio tracks are refused. (An explicit mute still wins over solo:
/// a muted+soloed track stays silent.)
pub fn set_track_solo(
    project: &mut Project,
    track_id: &str,
    on: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let track = audio_track_mut(project, track_id, "solo")?;
    let old = track.solo;
    track.solo = on;
    // The track id rides on OpEffect.track (not the flattened detail) — see the note
    // in set_track_muted: a `track` detail key collides with OpEffect.track.
    Ok(vec![fx(
        Some(track_id),
        json!({"old_solo": old, "solo": on}),
    )])
}

/// Set a track's NON-DESTRUCTIVE stereo PAN/balance (verb
/// `edit.pan {track, pan}`). `pan` ∈ [−1.0, +1.0]: −1 = full left, 0 = center, +1 = full
/// right. BALANCE semantics (the mixer-knob model for stereo material): center is
/// unity — the renderer emits NO filter at 0.0, so an untouched mix stays
/// byte-identical — and panning ATTENUATES the opposite channel on a cosine taper
/// (never boosts → no clipping risk). Resolved at the mix stage after concat/duck
/// ([`cut_media` build_graph]), so render, preview, and export.audio stems agree.
/// Like mute/solo: independent of `gain_db`, survives reload. Non-audio tracks are
/// refused. NaN/∞ and out-of-range values are refused, not clamped —
/// an agent typo should fail loudly, not silently hard-pan.
pub fn set_track_pan(
    project: &mut Project,
    track_id: &str,
    pan: f64,
) -> Result<Vec<OpEffect>, CutError> {
    if !pan.is_finite() || !(-1.0..=1.0).contains(&pan) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("pan {pan} is out of range"),
            "pan must be between -1.0 (full left) and 1.0 (full right); 0 = center",
        ));
    }
    let track = audio_track_mut(project, track_id, "pan")?;
    let old = track.pan;
    track.pan = pan;
    // The track id rides on OpEffect.track (not the flattened detail) — see the note
    // in set_track_muted: a `track` detail key collides with OpEffect.track.
    Ok(vec![fx(
        Some(track_id),
        json!({"old_pan": old, "pan": pan}),
    )])
}

/// Non-destructive MUTE range on one media clip (verb
/// `edit.mute_range {clip, range_ms?|clear}`), the low-level half of mute-word;
/// `transcript.mute_words` resolves words → these ranges). `range_ms` is in
/// SOURCE-ASSET time (the src_in/src_out clock) so the mute stays glued to the
/// spoken content through trims/slips/splits — see the field doc on
/// [`MediaClip::mute_ranges`]. ADD one range (normalized: sorted + merged with
/// any overlap/adjacency) or CLEAR the whole list.
///
/// Refusals: clip must be a MEDIA clip on an AUDIO track (only audio tracks
/// reach the mix — muting a video-track clip would "succeed" inaudibly, which
/// is a lie; target the linked audio clip or edit.detach_audio first). The
/// range must be non-empty and intersect the clip's CURRENT visible source
/// window (a mute the clip can never play is almost certainly a caller bug).
/// A speed-RAMPED clip is refused (the renderer's source→output mapping is
/// linear; a ramp would silently drift the gate off the word). Plain speed and
/// reverse are fine — the renderer maps/mirrors exactly.
pub fn mute_range(
    project: &mut Project,
    clip_id: &str,
    range_ms: Option<[u64; 2]>,
    remove_ms: Option<[u64; 2]>,
    clear: bool,
) -> Result<Vec<OpEffect>, CutError> {
    let modes =
        usize::from(range_ms.is_some()) + usize::from(remove_ms.is_some()) + usize::from(clear);
    if modes != 1 {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "edit.mute_range needs exactly one of `range_ms`, `remove_ms`, or `clear:true`",
            "range_ms adds a mute, remove_ms surgically unmutes an interval, clear removes all",
        ));
    }
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let track_kind = project
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.kind)
        .unwrap();
    if track_kind != TrackKind::Audio {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("clip '{clip_id}' is on {track_id} ({track_kind:?}) — only audio tracks reach the mix"),
            "a mute range on a non-audio clip would change nothing audible",
        )
        .with_clip(clip_id)
        .with_suggested_action(
            "target the linked audio clip (same asset on an audio track), or edit.detach_audio first",
        ));
    }
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    match &mut t.clips[idx] {
        Clip::Media(c) => {
            if c.speed_ramp.is_some() {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("clip '{clip_id}' has a speed ramp — mute ranges need a linear source→output mapping"),
                    "a piecewise-ramped mapping would silently drift the mute gate off the word",
                )
                .with_clip(clip_id)
                .with_suggested_action("remove the ramp (edit.speed_ramp) or mute before ramping"));
            }
            let old_count = c.mute_ranges.len();
            let action = if clear {
                "clear"
            } else if remove_ms.is_some() {
                "remove"
            } else {
                "add"
            };
            if clear {
                c.mute_ranges.clear();
            } else if let Some(r) = remove_ms {
                // Surgical unmute: subtract [r0, r1) from every stored range —
                // a range fully inside disappears, an overlap shrinks, a strict
                // superset SPLITS into two. Other mutes are untouched (a clear
                // that wiped them would be a UX lie).
                if r[1] <= r[0] {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        format!("remove range [{}, {}) is empty or inverted", r[0], r[1]),
                        "remove_ms must be [start_ms, end_ms) with end > start (SOURCE time)",
                    )
                    .with_clip(clip_id));
                }
                let mut next: Vec<[u64; 2]> = Vec::with_capacity(c.mute_ranges.len() + 1);
                for m in c.mute_ranges.drain(..) {
                    if m[1] <= r[0] || m[0] >= r[1] {
                        next.push(m); // no overlap — untouched
                    } else {
                        if m[0] < r[0] {
                            next.push([m[0], r[0]]); // left remainder
                        }
                        if m[1] > r[1] {
                            next.push([r[1], m[1]]); // right remainder
                        }
                    }
                }
                c.mute_ranges = next;
            } else {
                let r = range_ms.unwrap();
                if r[1] <= r[0] {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        format!("mute range [{}, {}) is empty or inverted", r[0], r[1]),
                        "range_ms must be [start_ms, end_ms) with end > start (SOURCE time)",
                    )
                    .with_clip(clip_id));
                }
                if r[1] <= c.src_in_ms || r[0] >= c.src_out_ms {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        format!(
                            "mute range [{}, {}) does not intersect the clip's visible source window [{}, {})",
                            r[0], r[1], c.src_in_ms, c.src_out_ms
                        ),
                        "mute ranges are in SOURCE-asset ms (the src_in/src_out clock), not timeline ms",
                    )
                    .with_clip(clip_id)
                    .with_suggested_action(
                        "use transcript.mute_words for word addressing, or convert timeline→source time first",
                    ));
                }
                c.mute_ranges.push(r);
                // Normalize: sorted + overlap/adjacency merged — canonical state,
                // minimal render expressions, idempotent re-adds.
                c.mute_ranges.sort_unstable();
                let mut merged: Vec<[u64; 2]> = Vec::with_capacity(c.mute_ranges.len());
                for r in c.mute_ranges.drain(..) {
                    match merged.last_mut() {
                        Some(last) if r[0] <= last[1] => last[1] = last[1].max(r[1]),
                        _ => merged.push(r),
                    }
                }
                c.mute_ranges = merged;
            }
            Ok(vec![fx(
                Some(&track_id),
                json!({
                    "id": clip_id,
                    "action": action,
                    "range_ms": range_ms.or(remove_ms),
                    "mute_ranges": c.mute_ranges,
                    "old_count": old_count,
                }),
            )])
        }
        _ => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("'{clip_id}' is not a media clip"),
            "mute ranges apply to media clips",
        )
        .with_clip(clip_id)),
    }
}

/// Add a non-destructive ADJUSTMENT LAYER over a time span (verb `edit.adjustment`).
/// Stores a [`crate::types::Adjustment`] (grade + look effects, gated to `range_ms`)
/// in `project.adjustments`; the renderer applies it as a TIME-GATED pass on the
/// composite of everything beneath it (v1: the top-most layer). NOT a per-clip edit —
/// no clip is mutated or split, so it composes/undoes/replays as a single layer.
///
/// Validates: `range_ms` is a non-empty span; at least one of `grade` (non-identity)
/// / `effects` is present (an empty layer is refused); effects are VISUAL look effects
/// only — audio effects (denoise/compressor/gate) and chroma-key are refused (an
/// adjustment grades a composite: it has no audio chain and no single layer below to
/// key out). The layer id is allocated deterministically (`adj{N}` = max existing
/// index + 1) so replay re-derives the same id; it is recorded in the effect.
pub fn add_adjustment(
    project: &mut Project,
    range_ms: [u64; 2],
    grade: Option<crate::types::ClipGrade>,
    effects: Vec<crate::types::ClipEffect>,
) -> Result<Vec<OpEffect>, CutError> {
    use crate::types::ClipEffect as E;
    if range_ms[1] <= range_ms[0] {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "adjustment range [{}, {}) is empty or inverted",
                range_ms[0], range_ms[1]
            ),
            "range_ms must be [start_ms, end_ms) with end > start",
        )
        .with_suggested_action("pass range_ms = [start, end] in ms, end strictly after start"));
    }
    // An identity grade carries nothing; treat it as absent so "grade-or-effect"
    // means "something will actually render".
    let grade = grade.filter(|g| !g.is_identity());
    if grade.is_none() && effects.is_empty() {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "adjustment layer has neither a grade nor an effect",
            "an adjustment must carry a non-identity grade and/or at least one effect",
        )
        .with_suggested_action(
            "pass grade:{saturation:0,…} (the color band) and/or effect(s):[{type:\"vignette\",…}]",
        ));
    }
    // Reject effects that make no sense on a composite grade band: audio effects
    // (the adjustment has no audio chain) and chroma-key (it keys ONE layer to reveal
    // the one below — an adjustment grades the whole stack beneath it, there is no
    // single layer to key). Caught here so a bad effect never reaches the renderer.
    for e in &effects {
        match e {
            E::Denoise { .. } | E::Compressor { .. } | E::Gate { .. } => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "an adjustment layer cannot carry an AUDIO effect",
                    "denoise / compressor / gate are audio effects — an adjustment grades video",
                )
                .with_suggested_action(
                    "apply audio effects per-clip with edit.effect on an audio clip",
                ));
            }
            E::ChromaKey { .. } => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "an adjustment layer cannot carry chroma_key",
                    "chroma_key keys ONE layer to reveal the one below — an adjustment grades the \
                     whole composite beneath it (nothing single to key)",
                )
                .with_suggested_action("chroma-key the overlay clip itself with edit.effect"));
            }
            _ => {}
        }
    }
    // Deterministic id: max existing adjN index + 1 (pure function of project state →
    // replay re-derives it). Recorded in the effect for the receipt / undo rail.
    let n = project
        .adjustments
        .iter()
        .filter_map(|a| a.id.strip_prefix("adj").and_then(|x| x.parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
        + 1;
    let id = format!("adj{n}");
    let adj = crate::types::Adjustment {
        id: id.clone(),
        range_ms,
        grade: grade.clone(),
        effects: effects.clone(),
    };
    project.adjustments.push(adj);
    Ok(vec![fx(
        None,
        json!({
            "adjustment_id": id,
            "range_ms": range_ms,
            "grade": grade,
            "effects": effects,
        }),
    )])
}

/// Add a marker at `at_ms` (the two-segment verb-name contract: verb `edit.add_marker`). Marker ids
/// are deterministic "mN" allocations so replay reproduces them exactly.
pub fn marker_add(
    project: &mut Project,
    at_ms: u64,
    label: &str,
    note: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    marker_add_pinned(project, at_ms, label, note, None)
}

/// Id-pinning variant of [`marker_add`]. `pinned_id` forces the marker id
/// (effect `added_marker.id`) instead of allocating positionally (`mN`), so a
/// skip-replay keeps marker ids stable when an earlier op is rebased out
/// (rebase.rs). Live path passes None. No-skip replay's pinned id equals the
/// positional one (byte-identical).
pub fn marker_add_pinned(
    project: &mut Project,
    at_ms: u64,
    label: &str,
    note: Option<&str>,
    pinned_id: Option<&str>,
) -> Result<Vec<OpEffect>, CutError> {
    let id = match pinned_id {
        Some(p) => p.to_string(),
        None => {
            let n = project
                .markers
                .iter()
                .filter_map(|m| m.id.strip_prefix('m').and_then(|x| x.parse::<u64>().ok()))
                .max()
                .unwrap_or(0);
            format!("m{}", n + 1)
        }
    };
    let marker = Marker {
        id,
        at_ms,
        label: label.to_string(),
        note: note.map(String::from),
        color: None,
    };
    let detail = json!({"added_marker": marker});
    project.markers.push(marker);
    Ok(vec![fx(None, detail)])
}

/// Remove marker by id (the two-segment verb-name contract: verb `edit.remove_marker`). The removed
/// marker is recorded in full in the effects so the op stays inspectable.
pub fn marker_remove(project: &mut Project, marker_id: &str) -> Result<Vec<OpEffect>, CutError> {
    let idx = project
        .markers
        .iter()
        .position(|m| m.id == marker_id)
        .ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no marker '{marker_id}'"),
                "marker ids are listed in project.state markers[]",
            )
        })?;
    let removed = project.markers.remove(idx);
    Ok(vec![fx(None, json!({"removed_marker": removed}))])
}

/// Replay one historic snapshot-era restore by applying its recorded inverse.
///
/// Current restore calls recompute a journal prefix in `ProjectStore::apply`
/// and record `restored_timeline`; only a legacy `edit.restore` lacking that
/// materialized result reaches this compatibility function.
pub fn restore(
    project: &mut Project,
    original: &crate::ops::OpRecord,
) -> Result<Vec<OpEffect>, CutError> {
    let inv = original.inverse.as_ref().ok_or_else(|| {
        CutError::new(
            codes::INVALID_ARGS,
            format!(
                "op '{}' ({}) is not undoable",
                original.op_id, original.verb
            ),
            "the op record carries no inverse (e.g. project.checkpoint, media.import)",
        )
    })?;
    match inv.verb.as_str() {
        "edit._set_timeline" => {
            apply_set_timeline(project, &inv.args)?;
            Ok(vec![fx(
                None,
                json!({"restored_op": original.op_id, "undid_verb": original.verb}),
            )])
        }
        other => Err(CutError::new(
            codes::INVALID_ARGS,
            format!("unknown inverse verb '{other}' on op '{}'", original.op_id),
            "this core build only understands edit._set_timeline inverses",
        )),
    }
}

/// Apply the internal timeline snapshot verb. Shared by restore() (live path)
/// and store::apply_record (replay path). Project-level timeline state fields
/// are optional so snapshots written before each extension still apply unchanged.
pub(crate) fn apply_set_timeline(
    project: &mut Project,
    args: &serde_json::Value,
) -> Result<(), CutError> {
    #[derive(serde::Deserialize)]
    struct Snapshot {
        tracks: Vec<Track>,
        markers: Vec<Marker>,
        #[serde(default)]
        caption_styles: Option<std::collections::BTreeMap<String, crate::types::CaptionStyle>>,
        #[serde(default)]
        adjustments: Option<Vec<crate::types::Adjustment>>,
        #[serde(default)]
        nests: Option<Vec<crate::types::Nest>>,
        #[serde(default)]
        transcript_ignores: Option<Vec<crate::types::TranscriptIgnore>>,
    }
    let s: Snapshot = serde_json::from_value(args.clone())?;
    validate_timeline_media_ranges(&s.tracks)?;
    project.tracks = s.tracks;
    project.markers = s.markers;
    if let Some(styles) = s.caption_styles {
        project.caption_styles = styles;
    }
    if let Some(adjustments) = s.adjustments {
        project.adjustments = adjustments;
    }
    if let Some(nests) = s.nests {
        project.nests = nests;
    }
    if let Some(transcript_ignores) = s.transcript_ignores {
        project.transcript_ignores = transcript_ignores;
    }
    Ok(())
}

fn validate_timeline_media_ranges(tracks: &[Track]) -> Result<(), CutError> {
    for track in tracks {
        for clip in &track.clips {
            if let Clip::Media(c) = clip {
                if c.src_in_ms > c.src_out_ms {
                    return Err(CutError::new(
                        codes::INVALID_ARGS,
                        "malformed timeline: media clip source range is inverted",
                        format!(
                            "clip '{}' on track '{}' has src_in_ms {} > src_out_ms {}",
                            c.id, track.id, c.src_in_ms, c.src_out_ms
                        ),
                    )
                    .with_clip(&c.id)
                    .with_suggested_action("repair the timeline snapshot before replaying it"));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// shared error constructors
// ---------------------------------------------------------------------------

pub(crate) fn track_not_found(id: &str) -> CutError {
    CutError::new(
        codes::NOT_FOUND,
        format!("no track '{id}'"),
        "track ids are listed in project.state tracks[]",
    )
}

pub(crate) fn clip_not_found(id: &str) -> CutError {
    CutError::new(
        codes::NOT_FOUND,
        format!("no clip '{id}'"),
        "clip ids are listed in project.state tracks[].clips[]",
    )
    .with_clip(id)
}

fn caption_on_non_caption_track(track: &str) -> CutError {
    CutError::new(
        codes::INVALID_ARGS,
        "malformed timeline: caption clip on non-caption track",
        format!("track '{track}' is not a caption track but contains a caption clip"),
    )
    .with_suggested_action("repair the project timeline so captions live on caption tracks")
}

/// Refuse an operation `op` on a clip that carries a variable-speed ramp
/// (edit.speed_ramp). The ramp is realized as constant-speed sub-segments whose
/// SOURCE-offset control points and non-linear timeline map are incompatible with
/// retime / re-cut / per-frame-baked features — clear the ramp first. Shared by
/// the verbs that would otherwise mis-map or silently drop the combined feature.
pub(crate) fn ramp_conflict(clip_id: &str, op: &str) -> CutError {
    CutError::new(
        codes::INVALID_ARGS,
        format!(
            "clip '{clip_id}' has a variable-speed ramp; {op} is not supported on a ramped clip"
        ),
        "a speed ramp warps the clip's timeline↔source mapping non-linearly, so this \
         operation cannot be applied correctly while the ramp is set",
    )
    .with_clip(clip_id)
    .with_suggested_action(
        "clear the ramp first (edit.speed_ramp{clip, points:[]}), apply this, then re-ramp",
    )
}

#[cfg(test)]
mod tests;
