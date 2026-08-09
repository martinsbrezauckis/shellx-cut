use super::*;

pub(in crate::dispatch) async fn edit_split_at_scenes(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: String,
        track: Option<String>,
        min_shot_ms: Option<u64>,
    }
    let a: Args = parse_args(args)?;
    let min_shot = a.min_shot_ms.unwrap_or(500);
    // Load the asset's detected scene cuts (source time).
    let scenes: Vec<u64> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let report =
            cut_perception::load_report(&store.receipts_dir(), &a.asset)?.ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no perception report for '{}'", a.asset),
                    "scene cuts come from the scenes instrument",
                )
                .with_suggested_action("run media.perception first (or wait for the import chain)")
            })?;
        report.scenes.iter().map(|s| s.at_ms).collect()
    };
    // Map each source scene boundary to a timeline split position via the clip
    // that carries it on the target track.
    let (track, mut positions): (String, Vec<u64>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let track = match &a.track {
            Some(t) => t.clone(),
            None => store
                .project
                .tracks
                .iter()
                .find(|t| t.kind == cut_core::TrackKind::Video)
                .map(|t| t.id.clone())
                .ok_or_else(no_project)?,
        };
        let edl = cut_core::edl_from_project(&store.project);
        let mut positions = Vec::new();
        for sc in &scenes {
            for seg in edl.track_segments(&track) {
                let (Some(si), Some(so)) = (seg.src_in_ms, seg.src_out_ms) else {
                    continue;
                };
                if seg.asset.as_deref() == Some(a.asset.as_str()) && *sc > si && *sc < so {
                    // Scene cut is a source position; map to timeline via speed.
                    let tl = seg.timeline_in_ms + cut_core::src_off_to_tl(*sc - si, seg.speed);
                    // keep min_shot away from both clip edges
                    if tl >= seg.timeline_in_ms + min_shot && tl + min_shot <= seg.timeline_out_ms {
                        positions.push(tl);
                    }
                }
            }
        }
        (track, positions)
    };
    positions.sort_unstable();
    positions.dedup();
    // Enforce min_shot BETWEEN consecutive cuts too.
    let mut kept: Vec<u64> = Vec::new();
    for p in positions {
        if kept.last().is_none_or(|&last| p >= last + min_shot) {
            kept.push(p);
        }
    }
    let mut op_ids: Vec<String> = Vec::new();
    let mut splits = 0u64;
    for pos in &kept {
        let args = json!({"track": track, "at_ms": pos, "rationale": "edit.split_at_scenes: detected scene cut"});
        // A split at an existing boundary / gap is idempotent; every other
        // failure is a real edit failure and must be surfaced.
        match commit_core(state, "edit.split", args, actor.clone()).await {
            Ok(r) => {
                if let Some(ids) = r.op_ids {
                    op_ids.extend(ids);
                }
                splits += 1;
            }
            Err(e) if is_benign_split_miss(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(VerbResult::ok_with_ops(
        json!({
            "asset": a.asset,
            "track": track,
            "scene_cuts": scenes.len(),
            "splits": splits,
        }),
        op_ids,
    ))
}

/// edit.mark_scenes{asset, track?, label_prefix?} — add a marker at each detected
/// SCENE boundary (timeline). Complements edit.split_at_scenes (navigate instead
/// of cut) and composes with export.chapters for automatic chapter generation
/// from scenes. One edit.add_marker op per scene boundary on the timeline.
/// edit.trim_edges{keep_pad_ms?, min_trim_ms?} — top-and-tail: remove leading and
/// trailing DEAD AIR (everything before the first spoken word / after the last)
/// while preserving the internal pacing (unlike transcript.remove_silences,
/// which removes EVERY silence). Anchors to SPEECH (word timings mapped to the
/// timeline), leaving `keep_pad_ms` of breath on each side. The dedicated fix
/// for the silence_at_edges render check. Two ripple_deletes (trailing first so
/// the leading range stays valid). Silent footage → an honest no-op note.
pub(in crate::dispatch) async fn edit_trim_edges(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        keep_pad_ms: Option<u64>,
        min_trim_ms: Option<u64>,
    }
    let a: Args = parse_args(args)?;
    let keep_pad = a.keep_pad_ms.unwrap_or(200);
    let min_trim = a.min_trim_ms.unwrap_or(250);

    // Find the timeline extent of SPEECH: first word start, last word end,
    // across every asset's transcript mapped through the EDL.
    let asset_ids: Vec<String> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.project.assets.keys().cloned().collect()
    };
    let mut first_speech = u64::MAX;
    let mut last_speech = 0u64;
    let mut any_words = false;
    for id in &asset_ids {
        let Ok(t) = load_transcript(state, id).await else {
            continue;
        };
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        for w in &t.words {
            for r in
                speech_text::source_to_timeline(&store.project, id, [w.start_ms, w.end_ms], None)
            {
                any_words = true;
                first_speech = first_speech.min(r[0]);
                last_speech = last_speech.max(r[1]);
            }
        }
    }
    let timeline_ms = {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?.project.duration_ms()
    };
    if !any_words {
        return Ok(VerbResult::ok(json!({
            "leading_trimmed_ms": 0,
            "trailing_trimmed_ms": 0,
            "note": "no speech on the timeline to anchor edges (silent footage) — nothing trimmed",
        })));
    }

    // Leading dead air = [0, first_speech - pad]; trailing = [last_speech + pad,
    // end]. Each trimmed only if it exceeds min_trim.
    let leading_end = first_speech.saturating_sub(keep_pad);
    let trailing_start = (last_speech + keep_pad).min(timeline_ms);
    let do_leading = leading_end >= min_trim;
    let do_trailing = timeline_ms.saturating_sub(trailing_start) >= min_trim;

    let mut op_ids: Vec<String> = Vec::new();
    let mut trailing_trimmed = 0u64;
    let mut leading_trimmed = 0u64;
    let group_id = if do_leading && do_trailing {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        Some(format!("grp-trim_edges-{}", store.log.next_id()?))
    } else {
        None
    };
    // TRAILING first: deleting the tail does not shift earlier content, so the
    // leading range stays valid for the second ripple.
    if do_trailing {
        let r = commit_core(
            state,
            "edit.ripple_delete",
            json!({
                "range_ms": [trailing_start, timeline_ms],
                "rationale": "trim edges: remove trailing dead air",
                "group_id": group_id,
            }),
            actor.clone(),
        )
        .await?;
        if let Some(ids) = r.op_ids {
            op_ids.extend(ids);
        }
        trailing_trimmed = timeline_ms - trailing_start;
    }
    if do_leading {
        let r = commit_core(
            state,
            "edit.ripple_delete",
            json!({
                "range_ms": [0, leading_end],
                "rationale": "trim edges: remove leading dead air",
                "group_id": group_id,
            }),
            actor.clone(),
        )
        .await?;
        if let Some(ids) = r.op_ids {
            op_ids.extend(ids);
        }
        leading_trimmed = leading_end;
    }
    Ok(VerbResult::ok_with_ops(
        json!({
            "leading_trimmed_ms": leading_trimmed,
            "trailing_trimmed_ms": trailing_trimmed,
            "first_speech_ms": first_speech,
            "last_speech_ms": last_speech,
        }),
        op_ids,
    ))
}

pub(in crate::dispatch) async fn edit_mark_scenes(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: String,
        track: Option<String>,
        label_prefix: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let prefix = a.label_prefix.unwrap_or_else(|| "Scene".into());
    let scenes: Vec<u64> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let report =
            cut_perception::load_report(&store.receipts_dir(), &a.asset)?.ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no perception report for '{}'", a.asset),
                    "scene cuts come from the scenes instrument",
                )
                .with_suggested_action("run media.perception first")
            })?;
        report.scenes.iter().map(|s| s.at_ms).collect()
    };
    let mut positions: Vec<u64> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let track = match &a.track {
            Some(t) => t.clone(),
            None => store
                .project
                .tracks
                .iter()
                .find(|t| t.kind == cut_core::TrackKind::Video)
                .map(|t| t.id.clone())
                .ok_or_else(no_project)?,
        };
        let edl = cut_core::edl_from_project(&store.project);
        let mut positions = Vec::new();
        for sc in &scenes {
            for seg in edl.track_segments(&track) {
                let (Some(si), Some(so)) = (seg.src_in_ms, seg.src_out_ms) else {
                    continue;
                };
                if seg.asset.as_deref() == Some(a.asset.as_str()) && *sc >= si && *sc < so {
                    // Scene source position → timeline marker via clip speed.
                    positions
                        .push(seg.timeline_in_ms + cut_core::src_off_to_tl(*sc - si, seg.speed));
                }
            }
        }
        positions
    };
    positions.sort_unstable();
    positions.dedup();
    let mut op_ids: Vec<String> = Vec::new();
    for (i, pos) in positions.iter().enumerate() {
        let args = json!({"at_ms": pos, "label": format!("{prefix} {}", i + 1)});
        if let Ok(r) = commit_core(state, "edit.add_marker", args, actor.clone()).await {
            if let Some(ids) = r.op_ids {
                op_ids.extend(ids);
            }
        }
    }
    Ok(VerbResult::ok_with_ops(
        json!({"asset": a.asset, "scene_cuts": scenes.len(), "markers_added": op_ids.len()}),
        op_ids,
    ))
}

pub(in crate::dispatch) async fn edit_split(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        track: String,
        at_ms: u64,
    }
    let _a: Args = parse_args(args.clone())?; // arg-shape validation up front
    commit_core(state, "edit.split", args, actor).await
}

/// Skip a beat sitting within this many ms of an existing cut (or a beat already
/// taken) so `edit.cut_to_beat` never mints a sub-frame sliver. ~half a 30fps
/// frame — tight enough to keep distinct beats (≥ ~250ms apart even at 240 BPM),
/// loose enough to coalesce a beat that effectively lands on a cut.
const BEAT_CUT_EPSILON_MS: u64 = 16;

/// Default `snap` window: how far a cut boundary may travel to reach a beat.
/// ~3.5 frames @30fps — locks a loosely-assembled sequence without yanking a
/// cut that was deliberately placed off the grid.
const BEAT_SNAP_DEFAULT_MS: u64 = 120;

fn is_benign_split_miss(err: &CutError) -> bool {
    err.code == error_codes::NOT_FOUND
        && (err.message.contains("no clip under")
            || err.message.contains("inside a gap")
            || err.cause.contains("position is on a clip boundary"))
}

/// edit.cut_to_beat{track?, mode?, every_n?, range_ms?, max_snap_ms?, rationale?}
/// — cut/align clips to MUSIC BEATS (a beat-synced montage:
/// each clip change lands on a beat).
///
/// BEAT SOURCE (v1): the `beat:N` markers `audio.add_music{beat_markers:true}`
/// surfaces from the bed asset's perception `BeatGrid` (stored as markers with
/// `label == "beat"`, the index N in the note, `at_ms` = the TIMELINE position
/// the beat maps to). No beat markers → an actionable error pointing at
/// audio.add_music (transcript-onset / downbeat detection are documented future
/// sources). `every_n` (default 1) thins to every Nth beat (slower cutting);
/// `range_ms` limits the span.
///
/// mode:"split" (DEFAULT) — split the chosen track (default the base video
/// track) at each selected beat inside its content, so every cut lands on a beat
/// (the montage skeleton the user then fills/reorders). Beats on an existing cut
/// (±epsilon) or in a gap are skipped — no slivers, idempotent re-run.
///
/// mode:"snap" — ROLL each existing cut boundary to the nearest beat within
/// `max_snap_ms`, locking an already-assembled sequence to the grid. A roll
/// trims the left clip's out-edge and the right clip's in-edge by the SAME
/// timeline delta (a classic NLE roll), so only the boundary moves — downstream
/// timing is preserved. Boundaries are never pushed past a neighbour, and a roll
/// that lacks source headroom on either side is skipped (reported honestly).
///
/// The pure beat-selection logic lives in `cut_core::beatsync` (unit-tested);
/// this handler is scope + lowering + receipt. v1 lowers to individual,
/// replay-safe `edit.split` / `edit.trim` ops — one per cut/edge, exactly like
/// edit.split_at_scenes — rather than a single batched op (a lowered op with
/// MANY id-allocating splits is NOT yet replay-stable: PinnedIds carries one
/// `split_right`, so reopen replay would collide their ids). Receipt:
/// {mode, track, cuts:[at_ms], beats_used, every_n, beats_available, ...}.
pub(in crate::dispatch) async fn edit_cut_to_beat(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        /// Track to cut/align. Default = the base video track.
        track: Option<String>,
        /// "split" (default) | "snap".
        mode: Option<String>,
        /// Use every Nth beat (default 1). 2 = every 2nd beat, 4 = every bar.
        every_n: Option<usize>,
        /// Limit the cut/align span to [r0, r1) timeline ms.
        range_ms: Option<[u64; 2]>,
        /// snap mode: max ms a boundary may move to reach a beat (default 120).
        max_snap_ms: Option<u64>,
    }
    let a: Args = parse_args(args.clone())?;
    let mode = a.mode.as_deref().unwrap_or("split").to_lowercase();
    if mode != "split" && mode != "snap" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown mode '{mode}'"),
            "mode must be \"split\" (default) or \"snap\"",
        ));
    }
    let every_n = a.every_n.unwrap_or(1).max(1);
    let max_snap_ms = a.max_snap_ms.unwrap_or(BEAT_SNAP_DEFAULT_MS);
    if let Some([r0, r1]) = a.range_ms {
        if r0 >= r1 {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("range_ms [{r0}, {r1}) is empty or inverted"),
                "range start must be < end",
            ));
        }
    }

    // What the read phase resolves: the track, its beat-aligned plan, and the
    // facts the receipt reports. Splits carry cut positions; snaps carry the
    // per-clip trims to commit + the accepted boundary moves.
    struct SplitPlan {
        track: String,
        cuts: Vec<u64>,
        beats_available: usize,
    }
    struct ClipTrim {
        clip: String,
        new_in: u64,
        new_out: u64,
        in_changed: bool,
        out_changed: bool,
    }
    struct SnapPlan {
        track: String,
        trims: Vec<ClipTrim>,
        moves: Vec<cut_core::beatsync::BeatSnap>,
        beats_available: usize,
    }
    enum Plan {
        Split(SplitPlan),
        Snap(SnapPlan),
    }

    let plan: Plan = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;

        // BEAT SOURCE: the audio.add_music `beat:N` markers (label == "beat",
        // at_ms = the TIMELINE position). Deduped + sorted by the picker.
        let beats: Vec<u64> = project
            .markers
            .iter()
            .filter(|m| m.label == "beat")
            .map(|m| m.at_ms)
            .collect();
        if beats.is_empty() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                "no beat markers on the timeline",
                "add a music bed with beats first: audio.add_music{asset, beat_markers:true} \
                 (transcript-onset / downbeat detection are future beat sources)",
            )
            .with_suggested_action(
                "call audio.add_music{asset:<music>, beat_markers:true} on a beat-bearing asset",
            ));
        }
        let beats_available = {
            let mut b = beats.clone();
            b.sort_unstable();
            b.dedup();
            b.len()
        };

        let edl = cut_core::edl_from_project(project);
        // Resolve the target track: explicit arg, else the base video track.
        let track = match &a.track {
            Some(t) => {
                if project.track(t).is_none() {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        format!("no track '{t}'"),
                        "pass an existing track id, or omit `track` to use the base video track",
                    ));
                }
                t.clone()
            }
            None => match edl.base_video_track() {
                Some(t) => t.to_string(),
                // No video content at all → a clean no-op rather than an error.
                None => {
                    return Ok(VerbResult::ok_with_ops(
                        json!({
                            "mode": mode,
                            "track": Value::Null,
                            "cuts": [],
                            "beats_used": 0,
                            "every_n": every_n,
                            "beats_available": beats_available,
                            "note": "no video track with clips — nothing to cut",
                        }),
                        vec![],
                    ));
                }
            },
        };

        // Media segments on the track, in timeline order. asset.is_some() drops
        // gaps; these define the track's content extent + the existing cuts.
        let media: Vec<&cut_core::EdlSegment> = edl
            .track_segments(&track)
            .filter(|s| s.asset.is_some())
            .collect();
        if media.is_empty() {
            // Track exists but holds no clips in this view → clean no-op.
            return Ok(VerbResult::ok_with_ops(
                json!({
                    "mode": mode,
                    "track": track,
                    "cuts": [],
                    "beats_used": 0,
                    "every_n": every_n,
                    "beats_available": beats_available,
                    "note": "no clips on the track in range — nothing to cut",
                }),
                vec![],
            ));
        }
        let extent = (
            media.first().unwrap().timeline_in_ms,
            media.last().unwrap().timeline_out_ms,
        );
        // Existing cut points (timeline) on the track.
        let existing_cuts: Vec<u64> = media
            .iter()
            .flat_map(|s| [s.timeline_in_ms, s.timeline_out_ms])
            .collect();

        match mode.as_str() {
            "split" => {
                let cuts = cut_core::beatsync::pick_split_cuts(
                    &beats,
                    &existing_cuts,
                    extent,
                    a.range_ms,
                    every_n,
                    BEAT_CUT_EPSILON_MS,
                );
                Plan::Split(SplitPlan {
                    track,
                    cuts,
                    beats_available,
                })
            }
            _ => {
                // Internal boundaries = media-segment edges minus the program
                // edges (extent.0 / extent.1).
                let mut boundaries: Vec<u64> = existing_cuts
                    .iter()
                    .copied()
                    .filter(|&p| p != extent.0 && p != extent.1)
                    .collect();
                boundaries.sort_unstable();
                boundaries.dedup();
                let moves = cut_core::beatsync::snap_boundaries(
                    &boundaries,
                    &beats,
                    extent,
                    a.range_ms,
                    every_n,
                    max_snap_ms,
                );

                // Lower each accepted move to a ROLL: trim the left clip's
                // out-edge and the right clip's in-edge by the same timeline
                // delta. Per-clip planned ranges accumulate so a clip touched on
                // BOTH edges (a middle clip) gets one combined trim; a roll that
                // overruns its source headroom is skipped (honest under-count).
                let asset_dur = |asset: &str| -> u64 {
                    project
                        .assets
                        .get(asset)
                        .and_then(|x| x.probe.as_ref())
                        .and_then(|p| p.get("duration_ms"))
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::MAX) // unknown duration → don't block the roll
                };
                // clip_id → (base_in, base_out, new_in, new_out, asset_dur)
                use std::collections::HashMap;
                let mut planned: HashMap<String, [u64; 5]> = HashMap::new();
                let mut accepted: Vec<cut_core::beatsync::BeatSnap> = Vec::new();
                for mv in &moves {
                    let delta = mv.to as i64 - mv.from as i64;
                    let left = media.iter().find(|s| s.timeline_out_ms == mv.from);
                    let right = media.iter().find(|s| s.timeline_in_ms == mv.from);
                    let (Some(l), Some(r)) = (left, right) else {
                        continue; // a gap on one side → can't roll this boundary
                    };
                    let (Some(lid), Some(rid)) = (l.clip_id.clone(), r.clip_id.clone()) else {
                        continue;
                    };
                    let l_state = *planned.entry(lid.clone()).or_insert_with(|| {
                        [
                            l.src_in_ms.unwrap_or(0),
                            l.src_out_ms.unwrap_or(0),
                            l.src_in_ms.unwrap_or(0),
                            l.src_out_ms.unwrap_or(0),
                            asset_dur(l.asset.as_deref().unwrap_or("")),
                        ]
                    });
                    let r_state = *planned.entry(rid.clone()).or_insert_with(|| {
                        [
                            r.src_in_ms.unwrap_or(0),
                            r.src_out_ms.unwrap_or(0),
                            r.src_in_ms.unwrap_or(0),
                            r.src_out_ms.unwrap_or(0),
                            asset_dur(r.asset.as_deref().unwrap_or("")),
                        ]
                    });
                    // Source deltas at each clip's own speed (round, signed).
                    let d_l = (delta as f64 * l.speed).round() as i64;
                    let d_r = (delta as f64 * r.speed).round() as i64;
                    let new_out_l = l_state[3] as i64 + d_l;
                    let new_in_r = r_state[2] as i64 + d_r;
                    // Headroom: left keeps ≥1 source ms within its asset; right
                    // keeps ≥1 source ms and a non-negative in-point.
                    let l_ok = new_out_l > l_state[2] as i64 && new_out_l as u64 <= l_state[4];
                    let r_ok = new_in_r >= 0 && new_in_r < r_state[3] as i64;
                    if !l_ok || !r_ok {
                        continue; // skip — would collapse a clip or run off source
                    }
                    planned.get_mut(&lid).unwrap()[3] = new_out_l as u64;
                    planned.get_mut(&rid).unwrap()[2] = new_in_r as u64;
                    accepted.push(*mv);
                }
                let trims: Vec<ClipTrim> = planned
                    .into_iter()
                    .filter_map(|(clip, st)| {
                        let in_changed = st[2] != st[0];
                        let out_changed = st[3] != st[1];
                        (in_changed || out_changed).then_some(ClipTrim {
                            clip,
                            new_in: st[2],
                            new_out: st[3],
                            in_changed,
                            out_changed,
                        })
                    })
                    .collect();
                Plan::Snap(SnapPlan {
                    track,
                    trims,
                    moves: accepted,
                    beats_available,
                })
            }
        }
    };

    // ---- commit phase: one replay-safe op per cut / per rolled clip ----------
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    match plan {
        Plan::Split(p) => {
            let mut op_ids: Vec<String> = Vec::new();
            let mut done: Vec<u64> = Vec::new();
            for &at in &p.cuts {
                let sargs = json!({
                    "track": p.track,
                    "at_ms": at,
                    "rationale": rationale.clone()
                        .unwrap_or_else(|| "edit.cut_to_beat: align cut to music beat".into()),
                });
                // A beat landing in a gap / on a boundary is idempotent; every
                // other split failure is real and must not be hidden.
                match commit_core(state, "edit.split", sargs, actor.clone()).await {
                    Ok(r) => {
                        if let Some(ids) = r.op_ids {
                            op_ids.extend(ids);
                        }
                        done.push(at);
                    }
                    Err(e) if is_benign_split_miss(&e) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(VerbResult::ok_with_ops(
                json!({
                    "mode": "split",
                    "track": p.track,
                    "cuts": done,
                    "beats_used": done.len(),
                    "every_n": every_n,
                    "beats_available": p.beats_available,
                }),
                op_ids,
            ))
        }
        Plan::Snap(p) => {
            let mut op_ids: Vec<String> = Vec::new();
            for t in &p.trims {
                let mut targs = serde_json::Map::new();
                targs.insert("clip".into(), json!(t.clip));
                if t.in_changed {
                    targs.insert("src_in_ms".into(), json!(t.new_in));
                }
                if t.out_changed {
                    targs.insert("src_out_ms".into(), json!(t.new_out));
                }
                targs.insert(
                    "rationale".into(),
                    json!(rationale
                        .clone()
                        .unwrap_or_else(|| "edit.cut_to_beat: snap cut to music beat".into())),
                );
                let r =
                    commit_core(state, "edit.trim", Value::Object(targs), actor.clone()).await?;
                if let Some(ids) = r.op_ids {
                    op_ids.extend(ids);
                }
            }
            let cuts: Vec<u64> = p.moves.iter().map(|m| m.to).collect();
            let moves: Vec<Value> = p
                .moves
                .iter()
                .map(|m| json!({"from": m.from, "to": m.to}))
                .collect();
            Ok(VerbResult::ok_with_ops(
                json!({
                    "mode": "snap",
                    "track": p.track,
                    "cuts": cuts,
                    "moves": moves,
                    "beats_used": p.moves.len(),
                    "every_n": every_n,
                    "beats_available": p.beats_available,
                }),
                op_ids,
            ))
        }
    }
}

pub(in crate::dispatch) async fn edit_ripple_delete(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        track: Option<String>,
        range_ms: [u64; 2],
        // Ripple (close gap, default) vs lift (leave gap). Validated
        // here for arg-shape; core re-parses with the replay-safe default true.
        ripple: Option<bool>,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.ripple_delete", args, actor).await
}

pub(in crate::dispatch) async fn edit_trim(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        src_in_ms: Option<u64>,
        src_out_ms: Option<u64>,
        linked: Option<bool>,
    }
    let a: Args = parse_args(args.clone())?;
    let linked = a.linked.unwrap_or(true);
    let mut normalized_args = args;
    if let Some(obj) = normalized_args.as_object_mut() {
        obj.insert("linked".into(), json!(linked));
    }
    let linked_media = resolve_linked_media(state, &a.clip, linked, "edit.trim").await?;
    if let Some(link) = linked_media {
        return commit_lowered(
            state,
            "edit.trim",
            normalized_args,
            actor,
            link.trim_steps(&a.clip, a.src_in_ms, a.src_out_ms),
            vec![effect(
                None,
                json!({"linked_trim": {"clip": link.clip, "track": link.track}}),
            )],
        )
        .await;
    }
    commit_core(state, "edit.trim", normalized_args, actor).await
}

pub(in crate::dispatch) async fn edit_move(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        to_track: String,
        at_ms: u64,
        // AV-sync ripple at the destination (default false = float).
        ripple: Option<bool>,
        // Default true for live calls. Old logged moves have no lowered pair
        // and therefore retain their historical one-clip replay semantics.
        linked: Option<bool>,
    }
    let a: Args = parse_args(args.clone())?;
    let ripple = a.ripple.unwrap_or(false);
    let linked = a.linked.unwrap_or(true);
    let mut normalized_args = args;
    if let Some(obj) = normalized_args.as_object_mut() {
        obj.insert("ripple".into(), json!(ripple));
        obj.insert("linked".into(), json!(linked));
    }

    let linked_move = resolve_linked_media(state, &a.clip, linked, "edit.move").await?;

    let res = if let Some(link) = &linked_move {
        let steps = link.move_steps(&a.clip, &a.to_track, a.at_ms, ripple);
        commit_lowered(
            state,
            "edit.move",
            normalized_args,
            actor,
            steps,
            vec![effect(
                None,
                json!({
                    "linked_move": {
                        "clip": link.clip,
                        "track": link.track,
                    }
                }),
            )],
        )
        .await?
    } else {
        commit_core(state, "edit.move", normalized_args, actor).await?
    };
    // F-1: after the move lands, resolve the moved clip's asset and warn if its
    // audio won't be mixed on the destination (video) track.
    let asset = {
        let guard = state.project.read().await;
        guard.as_ref().and_then(|store| {
            store
                .project
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .find_map(|c| match c {
                    cut_core::Clip::Media(m) if m.id == a.clip => Some(m.asset.clone()),
                    _ => None,
                })
        })
    };
    let warning = match (asset, linked_move.is_some()) {
        (_, true) => None,
        (Some(aid), false) => audio_drop_warning(state, &aid, &a.to_track).await,
        (None, false) => None,
    };
    Ok(append_warning(res, warning))
}

/// edit.crossfade{track, at_ms, duration_ms} — dissolve the cut at at_ms on
/// `track` between two adjacent media clips. duration_ms 0 clears it.
/// Stored on the right clip (travels through ripples); the EDL pulls the
/// timeline back by the overlap, the renderer emits xfade/acrossfade. A
/// crossfade OWNS the cut, so it clears the boundary's per-clip fades.
pub(in crate::dispatch) async fn edit_crossfade(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        track: String,
        at_ms: u64,
        duration_ms: u64,
    }
    let a: Args = parse_args(args.clone())?;
    let res = commit_core(state, "edit.crossfade", args, actor).await?;
    // F-2: warn if the crossfade left the base video/audio pair at unequal
    // realized lengths (single-track crossfade desyncs AV after the seam).
    let warning = crossfade_av_sync_warning(state, &a.track).await;
    Ok(append_warning(res, warning))
}

/// F-2: edit.crossfade shortens ONE track by the overlap. If the base video and
/// base audio tracks (the mirrored A/V pair the talking-head wedge keeps in
/// lockstep) end up at DIFFERENT realized lengths, every frame after the seam is
/// offset from its audio — and no receipt check catches an intra-EDL AV-length
/// divergence. We warn in-band so the desync can't ship silently; the fix is to
/// crossfade the sibling track at the same seam. Returns None when the crossfade
/// touched a track outside the base pair, or the pair stayed equal.
async fn crossfade_av_sync_warning(
    state: &AppState,
    track_id: &str,
) -> Option<cut_core::VerbWarning> {
    let guard = state.project.read().await;
    let store = guard.as_ref()?;
    let edl = cut_core::edl_from_project(&store.project);
    let base_v = edl.base_video_track()?.to_string();
    let base_a = store
        .project
        .tracks
        .iter()
        .find(|t| t.kind == cut_core::TrackKind::Audio && !t.clips.is_empty())
        .map(|t| t.id.clone())?;
    // Only the base A/V pair is expected to stay length-locked; a music bed on
    // another audio track is intentionally independent.
    if track_id != base_v && track_id != base_a {
        return None;
    }
    let realized = |tid: &str| -> u64 {
        edl.track_segments(tid)
            .map(|s| s.timeline_out_ms)
            .max()
            .unwrap_or(0)
    };
    let (v_len, a_len) = (realized(&base_v), realized(&base_a));
    if v_len == a_len {
        return None;
    }
    let delta = v_len.abs_diff(a_len);
    let mut detail = serde_json::Map::new();
    detail.insert("base_video_track".into(), json!(base_v));
    detail.insert("base_audio_track".into(), json!(base_a));
    detail.insert("video_len_ms".into(), json!(v_len));
    detail.insert("audio_len_ms".into(), json!(a_len));
    detail.insert("delta_ms".into(), json!(delta));
    detail.insert(
        "suggested_action".into(),
        json!("crossfade the sibling track at the same seam to keep audio and video in sync"),
    );
    Some(cut_core::VerbWarning {
        code: "av_length_diverged".into(),
        message: format!(
            "base video track '{base_v}' ({v_len}ms) and base audio track '{base_a}' ({a_len}ms) \
             now differ by {delta}ms — content after the crossfade seam is offset from its audio"
        ),
        detail,
    })
}

/// edit.move_marker{id, at_ms} — reposition an existing marker, id preserved
/// One op; a remove+add would mint a new id and break references.
pub(in crate::dispatch) async fn edit_move_marker(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        id: String,
        at_ms: u64,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.move_marker", args, actor).await
}

/// edit.update_marker — relabel, recolor, and/or edit a marker note
/// This is one core editing operation.
/// One op, id + position preserved; core validates the color set and refuses
/// machine-managed system markers (beat / capture:).
pub(in crate::dispatch) async fn edit_update_marker(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        id: String,
        label: Option<String>,
        color: Option<String>,
        note: Option<String>,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "edit.update_marker", args, actor).await
}

/// edit.seek_marker — JUMP-TO navigation over markers/bookmarks: given a
/// position `from_ms`, return the next/prev marker (or first/last, or one by
/// id). A pure READ (no mutation) — the caller (UI key-binding or agent) moves
/// the playhead to the returned `at_ms` via ui.playhead. Markers are the named
/// bookmarks (edit.add_marker sets a label); beat markers (beat:N from
/// audio.add_music) are included, so this also steps through the beat grid.
pub(in crate::dispatch) async fn edit_seek_marker(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        /// Reference position; next/prev are relative to this (default 0).
        from_ms: Option<u64>,
        /// next | prev | first | last (default next). Ignored when `id` is set.
        direction: Option<String>,
        /// Jump to this exact marker id (overrides direction).
        id: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let mut markers: Vec<&cut_core::Marker> = store.project.markers.iter().collect();
    markers.sort_by_key(|m| m.at_ms);
    let total = markers.len();
    if total == 0 {
        return Ok(VerbResult::ok(json!({ "marker": Value::Null, "total": 0 })));
    }
    let from = a.from_ms.unwrap_or(0);
    let picked: Option<(usize, &cut_core::Marker)> = if let Some(id) = a.id.as_deref() {
        markers
            .iter()
            .enumerate()
            .find(|(_, m)| m.id == id)
            .map(|(i, m)| (i, *m))
    } else {
        match a.direction.as_deref().unwrap_or("next") {
            "next" => markers
                .iter()
                .enumerate()
                .find(|(_, m)| m.at_ms > from)
                .map(|(i, m)| (i, *m)),
            "prev" => markers
                .iter()
                .enumerate()
                .rev()
                .find(|(_, m)| m.at_ms < from)
                .map(|(i, m)| (i, *m)),
            "first" => markers.first().map(|m| (0, *m)),
            "last" => markers.last().map(|m| (total - 1, *m)),
            other => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("unknown direction '{other}'"),
                    "direction must be next | prev | first | last (or pass an id)",
                ))
            }
        }
    };
    match picked {
        Some((idx, m)) => Ok(VerbResult::ok(json!({
            "marker": { "id": m.id, "at_ms": m.at_ms, "label": m.label, "note": m.note },
            "index": idx,
            "total": total,
        }))),
        None => Ok(VerbResult::ok(json!({
            "marker": Value::Null,
            "total": total,
            "reason": "no marker in that direction",
        }))),
    }
}

/// captions.set_range{clip, range_ms} — set a caption clip's absolute timeline
/// range: retime (shift both edges) or trim (one edge) for direct
/// manipulation of caption clips, which edit.move/edit.trim refuse.
pub(in crate::dispatch) async fn captions_set_range(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        range_ms: [u64; 2],
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "captions.set_range", args, actor).await
}

/// captions.set_text{clip, text, style_ref?} — replace an EXISTING caption clip's
/// words (and optionally its style_ref) in place, by clip id. The edit companion
/// to captions.set_range (retime): together they make a placed caption fully
/// editable from the Inspector (caption-editing regression — captions.add_text
/// only ADDS; nothing EDITED an existing caption's text). Thin: validate then
/// commit_core, which replays through edit::caption_set_text in core.
pub(in crate::dispatch) async fn captions_set_text(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        clip: String,
        text: String,
        #[serde(default)]
        style_ref: Option<String>,
    }
    let _a: Args = parse_args(args.clone())?;
    commit_core(state, "captions.set_text", args, actor).await
}

/// True when `track_id` is the BASE track of its kind — the first video (or
/// audio) track that has clips, falling back to the positionally-first track
/// of that kind when none have clips yet. The video rule is exactly the
/// renderer's "first video track with clips is the base canvas"
/// (cut_media::build_graph); audio mirrors it. Base tracks carry the program
/// content whose timing everything else is aligned to — inserting into one
/// defaults to ripple (AV sync); overlay/extra tracks float (PiP, music
/// beds), so inserting into those defaults to no ripple and preserves A/V sync.
/// A/V placement guard: the renderer (cut-media build_graph) mixes audio from
/// TrackKind::Audio tracks ONLY. A media clip carrying audio that lands on a
/// VIDEO track therefore has its audio SILENTLY dropped from the render — and
/// the receipt stays green on an audio-less render (the exact trust failure the
/// product forbids). We convert that silent drop into an in-band guardrail
/// warning (public verb contract warnings[] channel) so the agent/UI sees it and can place
/// the clip on an audio track to include its sound. Auto-place bypasses these
/// handlers (it calls commit_core directly with its v1+a1t mirror), so the
/// first-import mirror never false-warns. Returns None when the placement loses
/// no audio (target is an audio track, or the asset has no audio stream).
async fn audio_drop_warning(
    state: &AppState,
    asset_id: &str,
    track_id: &str,
) -> Option<cut_core::VerbWarning> {
    let guard = state.project.read().await;
    let store = guard.as_ref()?;
    // Only a VIDEO track drops audio (the renderer mixes Audio tracks only).
    if store.project.track(track_id).map(|t| t.kind) != Some(cut_core::TrackKind::Video) {
        return None;
    }
    // Does the asset carry an audio stream? (probe.has_audio)
    let has_audio = store
        .project
        .assets
        .get(asset_id)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("has_audio"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !has_audio {
        return None;
    }
    let mut detail = serde_json::Map::new();
    detail.insert("asset".into(), json!(asset_id));
    detail.insert("track".into(), json!(track_id));
    detail.insert(
        "suggested_action".into(),
        json!("also place this clip on an audio track to include its sound in the render"),
    );
    Some(cut_core::VerbWarning {
        code: "audio_not_mixed".into(),
        message: format!(
            "clip on video track '{track_id}' carries audio that the renderer does not mix \
             (audio is rendered from audio tracks only) — its sound will be absent from the render"
        ),
        detail,
    })
}

/// Append `warning` (if any) to an already-successful verb envelope without
/// clobbering warnings the core path already attached.
fn append_warning(res: VerbResult, warning: Option<cut_core::VerbWarning>) -> VerbResult {
    match warning {
        None => res,
        Some(w) => {
            let mut warns = res.warnings.clone().unwrap_or_default();
            warns.push(w);
            res.with_warnings(warns)
        }
    }
}

fn is_base_track(project: &cut_core::Project, track_id: &str) -> bool {
    let Some(kind) = project.track(track_id).map(|t| t.kind) else {
        return false; // unknown track — core will produce the not_found error
    };
    if kind == cut_core::TrackKind::Caption {
        return false; // insert refuses caption tracks anyway
    }
    let same_kind = || project.tracks.iter().filter(|t| t.kind == kind);
    let base = same_kind()
        .find(|t| !t.clips.is_empty())
        .or_else(|| same_kind().next())
        .map(|t| t.id.as_str());
    base == Some(track_id)
}

pub(in crate::dispatch) async fn edit_insert(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // at_ms validates only; the rest are read
    struct Args {
        asset: String,
        track: String,
        at_ms: u64,
        src_range_ms: Option<[u64; 2]>,
        duration_ms: Option<u64>,
        ripple: Option<bool>,
    }
    let a: Args = parse_args(args.clone())?;
    // Omitted src_range_ms means full source length, resolved HERE from the
    // live probe and recorded explicitly on the op. Logged ops must be
    // self-contained: probe write-back is cache (not an op), so replay
    // cannot consult it — an op logged with an implicit range would fail
    // every later diff/replay.
    let src_range = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let probe = store
            .project
            .assets
            .get(&a.asset)
            .and_then(|x| x.probe.as_ref());
        let kind = probe
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str())
            .unwrap_or("");
        if kind == "image" {
            // Stills have no intrinsic duration — the EDIT supplies it
            // duration_ms maps to src_range [0, d): the clip
            // model stays uniform and the renderer loops the still over it.
            if store.project.track(&a.track).map(|t| t.kind) == Some(cut_core::TrackKind::Audio) {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "asset '{}' is a still image — it cannot go on an audio track",
                        a.asset
                    ),
                    "images have no audio stream",
                )
                .with_suggested_action("insert it on a video track"));
            }
            match (a.src_range_ms, a.duration_ms) {
                (Some(r), None) => r, // replay path / explicit range — same meaning
                (None, Some(d)) if d > 0 => [0, d],
                (None, Some(_)) => {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        "duration_ms must be > 0",
                        "a zero-length clip would be empty",
                    ))
                }
                (Some(_), Some(_)) => return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "pass either duration_ms or src_range_ms, not both",
                    "for a still image they mean the same thing — duration_ms is the idiomatic arg",
                )),
                (None, None) => return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "asset '{}' is a still image — an explicit duration is required",
                        a.asset
                    ),
                    "stills have no intrinsic duration to default to",
                )
                .with_suggested_action(
                    "pass duration_ms (e.g. 3000 for a 3s intro card); the render loops the still",
                )),
            }
        } else {
            if a.duration_ms.is_some() {
                let (cause, action) = if kind.is_empty() {
                    (
                        format!(
                            "asset '{}' has no probe yet, so its kind is unknown",
                            a.asset
                        ),
                        "wait for the import chain job (or run media.probe), then retry",
                    )
                } else {
                    (
                        format!("asset '{}' probed as '{kind}'", a.asset),
                        "use src_range_ms to select a sub-range of timed media",
                    )
                };
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "duration_ms is only valid for still-image assets",
                    cause,
                )
                .with_suggested_action(action));
            }
            match a.src_range_ms {
                Some(r) => r,
                None => {
                    let d = probe
                        .and_then(|p| p.get("duration_ms"))
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| {
                            CutError::new(
                                error_codes::INVALID_ARGS,
                                format!(
                                    "asset '{}' has no probe yet — full-length insert needs a duration",
                                    a.asset
                                ),
                                "src_range_ms was omitted and probe.duration_ms is unavailable",
                            )
                            .with_suggested_action("run media.probe first or pass src_range_ms")
                        })?;
                    [0, d]
                }
            }
        }
    };
    // Ripple default (the ripple-sync contract): base tracks ripple siblings (AV sync is the
    // safe default — the intro-card insert regression desynced audio by 2.5 s),
    // overlay/extra tracks float. An explicit arg always wins. The RESOLVED
    // value is recorded on the op: logged ops must be self-contained, and
    // core treats a missing key as the legacy no-ripple behavior so old logs
    // replay unchanged.
    let ripple = match a.ripple {
        Some(explicit) => explicit,
        None => {
            let guard = state.project.read().await;
            let store = guard.as_ref().ok_or_else(no_project)?;
            is_base_track(&store.project, &a.track)
        }
    };
    let mut args = args;
    args["src_range_ms"] = json!(src_range); // self-contained op record
    args["ripple"] = json!(ripple);
    // duration_ms (image convenience) is consumed here; the recorded op
    // carries the resolved src_range_ms, which is what core/replay read.
    if let Some(obj) = args.as_object_mut() {
        obj.remove("duration_ms");
    }
    let res = commit_core(state, "edit.insert", args, actor).await?;
    // F-1: warn (don't silently drop) when an audio-bearing clip lands on a
    // video track whose audio the renderer won't mix.
    let warning = audio_drop_warning(state, &a.asset, &a.track).await;
    Ok(append_warning(res, warning))
}

/// edit.duplicate{clip, rationale?} — duplicate a clip (the universal NLE Ctrl+D
/// / "Duplicate"): copy a clip and place the copy IMMEDIATELY AFTER it on the
/// SAME track, carrying its source range + ALL per-clip attributes
/// (effects/grade/transform/crop/fade/speed/speed_ramp/reverse/freeze/matte/mask
/// (redact)/eq/stabilize/keyframes/animation/gain — the whole clip minus its id).
/// The rest of the track ripples like a normal insert.
///
/// THIN orchestration over a REAL core op: each half lowers to one replay-safe
/// `edit.duplicate` core op (`cut_core::edit::duplicate` — a clone-by-reference
/// whose new id is pinned via `added_clip`). The op references the SOURCE clip by
/// id and re-clones it at apply time, so the log replays byte-identically AND the
/// clone carries every attribute (and any future one) automatically — no clip
/// snapshot in the op. v1 copies the whole clip (no sub-range, no overwrite).
///
/// LINKED AUDIO: when the source is a muxed VIDEO clip whose audio rides a sibling
/// AUDIO clip (same asset + same timeline start — the auto-place / linked-insert
/// signature edit.paste and the linked-delete also key off), that sibling is
/// duplicated too so the A/V pair stays aligned (one logical action). BOTH halves
/// are placed with `ripple:false` — the aligned-pair pattern the importer's
/// auto-place uses: each track shifts its OWN later content by the clone's
/// duration, so the two new clips land at the same timeline position and stay
/// frame-locked, with NO leftover gap. (A single rippling half would open a
/// clip-length gap on the SIBLING track, and the second insert would then sit
/// BESIDE that gap — doubling the audio span and desyncing the pair: the exact
/// trap the linked-paste regression fixed.) The two ops share one
/// `group_id` so a single Ctrl+Z undoes the pair. A LONE
/// clip (no linked audio) resolves its ripple from the host track exactly like
/// edit.insert/paste (base track → true for AV sync, overlay/extra → false).
pub(in crate::dispatch) async fn edit_duplicate(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale is read off `args` by commit_core
    struct Args {
        clip: String,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;

    // The linked audio sibling's clip id (None ⇒ a lone clip / non-video source).
    struct AudioLink {
        clip: String,
    }

    // ── Resolve under a read lock: validate the source clip, detect a linked
    //    audio sibling, and resolve the video half's ripple. No ids allocated. ──
    let (video_ripple, audio_link): (bool, Option<AudioLink>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;

        let (track_id, idx) = project.find_clip(&a.clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no clip '{}' on the timeline", a.clip),
                "clip must be an existing clip id (project.state lists clips)",
            )
            .with_clip(&a.clip)
        })?;
        let track_id = track_id.to_string();
        let from_kind = project.track(&track_id).expect("track exists").kind;
        if from_kind == cut_core::TrackKind::Caption {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("clip '{}' is a caption clip", a.clip),
                "caption clips are managed via captions.* verbs, not edit.duplicate",
            )
            .with_clip(&a.clip));
        }
        // Must be a media clip (find_clip never returns a gap — defensive).
        let cut_core::Clip::Media(mc) = &project.track(&track_id).expect("track").clips[idx] else {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("clip '{}' is not a media clip", a.clip),
                "edit.duplicate copies a media (video/audio) clip",
            )
            .with_clip(&a.clip));
        };
        let asset = mc.asset.clone();

        // Linked-audio lookup — the SAME signature as edit.paste: a muxed video
        // clip's audio is a SEPARATE clip on an audio track at the SAME timeline
        // start with the SAME asset. Use the EDL for absolute timeline positions
        // (clips are positional in the model). Only meaningful for a video source.
        let audio_link = if from_kind == cut_core::TrackKind::Video {
            let edl = cut_core::edl_from_project(project);
            let start = edl
                .segments
                .iter()
                .find(|s| s.clip_id.as_deref() == Some(a.clip.as_str()))
                .map(|s| s.timeline_in_ms);
            start.and_then(|start_ms| {
                edl.segments
                    .iter()
                    .find(|s| {
                        s.track_kind == cut_core::TrackKind::Audio
                            && s.asset.as_deref() == Some(asset.as_str())
                            && s.timeline_in_ms == start_ms
                            && s.clip_id.is_some()
                    })
                    .and_then(|s| s.clip_id.clone().map(|clip| AudioLink { clip }))
            })
        } else {
            None
        };

        // Video/lone half ripple: the aligned-pair path forces ripple:false on
        // BOTH halves; a lone clip resolves from its track like edit.insert/paste.
        let video_ripple = if audio_link.is_some() {
            false
        } else {
            is_base_track(project, &track_id)
        };
        (video_ripple, audio_link)
    };

    // One group id when a linked-audio half will ALSO be duplicated, so a single
    // Ctrl+Z undoes the whole pair using the next globally unique op id.
    let group_id = if audio_link.is_some() {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        Some(format!("grp-duplicate-{}", store.log.next_id()?))
    } else {
        None
    };
    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("duplicate clip '{}'", a.clip));

    // ── Video (or lone) half: one replay-safe edit.duplicate core op. ──
    let res = commit_core(
        state,
        "edit.duplicate",
        json!({
            "clip": a.clip,
            "ripple": video_ripple,
            "rationale": rationale,
            "group_id": group_id,
        }),
        actor.clone(),
    )
    .await?;
    let new_clip = res
        .result
        .as_ref()
        .and_then(|r| r.get("clip_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let track = res
        .result
        .as_ref()
        .and_then(|r| r.get("track"))
        .cloned()
        .unwrap_or(Value::Null);
    let at_ms = res
        .result
        .as_ref()
        .and_then(|r| r.get("at_ms"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut op_ids: Vec<String> = res.op_ids.clone().unwrap_or_default();

    // ── Linked audio half: a SECOND edit.duplicate on the sibling audio clip,
    //    ripple:false (the aligned-pair rule above). Two ops / two op_ids, one
    //    group → one undo. A failure here is surfaced as a warning (the video
    //    clone already landed; we don't silently drop the audio half) but does
    //    not unwind the video duplicate — mirrors the linked-paste contract. ──
    let mut linked_audio = Value::Null;
    if let Some(link) = &audio_link {
        match commit_core(
            state,
            "edit.duplicate",
            json!({
                "clip": link.clip,
                "ripple": false,
                "rationale": format!("duplicate linked audio of '{}'", a.clip),
                "group_id": group_id,
            }),
            actor,
        )
        .await
        {
            Ok(audio_res) => {
                if let Some(ids) = audio_res.op_ids.clone() {
                    op_ids.extend(ids);
                }
                linked_audio = json!({
                    "source_clip": link.clip,
                    "new_clip": audio_res
                        .result
                        .as_ref()
                        .and_then(|r| r.get("clip_id"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "track": audio_res
                        .result
                        .as_ref()
                        .and_then(|r| r.get("track"))
                        .cloned()
                        .unwrap_or(Value::Null),
                });
            }
            Err(e) => {
                let res = VerbResult::ok_with_ops(
                    json!({
                        "source_clip": a.clip,
                        "new_clip": new_clip,
                        "track": track,
                        "at_ms": at_ms,
                        "linked_audio": null,
                    }),
                    op_ids,
                );
                return Ok(append_warning(
                    res,
                    Some(cut_core::VerbWarning {
                        code: "linked_audio_duplicate_failed".into(),
                        message: format!(
                            "the clip duplicated, but its linked audio did not: {}",
                            e.message
                        ),
                        detail: json!({"audio_clip": link.clip, "error": e.message})
                            .as_object()
                            .cloned()
                            .unwrap_or_default(),
                    }),
                ));
            }
        }
    }

    Ok(VerbResult::ok_with_ops(
        json!({
            "source_clip": a.clip,
            "new_clip": new_clip,
            "track": track,
            "at_ms": at_ms,
            "linked_audio": linked_audio,
        }),
        op_ids,
    ))
}

/// Resolve a `{asset?, source_clip?, source_in_ms?, source_out_ms?}` source
/// descriptor (shared by edit.replace + edit.fit_to_fill) into a concrete
/// `(asset, source_in?, source_out?)` under the caller's read lock. EXACTLY one of
/// `asset` / `source_clip` is required. When `source_clip` is given the asset and
/// the default source window come from that clip (a media clip's own
/// `[src_in_ms, src_out_ms]`); explicit `source_in_ms`/`source_out_ms` override
/// either edge. A media-only check mirrors edit.paste (captions/gaps have no source
/// window). The asset must be registered in the project.
fn resolve_fill_source(
    project: &cut_core::Project,
    asset: Option<&str>,
    source_clip: Option<&str>,
    source_in_ms: Option<u64>,
    source_out_ms: Option<u64>,
) -> Result<(String, Option<u64>, Option<u64>), CutError> {
    match (asset, source_clip) {
        (Some(_), Some(_)) | (None, None) => Err(CutError::new(
            error_codes::INVALID_ARGS,
            "pass exactly one source: `asset` or `source_clip`",
            "give the replacement footage as either an imported asset id or an existing clip id",
        )),
        (Some(aid), None) => {
            if !project.assets.contains_key(aid) {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no asset '{aid}' in the project"),
                    "the source asset is not (or no longer) imported",
                ));
            }
            Ok((aid.to_string(), source_in_ms, source_out_ms))
        }
        (None, Some(cid)) => {
            let (track, idx) = project.find_clip(cid).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no source clip '{cid}' on the timeline"),
                    "source_clip must be an existing clip id (project.state lists clips)",
                )
                .with_clip(cid)
            })?;
            let cut_core::Clip::Media(mc) = &project.track(track).expect("track exists").clips[idx]
            else {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("source clip '{cid}' is not a media clip"),
                    "the source must be a media (video/audio) clip with a source window",
                )
                .with_clip(cid));
            };
            // Default the source window to the clip's own range; explicit args win.
            Ok((
                mc.asset.clone(),
                Some(source_in_ms.unwrap_or(mc.src_in_ms)),
                Some(source_out_ms.unwrap_or(mc.src_out_ms)),
            ))
        }
    }
}

/// edit.nest{clips:[clip_ids], name?, rationale?} — collapse a CONTIGUOUS run of
/// clips on ONE track into a single NEST / COMPOUND CLIP (the standard nested-timeline
/// staple). The selected clips MOVE off the parent track into a new sub-timeline
/// stored on the project (`Project::nests`, every per-clip attribute preserved) and
/// are REPLACED in place by one nest clip spanning their combined range — so the
/// parent timeline length is unchanged. At render time the server bakes the
/// sub-timeline to a content-addressed file (crate::nest::bake_and_flatten, the matte
/// pattern) and feeds it in as the nest clip's source, so a project with no nest
/// renders byte-identical.
///
/// THIN wrapper over the replay-safe `edit.nest` CORE op (which does all validation —
/// contiguity, same-track, media-only, no nest-of-nest — and allocates the
/// sub-timeline + a single pinned nest clip id). v1 scope: CREATE + RENDER; editing
/// INSIDE a nest and multi-track / linked-audio nests are documented follow-ups.
pub(in crate::dispatch) async fn edit_nest(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    // Parse for a clean up-front error (the core op re-validates the selection). An
    // empty `clips` is refused here so malformed requests return an actionable
    // invalid_args rather than a deep core error.
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // name/rationale are read off `args` by the core op / commit_core
    struct Args {
        clips: Vec<String>,
        name: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.clips.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.nest needs at least one clip",
            "pass clips:[...] — a contiguous run of media clips on one track to collapse into a nest",
        ));
    }
    commit_core(state, "edit.nest", args, actor).await
}

/// edit.replace{target_clip, asset?|source_clip?, source_in_ms?, source_out_ms?,
/// link_audio?, rationale?} — the 3-point "REPLACE EDIT" (a three-point replace edit):
/// swap the clip at `target_clip`'s slot with new source footage while PRESERVING the
/// target's id, timeline position, and slot duration. The replacement keeps the
/// target's LOOK (effects/grade/transform/crop/fade/gain/eq/mask/matte/stabilize/
/// reverse/transitions/keyframes/animation) and plays at NORMAL speed; the three
/// source-timing fields tied to the old footage (constant speed, speed ramp, freeze)
/// are reset. If the chosen source window is SHORTER than the slot, the replacement is
/// clamped and the remainder of the slot is padded with a gap (so the slot — and all
/// downstream timing — is preserved exactly; `gap_ms` in the receipt reports it).
///
/// THIN orchestration over the replay-safe `edit.replace` core op (an in-place source
/// swap that allocates NO clip id). LINKED AUDIO (`link_audio`, default true): when the
/// target is a muxed VIDEO clip whose audio rides a sibling AUDIO clip (same asset +
/// same timeline start — the auto-place / linked-insert signature), that sibling is
/// replaced too (from the same new asset + source offset) so the A/V pair stays in
/// sync; the two ops share one undo group. Because each half is an in-place
/// EQUAL-DURATION swap, NEITHER track ripples — the linked-paste C-1 double-ripple
/// trap cannot arise (no gap is opened on the sibling track for a second op to double).
pub(in crate::dispatch) async fn edit_replace(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale is read off `args` by commit_core
    struct Args {
        target_clip: String,
        asset: Option<String>,
        source_clip: Option<String>,
        source_in_ms: Option<u64>,
        source_out_ms: Option<u64>,
        link_audio: Option<bool>,
        group_id: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let link_audio = a.link_audio.unwrap_or(true);

    // The linked audio sibling's clip id (None ⇒ a lone clip / non-video target).
    struct AudioLink {
        clip: String,
    }

    // ── Resolve under a read lock: the source descriptor, the target's kind, and
    //    the optional linked-audio sibling (keyed off the target's OLD asset). ──
    let (src_asset, src_in, src_out, audio_link): (
        String,
        Option<u64>,
        Option<u64>,
        Option<AudioLink>,
    ) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;

        let (src_asset, src_in, src_out) = resolve_fill_source(
            project,
            a.asset.as_deref(),
            a.source_clip.as_deref(),
            a.source_in_ms,
            a.source_out_ms,
        )?;

        let (track_id, idx) = project.find_clip(&a.target_clip).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no clip '{}' on the timeline", a.target_clip),
                "target_clip must be an existing clip id (project.state lists clips)",
            )
            .with_clip(&a.target_clip)
        })?;
        let track_id = track_id.to_string();
        let from_kind = project.track(&track_id).expect("track exists").kind;
        if from_kind == cut_core::TrackKind::Caption {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("clip '{}' is a caption clip", a.target_clip),
                "caption clips are managed via captions.* verbs, not edit.replace",
            )
            .with_clip(&a.target_clip));
        }
        // The target's OLD asset — the linked-audio sibling shares it (before swap).
        let old_asset = match &project.track(&track_id).expect("track").clips[idx] {
            cut_core::Clip::Media(mc) => mc.asset.clone(),
            _ => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{}' is not a media clip", a.target_clip),
                    "edit.replace swaps a media (video/audio) clip's source",
                )
                .with_clip(&a.target_clip))
            }
        };

        // Linked-audio lookup — the SAME EDL signature edit.paste/duplicate use: a
        // muxed video clip's audio is a SEPARATE clip on an audio track at the SAME
        // timeline start with the SAME (OLD) asset. Resolved BEFORE any swap, while
        // the pair still shares the old asset. Only meaningful for a video target.
        let audio_link = if link_audio && from_kind == cut_core::TrackKind::Video {
            let edl = cut_core::edl_from_project(project);
            let start = edl
                .segments
                .iter()
                .find(|s| s.clip_id.as_deref() == Some(a.target_clip.as_str()))
                .map(|s| s.timeline_in_ms);
            start.and_then(|start_ms| {
                edl.segments
                    .iter()
                    .find(|s| {
                        s.track_kind == cut_core::TrackKind::Audio
                            && s.asset.as_deref() == Some(old_asset.as_str())
                            && s.timeline_in_ms == start_ms
                            && s.clip_id.is_some()
                    })
                    .and_then(|s| s.clip_id.clone().map(|clip| AudioLink { clip }))
            })
        } else {
            None
        };
        (src_asset, src_in, src_out, audio_link)
    };

    // One group id when a linked-audio half will ALSO be replaced, so a single
    // Ctrl+Z undoes the whole pair using the next globally unique op id.
    let group_id = if let Some(group_id) = a.group_id.clone() {
        Some(group_id)
    } else if audio_link.is_some() {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        Some(format!("grp-replace-{}", store.log.next_id()?))
    } else {
        None
    };
    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("replace clip '{}' with '{}'", a.target_clip, src_asset));

    // ── Video (or lone) half: one replay-safe edit.replace core op. ──
    let res = commit_core(
        state,
        "edit.replace",
        json!({
            "clip": a.target_clip,
            "asset": src_asset,
            "source_in_ms": src_in,
            "source_out_ms": src_out,
            "rationale": rationale,
            "group_id": group_id,
        }),
        actor.clone(),
    )
    .await?;
    let result = res.result.clone().unwrap_or(Value::Null);
    let track = result.get("track").cloned().unwrap_or(Value::Null);
    let slot_ms = result.get("slot_ms").cloned().unwrap_or(Value::Null);
    let gap_ms = result.get("gap_ms").cloned().unwrap_or(Value::Null);
    let new_src_ms = result.get("new_src_ms").cloned().unwrap_or(Value::Null);
    let mut op_ids: Vec<String> = res.op_ids.clone().unwrap_or_default();

    // ── Linked audio half: a SECOND edit.replace on the sibling audio clip (same
    //    new asset + source offset). An in-place equal-duration swap, so it never
    //    ripples. A failure here is surfaced as a warning (the video already swapped;
    //    we don't silently drop the audio half) but does not unwind the video. ──
    let mut linked_audio = Value::Null;
    if let Some(link) = &audio_link {
        match commit_core(
            state,
            "edit.replace",
            json!({
                "clip": link.clip,
                "asset": src_asset,
                "source_in_ms": src_in,
                "source_out_ms": src_out,
                "rationale": format!("replace linked audio of '{}'", a.target_clip),
                "group_id": group_id,
            }),
            actor,
        )
        .await
        {
            Ok(audio_res) => {
                if let Some(ids) = audio_res.op_ids.clone() {
                    op_ids.extend(ids);
                }
                let ares = audio_res.result.clone().unwrap_or(Value::Null);
                linked_audio = json!({
                    "clip": link.clip,
                    "track": ares.get("track").cloned().unwrap_or(Value::Null),
                    "new_src_ms": ares.get("new_src_ms").cloned().unwrap_or(Value::Null),
                    "gap_ms": ares.get("gap_ms").cloned().unwrap_or(Value::Null),
                });
            }
            Err(e) => {
                let res = VerbResult::ok_with_ops(
                    json!({
                        "target_clip": a.target_clip,
                        "asset": src_asset,
                        "track": track,
                        "slot_ms": slot_ms,
                        "new_src_ms": new_src_ms,
                        "gap_ms": gap_ms,
                        "linked_audio": null,
                    }),
                    op_ids,
                );
                return Ok(append_warning(
                    res,
                    Some(cut_core::VerbWarning {
                        code: "linked_audio_replace_failed".into(),
                        message: format!(
                            "the clip was replaced, but its linked audio was not: {}",
                            e.message
                        ),
                        detail: json!({"audio_clip": link.clip, "error": e.message})
                            .as_object()
                            .cloned()
                            .unwrap_or_default(),
                    }),
                ));
            }
        }
    }

    Ok(VerbResult::ok_with_ops(
        json!({
            "target_clip": a.target_clip,
            "asset": src_asset,
            "track": track,
            "slot_ms": slot_ms,
            "new_src_ms": new_src_ms,
            "gap_ms": gap_ms,
            "linked_audio": linked_audio,
        }),
        op_ids,
    ))
}

/// edit.fit_to_fill{track, at_ms, duration_ms?, asset?|source_clip?, source_in_ms?,
/// source_out_ms?, rationale?} — FIT TO FILL: drop source footage into an
/// EMPTY slot and SPEED-ADJUST it so it exactly fills the slot. `speed = source_span /
/// slot`, so the retimed clip's timeline span lands back on the slot duration and NO
/// downstream content moves (the gap is consumed in place, not rippled).
///
/// The slot is `[at_ms, at_ms + duration_ms)` on `track`; when `duration_ms` is OMITTED
/// it is inferred from the GAP currently at `at_ms` (fill THIS gap). The target span
/// must be EMPTY (a single gap, or the track tail) — fit_to_fill fills space, it does
/// not overwrite media (delete the clip first, or use edit.replace). The required speed
/// must lie in the engine's [0.25×, 4.0×] retime range, else the source is too long /
/// too short to fit and the verb errors with an actionable hint. THIN over the
/// replay-safe `edit.fit_to_fill` core op; the resolved slot + source window + speed are
/// recorded on the op so replay reproduces the fill verbatim. Single track, no linked
/// audio (run it again on the audio track to speed-fit the matching audio slot).
pub(in crate::dispatch) async fn edit_fit_to_fill(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale is read off `args` by commit_core
    struct Args {
        track: String,
        at_ms: u64,
        duration_ms: Option<u64>,
        asset: Option<String>,
        source_clip: Option<String>,
        source_in_ms: Option<u64>,
        source_out_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;

    // ── Resolve under a read lock: source descriptor → concrete [src_in, src_out),
    //    the slot duration (explicit, or inferred from the gap at at_ms), then the
    //    fit speed (validated against the retime range BEFORE commit). ──
    let (src_asset, src_in, src_out, slot_ms): (String, u64, u64, u64) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;

        let track = project.track(&a.track).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("no track '{}'", a.track),
                "track must be an existing track id (project.state lists tracks)",
            )
        })?;
        if track.kind == cut_core::TrackKind::Caption {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "cannot fit_to_fill onto a caption track",
                "caption clips are created via captions.* verbs",
            ));
        }

        let (src_asset, src_in_opt, src_out_opt) = resolve_fill_source(
            project,
            a.asset.as_deref(),
            a.source_clip.as_deref(),
            a.source_in_ms,
            a.source_out_ms,
        )?;
        // The fit speed needs a CONCRETE source span, so resolve the out edge now:
        // explicit/source-clip out, else the asset's probed duration.
        let src_in = src_in_opt.unwrap_or(0);
        let src_out = match src_out_opt {
            Some(o) => o,
            None => project
                .assets
                .get(&src_asset)
                .and_then(|asset| asset.probe.as_ref())
                .and_then(|p| p.get("duration_ms"))
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("asset '{src_asset}' has no probe yet — fit needs a source length"),
                        "source_out_ms was omitted and probe.duration_ms is unavailable",
                    )
                    .with_suggested_action("run media.probe on the asset, or pass source_out_ms")
                })?,
        };
        if src_in >= src_out {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("source window [{src_in}, {src_out}) is empty or inverted"),
                "source_in_ms must be strictly less than source_out_ms",
            ));
        }

        // Slot duration: explicit duration_ms, or the gap at at_ms (fill this gap).
        let slot_ms = match a.duration_ms {
            Some(d) if d > 0 => d,
            Some(_) => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "duration_ms must be a positive number of ms",
                    "the slot to fill cannot be zero-length",
                )
                .with_at_ms(a.at_ms))
            }
            None => {
                // Infer from the gap covering at_ms (the "fill THIS gap" mode).
                let end = track.duration_ms();
                if a.at_ms >= end {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        format!(
                            "no gap at {}ms on '{}' to infer a duration from (track ends at {}ms)",
                            a.at_ms, a.track, end
                        ),
                        "at_ms is at or past the track end — pass an explicit duration_ms to fill the tail",
                    )
                    .with_at_ms(a.at_ms));
                }
                let mut cursor = 0u64;
                let mut found: Option<u64> = None;
                for c in &track.clips {
                    let dur = c.timeline_duration_ms();
                    if a.at_ms < cursor + dur {
                        if matches!(c, cut_core::Clip::Gap(_)) {
                            found = Some((cursor + dur) - a.at_ms);
                        }
                        break;
                    }
                    cursor += dur;
                }
                found.ok_or_else(|| {
                    CutError::new(
                        error_codes::CONFLICT,
                        format!("the slot at {}ms on '{}' is occupied by media", a.at_ms, a.track),
                        "fit_to_fill fills empty space; pass duration_ms only where a gap (or the tail) is free",
                    )
                    .with_at_ms(a.at_ms)
                    .with_suggested_action(
                        "delete the clip there first (that leaves a gap to fill), or use edit.replace",
                    )
                })?
            }
        };
        (src_asset, src_in, src_out, slot_ms)
    };

    // Fit speed = source_span / slot — validated against the engine's retime range.
    let source_span = src_out - src_in;
    let speed = source_span as f64 / slot_ms as f64;
    if !speed.is_finite() || !(0.25..=4.0).contains(&speed) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "fitting {source_span}ms of source into a {slot_ms}ms slot needs {speed:.3}× speed, outside the 0.25–4.0× retime range"
            ),
            "fit_to_fill speed-stretches the source; the ratio source_span/slot must be 0.25–4.0",
        )
        .with_at_ms(a.at_ms)
        .with_suggested_action(if speed > 4.0 {
            "the source is too long for this slot — trim the source window (source_in_ms/source_out_ms) or widen the slot"
        } else {
            "the source is too short for this slot — use more source, or narrow the slot"
        }));
    }
    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("fit '{src_asset}' to fill {slot_ms}ms @ {}ms", a.at_ms));

    let res = commit_core(
        state,
        "edit.fit_to_fill",
        json!({
            "track": a.track,
            "at_ms": a.at_ms,
            "slot_ms": slot_ms,
            "asset": src_asset,
            "src_range_ms": [src_in, src_out],
            "rationale": rationale,
        }),
        actor,
    )
    .await?;
    let result = res.result.clone().unwrap_or(Value::Null);
    Ok(VerbResult::ok_with_ops(
        json!({
            "clip_id": result.get("clip_id").cloned().unwrap_or(Value::Null),
            "track": a.track,
            "at_ms": a.at_ms,
            "slot_ms": slot_ms,
            "src_range_ms": [src_in, src_out],
            "speed": speed,
            "source_span_ms": source_span,
        }),
        res.op_ids.clone().unwrap_or_default(),
    ))
}

/// edit.detach_audio{clip, rationale?} — EXTRACT / PROMOTE a video clip's audio
/// onto its own editable audio track (the "Detach Audio" affordance, reframed for
/// this engine's actual model).
///
/// MODEL (why this is EXTRACT, not "unlink"): the renderer mixes audio from
/// `TrackKind::Audio` tracks ONLY (cut_media build_graph) — a video clip's muxed
/// audio is NEVER in the render — and a video's audio normally lives as a SEPARATE
/// audio-track clip (the auto-place v1/a1t mirror). There is no clip-level link to
/// clear; a video clip and its sibling audio clip are already two independent,
/// separately editable clips. edit.move keeps an exact sibling pair together by
/// default; linked:false deliberately moves one by id. So "detach" here means:
/// for a video-track clip whose asset has audio but which has NO sibling audio
/// clip yet (a plain edit.insert — its audio is silently dropped from the render,
/// the `audio_not_mixed` guardrail), create an audio clip from that audio so it
/// becomes an editable, independently-movable, J/L-splittable timeline element.
/// This RECOVERS audio that was absent from the output (the correct fix —
/// deliberately NOT audio-neutral). When a sibling audio clip already exists, the
/// audio is already detached → a clean informational no-op.
///
/// The pure decision lives in cut_core::detach (unit-tested + replay-checked); the
/// handler lowers an `Extract` onto ONE ordinary edit.insert (optionally preceded
/// by an edit.add_track when the project has no audio track), so the extracted
/// clip's id is pinned by the insert and the log replays byte-identically.
pub(in crate::dispatch) async fn edit_detach_audio(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale is read off `args` by commit_core
    struct Args {
        clip: String,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;

    // Decide under a read lock; the pure planner allocates no ids.
    let plan = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        cut_core::plan_detach_audio(&store.project, &a.clip)
    };
    let plan = match plan {
        Ok(p) => p,
        Err(reject) => {
            return Err(match reject {
                cut_core::DetachReject::ClipNotFound => CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no clip '{}' on the timeline", a.clip),
                    "clip must be an existing clip id (project.state lists clips)",
                )
                .with_clip(&a.clip),
                cut_core::DetachReject::AlreadyAudio => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{}' is already an audio clip", a.clip),
                    "edit.detach_audio extracts the audio of a VIDEO clip; this clip is on an audio track",
                )
                .with_clip(&a.clip)
                .with_suggested_action("the audio is already its own movable clip — edit it directly"),
                cut_core::DetachReject::NotVideoClip { kind } => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{}' is a {kind} clip, not a video clip", a.clip),
                    "edit.detach_audio extracts the audio of a video media clip",
                )
                .with_clip(&a.clip),
                cut_core::DetachReject::NoAudio => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{}' has no audio to detach", a.clip),
                    "the clip's asset carries no audio stream (probe.has_audio is false)",
                )
                .with_clip(&a.clip),
                cut_core::DetachReject::Retimed { speed } => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{}' is retimed ({speed:.3}×) — detaching its audio is not supported in v1", a.clip),
                    "a normal-speed extracted audio clip would not match the clip's stretched timeline span, desyncing A/V",
                )
                .with_clip(&a.clip)
                .with_suggested_action("reset the clip's speed to 1× first (edit.speed), then detach"),
                cut_core::DetachReject::Ramped => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("clip '{}' has a variable-speed ramp — detaching its audio is not supported in v1", a.clip),
                    "the ramp warps the clip's timeline non-linearly, so a normal-speed extracted audio clip would desync A/V",
                )
                .with_clip(&a.clip)
                .with_suggested_action("clear the ramp first (edit.speed_ramp{clip, points:[]}), then detach"),
            });
        }
    };

    match plan {
        // The audio is already on its own track — clean informational no-op (NOT
        // an error: in this engine an auto-placed clip's audio is already separate
        // + independently movable, which is the very thing "detach" produces).
        cut_core::DetachPlan::AlreadyDetached { audio_clip_id } => Ok(VerbResult::ok(json!({
            "detached": false,
            "reason": "audio already on its own track",
            "audio_clip": audio_clip_id,
        }))),

        cut_core::DetachPlan::Extract {
            asset,
            at_ms,
            src_range_ms,
        } => {
            // Use an existing audio track only when ordinary edit.insert cannot
            // shift any of its pre-existing clips/automation. ripple:false keeps
            // sibling tracks fixed, but the target track itself always splices;
            // an occupied target therefore needs a fresh track.
            let existing_audio = {
                let guard = state.project.read().await;
                let store = guard.as_ref().ok_or_else(no_project)?;
                cut_core::find_safe_detach_audio_track(&store.project, at_ms)
            };
            let group_id = if existing_audio.is_none() {
                let guard = state.project.read().await;
                let store = guard.as_ref().ok_or_else(no_project)?;
                Some(format!("grp-detach_audio-{}", store.log.next_id()?))
            } else {
                None
            };
            let mut op_ids: Vec<String> = Vec::new();
            let audio_track = match existing_audio {
                Some(t) => t,
                None => {
                    let res = commit_core(
                        state,
                        "edit.add_track",
                        json!({
                            "kind": "audio",
                            "rationale": "detach audio: add an audio track",
                            "group_id": group_id,
                        }),
                        actor.clone(),
                    )
                    .await?;
                    if let Some(ids) = res.op_ids.clone() {
                        op_ids.extend(ids);
                    }
                    res.result
                        .as_ref()
                        .and_then(|r| r.get("track_id"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .ok_or_else(|| {
                            // Defensive: edit.add_track always shapes {track_id,..}.
                            CutError::new(
                                error_codes::CONFLICT,
                                "edit.add_track did not return a track id",
                                "could not place the detached audio",
                            )
                        })?
                }
            };

            let rationale = a.rationale.clone().unwrap_or_else(|| {
                format!(
                    "detach audio: extract clip '{}' audio onto {audio_track}",
                    a.clip
                )
            });
            // ripple:false — drop the audio into its aligned slot WITHOUT shifting
            // any other track (linked A/V share start + length; a ripple would
            // open a spurious gap elsewhere — same reasoning as the linked-paste).
            let insert_args = json!({
                "asset": asset,
                "track": audio_track,
                "at_ms": at_ms,
                "src_range_ms": src_range_ms,
                "ripple": false,
                "rationale": rationale,
                "group_id": group_id,
            });
            let res = commit_core(state, "edit.insert", insert_args, actor).await?;
            let audio_clip = res
                .result
                .as_ref()
                .and_then(|r| r.get("clip_id"))
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(ids) = res.op_ids.clone() {
                op_ids.extend(ids);
            }
            Ok(VerbResult::ok_with_ops(
                json!({
                    "detached": true,
                    "audio_clip": audio_clip,
                    "audio_track": audio_track,
                    "at_ms": at_ms,
                    "src_range_ms": src_range_ms,
                    // Honest: the video clip's audio was NOT in the render before
                    // (video tracks contribute no audio); this added it.
                    "added_to_render": true,
                    "note": "recovered the clip's audio onto its own track — it is now audible AND independently editable/movable",
                }),
                op_ids,
            ))
        }
    }
}

/// edit.split_edit{at_ms, kind, offset_ms, video_track?, audio_track?, rationale?}
/// — J-cut / L-cut (split edit): offset the AUDIO transition relative to the
/// VIDEO cut at a clip boundary, so one clip's audio leads or lags its video
/// (the standard smooth-dialogue edit in every NLE).
///
/// MODEL (see edit.detach_audio / cut_core::split_edit): a video clip's audio
/// lives as a SEPARATE audio-track clip (the v1/a1t mirror). A J/L cut is a ROLL
/// of the AUDIO edit point relative to the (untouched) VIDEO cut — expressed as
/// two ordinary `edit.trim`s on the two linked AUDIO clips around the boundary.
/// Because clip positions are CUMULATIVE, extending the outgoing clip's out-edge
/// pushes the incoming clip's start later automatically and trimming the incoming
/// clip's in-edge pulls its end back — the net effect downstream is ZERO (a true
/// roll), only the A|B audio boundary moves, the video is never touched.
///
///  - L-cut (video leads, audio lags): extend A_audio out by offset + trim
///    B_audio in by offset → audio transition lands offset AFTER the video cut.
///  - J-cut (audio leads, video lags): extend B_audio in EARLIER by offset + trim
///    A_audio out by offset → audio transition lands offset BEFORE the video cut.
///
/// Requires the audio to ALREADY be two clips butted at the cut (the natural
/// distinct-sources state). A single continuous audio clip across the cut → an
/// honest `NoLinkedAudio` error (split the audio first); the verb never silently
/// splits the audio (that would allocate ids for zero semantic gain). The pure
/// decision lives in cut_core::split_edit (unit-tested + replay-checked); the
/// handler lowers it to exactly two replay-safe `edit.trim` ops (no id
/// allocation), grouped under one undo tag.
pub(in crate::dispatch) async fn edit_split_edit(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // rationale is read off `args` by commit_core
    struct Args {
        at_ms: u64,
        kind: String,
        offset_ms: u64,
        video_track: Option<String>,
        audio_track: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let kind = cut_core::SplitEditKind::parse(&a.kind).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown split-edit kind '{}'", a.kind),
            "kind must be \"j\" (audio leads / video lags) or \"l\" (video leads / audio lags)",
        )
    })?;

    // Decide under a read lock; the pure planner allocates no ids.
    let plan = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        cut_core::plan_split_edit(
            &store.project,
            a.video_track.as_deref(),
            a.audio_track.as_deref(),
            a.at_ms,
            kind,
            a.offset_ms,
        )
    };
    let plan = match plan {
        Ok(p) => p,
        Err(reject) => {
            return Err(match reject {
                cut_core::SplitEditReject::ZeroOffset => CutError::new(
                    error_codes::INVALID_ARGS,
                    "offset_ms must be greater than 0",
                    "a split edit with no offset is a no-op — pass how far to roll the audio",
                ),
                cut_core::SplitEditReject::NoVideoCut { at_ms, video_track } => CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no video cut at {at_ms}ms on track '{video_track}'"),
                    "at_ms must be the exact boundary between two adjacent video clips",
                )
                .with_at_ms(at_ms)
                .with_suggested_action(
                    "use project.state to find a clip boundary, or edit.split to make one",
                ),
                cut_core::SplitEditReject::NoLinkedAudio { side } => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("no linked audio clip at the {side} side of the cut"),
                    "a split edit rolls the audio cut against the video cut, so the audio must \
                     ALREADY be two clips butted at the boundary (the distinct-sources case). \
                     A single continuous audio clip across the cut has no audio transition to offset.",
                )
                .with_at_ms(a.at_ms)
                .with_suggested_action(
                    "split the audio at this position first (edit.split on the audio track), \
                     or detach the clip's audio (edit.detach_audio), then retry",
                ),
                cut_core::SplitEditReject::LinkedAudioDifferentTracks {
                    outgoing_track,
                    incoming_track,
                } => CutError::new(
                    error_codes::INVALID_ARGS,
                    "linked audio clips are on different tracks",
                    format!(
                        "outgoing audio is on '{outgoing_track}' but incoming audio is on '{incoming_track}'"
                    ),
                )
                .with_at_ms(a.at_ms)
                .with_suggested_action(
                    "place both audio sides on the same audio track, then retry the split edit",
                ),
                cut_core::SplitEditReject::Retimed { clip, speed } => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("audio clip '{clip}' is retimed ({speed:.3}×) — split-edit is not supported in v1"),
                    "a 1:1 ms roll assumes timeline time equals source time; a retimed clip breaks that",
                )
                .with_clip(&clip)
                .with_suggested_action("reset the audio clip's speed to 1× first (edit.speed), then retry"),
                cut_core::SplitEditReject::InsufficientHeadroom { what, available, needed } => CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("insufficient {what}: need {needed}ms but only {available}ms is available"),
                    "the offset would run the audio off the end of its source or shrink a clip to nothing",
                )
                .with_at_ms(a.at_ms)
                .with_suggested_action(format!("reduce offset_ms to {available}ms or less, or pick a different cut")),
            });
        }
    };

    // Lower to exactly two replay-safe edit.trim ops (one per audio clip), one
    // changed edge each — no id allocation, so the log replays byte-identically.
    // Both share a group tag so a single Ctrl+Z reverts the whole split edit
    // (the same grouping pattern edit.paste uses for linked A/V inserts).
    let group_id = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        format!("grp-split_edit-{}", store.log.next_id()?)
    };
    let rationale = a.rationale.clone().unwrap_or_else(|| {
        format!(
            "split edit ({}-cut): roll the audio boundary to {}ms ({}ms {} the {}ms video cut)",
            kind.as_str(),
            plan.audio_boundary_ms,
            a.offset_ms,
            if matches!(kind, cut_core::SplitEditKind::L) {
                "after"
            } else {
                "before"
            },
            plan.video_cut_ms,
        )
    });

    let mut op_ids: Vec<String> = Vec::new();
    // A_audio: only its OUT edge changes (extend for L, shorten for J).
    let res_a = commit_core(
        state,
        "edit.trim",
        json!({
            "clip": plan.audio_a,
            "src_out_ms": plan.a_new_src[1],
            "rationale": rationale,
            "group_id": group_id,
        }),
        actor.clone(),
    )
    .await?;
    if let Some(ids) = res_a.op_ids {
        op_ids.extend(ids);
    }
    // B_audio: only its IN edge changes (push later for L, pull earlier for J).
    let res_b = commit_core(
        state,
        "edit.trim",
        json!({
            "clip": plan.audio_b,
            "src_in_ms": plan.b_new_src[0],
            "rationale": rationale,
            "group_id": group_id,
        }),
        actor,
    )
    .await?;
    if let Some(ids) = res_b.op_ids {
        op_ids.extend(ids);
    }

    Ok(VerbResult::ok_with_ops(
        json!({
            "at_ms": plan.video_cut_ms,
            "kind": kind.as_str(),
            "offset_ms": a.offset_ms,
            "audio_a": plan.audio_a,
            "audio_b": plan.audio_b,
            "applied": true,
            "audio_track": plan.audio_track,
            "audio_boundary_ms": plan.audio_boundary_ms,
            // Honest before/after of the two AUDIO clips' source windows — the
            // only thing this verb changed (the video edit is untouched).
            "a_src_ms": {"old": plan.a_old_src, "new": plan.a_new_src},
            "b_src_ms": {"old": plan.b_old_src, "new": plan.b_new_src},
        }),
        op_ids,
    ))
}

/// edit.paste{clip?, asset?, to_track, at_ms, src_range_ms?, ripple?, link_audio?}
/// — Copy/Cut/Paste's PASTE half (UI Ctrl+V + the timeline context menu's
/// "Paste"; agents get it too — agent-first). A THIN verb (Option 2a): it
/// resolves a SOURCE clip → `(asset, src_in, src_out)`, then LOWERS to the
/// existing core op via `commit_core("edit.insert", …)`. Zero new
/// core/replay/rebase code — the logged op is a plain self-contained
/// `edit.insert` (the same lowering pattern as assemble.broll / import.otio /
/// screen_record.polish), so undo/diff/rebase already handle it.
///
/// SOURCE RESOLUTION (snapshot-tolerant):
///  - `clip` present + still on the timeline → read its MediaClip for the asset
///    and source window. This is the live copy/cut→paste path.
///  - `clip` absent OR no longer on the timeline → FALLBACK to `{asset,
///    src_range_ms}` (a stale clipboard snapshot whose source was since deleted
///    still pastes, because the descriptor carries asset+range). Requires both.
///
/// CURRENT LIMITS: insert-only (NO overwrite — the "Paste
/// Insert"); fresh clip via insert's make_media_clip (NO effect/grade/transform
/// copy — a pristine duplicate); single source clip (no multi-clip span); on a
/// RETIMED clip (speed ≠ 1) only the whole source window is copied (a sub-range
/// `src_range_ms` is REFUSED — the timeline→source mapping is wrong at speed≠1,
/// the same reason the trim-marquee disables there).
///
/// KIND-CHECK (like move_clip, edit.rs ~681): the source clip's track kind must
/// match the destination track kind — a paste cannot cross video↔audio. Caption
/// clips are refused (they carry absolute ranges; captions.* own them), matching
/// edit.move's refusal.
///
/// RIPPLE: explicit `ripple` wins; omitted → resolved from the destination track
/// via `is_base_track` (base track → true so AV sync is preserved, overlay/extra
/// → false so overlays float), exactly like edit.insert's default.
///
/// LINKED AUDIO (`link_audio`, default TRUE): when the source is a muxed VIDEO
/// clip whose audio rides a sibling AUDIO clip (same asset + same timeline start
/// — the linked-A/V signature the importer's auto-place mints, and the signature
/// removeItemById deletes as a pair), a SECOND edit.insert places that audio on
/// the base audio track at the same `at_ms`. Two inserts ⇒ two ops ⇒ two undos
/// (consistent with the linked-delete pair). No sibling, or `link_audio:false`,
/// or a fallback (snapshot) paste ⇒ the single video insert only.
pub(in crate::dispatch) async fn edit_paste(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // at_ms/ripple validate; the rest are read explicitly below
    struct Args {
        /// Source clip id to duplicate (the clipboard's clip). Optional so a
        /// snapshot paste of a since-deleted source still works via {asset,
        /// src_range_ms}.
        clip: Option<String>,
        /// Fallback source asset (used when `clip` is absent or gone).
        asset: Option<String>,
        to_track: String,
        at_ms: u64,
        /// Sub-range override [in,out] in SOURCE ms. Omitted = the source clip's
        /// whole window. Refused on a retimed (speed≠1) source (see v1 cuts).
        src_range_ms: Option<[u64; 2]>,
        ripple: Option<bool>,
        /// Also paste the source's linked sibling audio (default true).
        link_audio: Option<bool>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let link_audio = a.link_audio.unwrap_or(true);

    // ── Resolve the source descriptor + the optional linked-audio descriptor,
    //    holding the read lock only for the resolution (not across commits). ──
    struct AudioLink {
        track: String,       // the linked audio clip's source track (for the kind/base lookup)
        src_range: [u64; 2], // its own source window
        asset: String,
    }
    let (src_asset, src_range, audio_link): (String, [u64; 2], Option<AudioLink>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let project = &store.project;

        // Destination track must exist (insert would also error, but a clear
        // up-front message beats a generic core not_found).
        let to_kind = project
            .track(&a.to_track)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no track '{}'", a.to_track),
                    "to_track must be an existing track id (project.state lists tracks)",
                )
            })?
            .kind;

        // Try the live source clip first; fall back to the {asset, src_range}
        // snapshot when the clip id is absent or no longer on the timeline.
        let live = a
            .clip
            .as_deref()
            .and_then(|cid| project.find_clip(cid).map(|(t, i)| (cid, t.to_string(), i)));

        match live {
            Some((cid, from_track, idx)) => {
                let from_kind = project.track(&from_track).expect("track exists").kind;
                // Caption clips carry absolute ranges, not source windows —
                // refuse, mirroring edit.move (captions.* own caption edits).
                if from_kind == cut_core::TrackKind::Caption {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        "caption clips cannot be pasted with edit.paste",
                        "caption clips carry absolute ranges; use captions.* verbs",
                    )
                    .with_clip(cid));
                }
                // Cross-kind paste is rejected — same rule as move_clip.
                if from_kind != to_kind {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        format!(
                            "cannot paste a {from_kind:?} clip onto {to_kind:?} track '{}'",
                            a.to_track
                        ),
                        "source and destination track kinds must match",
                    )
                    .with_clip(cid));
                }
                let clip = &project.track(&from_track).expect("track exists").clips[idx];
                let cut_core::Clip::Media(mc) = clip else {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("'{cid}' is not a media clip"),
                        "edit.paste duplicates a media (video/audio) clip",
                    ));
                };
                // v1: a sub-range override on a RETIMED clip (constant speed≠1 OR a
                // variable-speed ramp) would map the timeline range through the
                // wrong / non-linear factor — refuse it, the same guard the UI
                // trim-marquee uses (disable when retimed).
                let retimed = mc.is_retimed();
                let src_range = match a.src_range_ms {
                    Some(_) if retimed => {
                        let how = if mc.has_speed_ramp() {
                            "has a variable-speed ramp".to_string()
                        } else {
                            format!("plays at {:.3}×", mc.speed)
                        };
                        return Err(CutError::new(
                            error_codes::INVALID_ARGS,
                            "a sub-range paste of a retimed clip is not supported",
                            format!(
                                "clip '{cid}' {how} — the timeline→source mapping is non-linear"
                            ),
                        )
                        .with_clip(cid)
                        .with_suggested_action(
                            "paste the whole clip (omit src_range_ms), or clear the retime first",
                        ));
                    }
                    Some(r) => r,
                    None => [mc.src_in_ms, mc.src_out_ms],
                };
                let src_asset = mc.asset.clone();

                // Linked-audio lookup: a muxed video clip's audio is a SEPARATE
                // clip on an audio track at the SAME timeline start with the SAME
                // asset (the auto-place / linked-insert signature). Use the EDL
                // for absolute timeline positions (clips are positional in the
                // model). Only meaningful for a video source.
                let audio_link = if link_audio && from_kind == cut_core::TrackKind::Video {
                    let edl = cut_core::edl_from_project(project);
                    // This clip's absolute timeline start.
                    let start = edl
                        .segments
                        .iter()
                        .find(|s| s.clip_id.as_deref() == Some(cid))
                        .map(|s| s.timeline_in_ms);
                    start.and_then(|start_ms| {
                        edl.segments
                            .iter()
                            .find(|s| {
                                s.track_kind == cut_core::TrackKind::Audio
                                    && s.asset.as_deref() == Some(src_asset.as_str())
                                    && s.timeline_in_ms == start_ms
                                    && s.clip_id.is_some()
                            })
                            .and_then(|s| match (s.src_in_ms, s.src_out_ms) {
                                (Some(i), Some(o)) => Some(AudioLink {
                                    track: s.track.clone(),
                                    src_range: [i, o],
                                    asset: s.asset.clone().unwrap_or_else(|| src_asset.clone()),
                                }),
                                _ => None,
                            })
                    })
                } else {
                    None
                };
                (src_asset, src_range, audio_link)
            }
            None => {
                // Fallback (snapshot): the source clip is gone (or none was
                // named). Require {asset, src_range_ms} so the descriptor is
                // self-sufficient. No linked-audio resolution is possible
                // without the live timeline — paste the single descriptor.
                let asset = a.asset.clone().ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        match a.clip.as_deref() {
                            Some(cid) => format!(
                                "source clip '{cid}' is no longer on the timeline and no fallback {{asset, src_range_ms}} was given"
                            ),
                            None => "edit.paste needs either a source `clip` or a fallback `asset` + `src_range_ms`".into(),
                        },
                        "pass `asset` + `src_range_ms` to paste a snapshot whose source clip was deleted",
                    )
                })?;
                let range = a.src_range_ms.ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "a fallback (asset-only) paste needs an explicit src_range_ms",
                        "without the source clip, the range can't be inferred",
                    )
                    .with_suggested_action("pass src_range_ms:[in,out] from the clipboard snapshot")
                })?;
                // Asset must exist in the project (insert would also reject it,
                // but the up-front message names paste's contract).
                if !project.assets.contains_key(&asset) {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        format!("no asset '{asset}' in the project"),
                        "the snapshot's asset is not (or no longer) imported",
                    ));
                }
                (asset, range, None)
            }
        }
    };

    // ── Resolve ripple from the destination track (explicit wins), then LOWER
    //    to edit.insert — the same self-contained op the live insert path logs.
    let ripple = match a.ripple {
        Some(explicit) => explicit,
        None => {
            let guard = state.project.read().await;
            let store = guard.as_ref().ok_or_else(no_project)?;
            is_base_track(&store.project, &a.to_track)
        }
    };
    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("paste clip onto {} @ {}ms", a.to_track, a.at_ms));
    // When a linked-audio half will ALSO be inserted, tag BOTH
    // inserts with one group id so a single Ctrl+Z undoes the whole linked paste
    // (not the audio then the video as two steps). The tag is the next op id —
    // globally unique in the append-only log, read before either insert appends.
    let group_id = if audio_link.is_some() {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        Some(format!("grp-paste-{}", store.log.next_id()?))
    } else {
        None
    };
    let insert_args = json!({
        "asset": src_asset,
        "track": a.to_track,
        "at_ms": a.at_ms,
        "src_range_ms": src_range,
        "ripple": ripple,
        "rationale": rationale,
        "group_id": group_id,
    });
    let res = commit_core(state, "edit.insert", insert_args, actor.clone()).await?;

    // ── Linked audio: a SECOND insert on the base audio track at the same
    //    position (two ops / two undos, mirroring the linked-delete pair). Its
    //    ripple resolves independently from ITS target track. A failure here is
    //    surfaced (the video clip already landed; we don't silently drop the
    //    audio half) but does not unwind the video paste.
    let res = if let Some(link) = audio_link {
        // linked-paste regression: the linked-audio half MUST NOT ripple.
        // When the video insert above rippled, it ALREADY opened a clip-length gap
        // across EVERY other track — including this audio track — and shifted
        // markers + captions once. A second rippling insert at the SAME at_ms would
        // open a SECOND gap in every OTHER track (PiP overlays, music/SFX beds) and
        // shift markers AGAIN, desyncing them by ~2× the clip length on an ordinary
        // mid-timeline paste. With ripple:false the audio simply drops into the gap
        // the video insert already opened (linked A/V share the same start + length),
        // and an overlay (ripple:false video) paste leaves the rest of the timeline
        // untouched as intended. Either way the audio half rippling is never wanted.
        let audio_args = json!({
            "asset": link.asset,
            "track": link.track,
            "at_ms": a.at_ms,
            "src_range_ms": link.src_range,
            "ripple": false,
            "rationale": format!("paste linked audio onto {} @ {}ms", link.track, a.at_ms),
            // Same group tag as the video half → ONE undo step.
            "group_id": group_id,
        });
        match commit_core(state, "edit.insert", audio_args, actor).await {
            Ok(audio_res) => {
                // Merge the audio op id into the result's op_ids so undo/history
                // see both halves; the primary result shape is the video insert's.
                let mut merged = res;
                if let Some(audio_ids) = audio_res.op_ids {
                    merged.op_ids.get_or_insert_with(Vec::new).extend(audio_ids);
                }
                merged
            }
            Err(e) => append_warning(
                res,
                Some(cut_core::VerbWarning {
                    code: "linked_audio_paste_failed".into(),
                    message: format!(
                        "the video clip pasted, but its linked audio did not: {}",
                        e.message
                    ),
                    detail: json!({"track": link.track, "error": e.message})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                }),
            ),
        }
    } else {
        res
    };

    // F-1: warn if the pasted clip's audio won't be mixed on a video track.
    let warning = audio_drop_warning(state, &src_asset, &a.to_track).await;
    Ok(append_warning(res, warning))
}

/// edit.gain{clip|track, db} — exactly one target must be given.
pub(in crate::dispatch) async fn edit_gain(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // db validates only; clip/track are read
    struct Args {
        clip: Option<String>,
        track: Option<String>,
        db: f64,
    }
    let a: Args = parse_args(args.clone())?;
    // Friendlier arg validation than core's (which also enforces this).
    if a.clip.is_some() == a.track.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.gain needs exactly one of clip|track",
            "both or neither were given",
        ));
    }
    commit_core(state, "edit.gain", args, actor).await
}

/// edit.fade{clip|track, in_ms?, out_ms?, kind? = "both"} — linear fades
/// under the fade-edit contract. Exactly one of clip|track; at least one of in_ms|out_ms.
/// Track form = first clip fades in / last clip fades out, resolved NOW
/// (recorded as plain clip fades — re-run after restructuring, edit.duck
/// doctrine). Honest scope: linear only, crossfades are v2. The resolved
/// kind is recorded explicitly on the op (self-contained replay).
pub(in crate::dispatch) async fn edit_fade(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses
    struct Args {
        clip: Option<String>,
        track: Option<String>,
        in_ms: Option<u64>,
        out_ms: Option<u64>,
        kind: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.clip.is_some() == a.track.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.fade needs exactly one of clip|track",
            "both or neither were given",
        ));
    }
    if a.in_ms.is_none() && a.out_ms.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.fade needs at least one of in_ms|out_ms",
            "neither was given — there is nothing to set",
        )
        .with_suggested_action("pass in_ms and/or out_ms (0 clears that side)"));
    }
    let kind = match a.kind.as_deref() {
        None => "both",
        Some(k @ ("audio" | "video" | "both")) => k,
        Some(other) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown fade kind '{other}'"),
                "must be audio|video|both (default both = whatever the track renders)",
            ))
        }
    };
    let mut args = args;
    args["kind"] = json!(kind); // self-contained op record
    commit_core(state, "edit.fade", args, actor).await
}

/// edit.mute_range{clip, range_ms?|clear} — the LOW-LEVEL half of
/// non-destructive mute-word: add one SOURCE-time mute range to a
/// media clip (normalized: sorted + merged) or clear them all. Word-addressed
/// callers use transcript.mute_words, which resolves words → these ranges.
/// All real validation (audio track, window intersection, ramp refusal) lives
/// in core edit::mute_range so live apply and replay agree exactly.
pub(in crate::dispatch) async fn edit_mute_range(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)] // shape validation; core re-parses
    struct Args {
        clip: String,
        range_ms: Option<[u64; 2]>,
        remove_ms: Option<[u64; 2]>,
        clear: Option<bool>,
    }
    let a: Args = parse_args(args.clone())?;
    let modes = usize::from(a.range_ms.is_some())
        + usize::from(a.remove_ms.is_some())
        + usize::from(a.clear.unwrap_or(false));
    if modes != 1 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "edit.mute_range needs exactly one of `range_ms`, `remove_ms`, or `clear:true`",
            "range_ms adds a mute, remove_ms surgically unmutes an interval, clear removes all",
        ));
    }
    commit_core(state, "edit.mute_range", args, actor).await
}
