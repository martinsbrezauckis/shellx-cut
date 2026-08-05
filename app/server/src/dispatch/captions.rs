//! Caption import, generation, translation, and reflow handlers.

use super::*;

const CAPTION_MAX_CHARS: usize = 42;
const CAPTION_GAP_MS: u64 = 1200;
const CAPTION_REFLOW_MIN_SPLIT_MS: u64 = 34;

/// captions.generate{style_ref?} — build the caption track from transcripts
/// mapped through the current EDL. Replaces prior generated captions
/// (regeneration is idempotent; the op records the replacement).
/// Caption reflow knobs — same units/defaults as verify.captions so reflow is
/// the FIX that satisfies the check (the measure→remedy loop, like
/// normalize_loudness ↔ the lufs check).
#[derive(Debug, Clone, Copy)]
pub(super) struct ReflowOpts {
    pub(super) max_cps: f64,
    pub(super) max_chars: usize,
    pub(super) max_duration_ms: u64,
    pub(super) min_gap_ms: u64,
}

/// Reflow a caption track to satisfy the timed-text limits. TWO distinct fixes,
/// because they address different violations:
///   - EXTEND (for reading speed): a too-fast cue (cps > max_cps) keeps its text
///     but borrows time from the gap before the next cue — pushing its end later
///     (capped at next-start − min_gap and at max_duration_ms) lowers CPS. (You
///     cannot fix CPS by splitting: chars/duration is scale-invariant.)
///   - SPLIT (for line length): a cue whose text exceeds max_chars is re-wrapped
///     at word boundaries into ≤ max_chars chunks, each chunk taking a slice of
///     the cue's span proportional to its character count.
/// Extend runs first (it may grow a cue that then needs splitting). Cues are
/// re-id'd cap_0001.. in order. Returns the new cues + a stats object
/// (cues_before/after, extended, split, still_too_fast) for the receipt.
pub(super) fn reflow_cues(
    cues: &[cut_core::CaptionClip],
    opts: ReflowOpts,
) -> (Vec<cut_core::CaptionClip>, serde_json::Value) {
    // Working copy sorted by start: (text, start, end, style_ref).
    let mut work: Vec<(String, u64, u64, Option<String>)> = cues
        .iter()
        .map(|c| {
            (
                c.text.trim().to_string(),
                c.range_ms[0],
                c.range_ms[1],
                c.style_ref.clone(),
            )
        })
        .collect();
    work.sort_by_key(|c| c.1);

    let cues_before = work.len();
    let mut extended = 0usize;
    let mut still_too_fast = 0usize;

    // EXTEND pass — lower CPS into the following gap.
    for i in 0..work.len() {
        let (ref text, start, end, _) = work[i];
        let chars = text.chars().count();
        if chars == 0 || end <= start {
            continue;
        }
        let cps = chars as f64 / ((end - start) as f64 / 1000.0);
        if cps <= opts.max_cps {
            continue;
        }
        // Duration that would hit exactly max_cps, capped at max_duration_ms.
        let want_dur = ((chars as f64 / opts.max_cps) * 1000.0).ceil() as u64;
        let want_end = start + want_dur.min(opts.max_duration_ms);
        // Cannot cross into the next cue (respect min_gap); last cue: free to
        // max_duration_ms only.
        let ceiling = if i + 1 < work.len() {
            work[i + 1].1.saturating_sub(opts.min_gap_ms)
        } else {
            want_end
        };
        let new_end = want_end.min(ceiling).max(end); // only ever extend
        if new_end > end {
            work[i].2 = new_end;
            extended += 1;
        }
        // Still over the limit after using all available gap?
        let final_cps = chars as f64 / ((work[i].2 - start) as f64 / 1000.0);
        if final_cps > opts.max_cps {
            still_too_fast += 1;
        }
    }

    // SPLIT pass — re-wrap over-length cues at word boundaries.
    let mut split = 0usize;
    let mut split_refused_short = 0usize;
    let mut out: Vec<cut_core::CaptionClip> = Vec::new();
    let mut n = 0u32;
    for (text, start, end, style_ref) in work {
        let chars = text.chars().count();
        if chars <= opts.max_chars || text.split_whitespace().count() < 2 {
            // Fits, or a single token that cannot be wrapped — keep as-is.
            n += 1;
            out.push(cut_core::CaptionClip {
                id: format!("cap_{n:04}"),
                text,
                style_ref,
                range_ms: [start, end],
            });
            continue;
        }
        // Greedy word-wrap into ≤ max_chars chunks.
        let mut chunks: Vec<String> = Vec::new();
        let mut cur = String::new();
        for w in text.split_whitespace() {
            if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > opts.max_chars {
                chunks.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(w);
        }
        if !cur.is_empty() {
            chunks.push(cur);
        }
        if chunks.len() < 2 {
            n += 1;
            out.push(cut_core::CaptionClip {
                id: format!("cap_{n:04}"),
                text: chunks.into_iter().next().unwrap_or(text),
                style_ref,
                range_ms: [start, end],
            });
            continue;
        }
        // Time-slice the span proportional to each chunk's character count.
        let total_chars: usize = chunks.iter().map(|c| c.chars().count()).sum();
        let span = end - start;
        if span < CAPTION_REFLOW_MIN_SPLIT_MS.saturating_mul(chunks.len() as u64) {
            split_refused_short += 1;
            n += 1;
            out.push(cut_core::CaptionClip {
                id: format!("cap_{n:04}"),
                text,
                style_ref,
                range_ms: [start, end],
            });
            continue;
        }
        split += 1;
        let mut cursor = start;
        let last = chunks.len() - 1;
        let mut remaining_span = span;
        let mut remaining_chars = total_chars as u64;
        for (ci, chunk) in chunks.iter().enumerate() {
            let seg = if ci == last {
                remaining_span // absorb rounding into the last slice
            } else {
                let chunk_chars = chunk.chars().count() as u64;
                let remaining_chunks_after = (last - ci) as u64;
                let reserve = CAPTION_REFLOW_MIN_SPLIT_MS * remaining_chunks_after;
                let max_seg = remaining_span.saturating_sub(reserve);
                let ideal = (remaining_span as f64 * chunk_chars as f64 / remaining_chars as f64)
                    .round() as u64;
                ideal.clamp(CAPTION_REFLOW_MIN_SPLIT_MS, max_seg)
            };
            let cstart = cursor;
            let cend = cursor.saturating_add(seg).min(end);
            cursor = cend;
            remaining_span = end.saturating_sub(cursor);
            remaining_chars = remaining_chars.saturating_sub(chunk.chars().count() as u64);
            n += 1;
            out.push(cut_core::CaptionClip {
                id: format!("cap_{n:04}"),
                text: chunk.clone(),
                style_ref: style_ref.clone(),
                range_ms: [cstart, cend],
            });
        }
    }

    let stats = json!({
        "cues_before": cues_before,
        "cues_after": out.len(),
        "extended": extended,
        "split": split,
        "split_refused_short": split_refused_short,
        "still_too_fast": still_too_fast,
    });
    (out, stats)
}

/// Harvest `(timeline_start, timeline_end, word)` triples for EVERY transcribed
/// word across all project assets, mapped source→timeline through the EDL.
/// Audio-track placements are PREFERRED (speech lives in the audio stream) so a
/// default v1+a1t first-import auto-place contributes each word ONCE, not twice
/// (caption-deduplication guard caption-doubling fix); the result is deduped + sorted by time.
///
/// Shared by `captions.generate` (which groups the words into LINE cues by char
/// budget + speech gaps) and `captions.kinetic{per_word}` (which animates each
/// WORD as its own centred cue — the 2026 word-by-word / "karaoke" style). An
/// empty Vec means no transcribed words on the timeline; each caller renders its
/// own actionable message.
pub(crate) async fn harvest_timeline_words(
    state: &AppState,
) -> Result<Vec<(u64, u64, String)>, CutError> {
    let mut words: Vec<(u64, u64, String)> = Vec::new();
    let (asset_ids, transcript_ignores): (Vec<String>, Vec<cut_core::TranscriptIgnore>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        (
            store.project.assets.keys().cloned().collect(),
            store.project.transcript_ignores.clone(),
        )
    };
    for id in &asset_ids {
        let Ok(t) = load_transcript(state, id).await else {
            continue;
        };
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let audio_tracks: Vec<String> = store
            .project
            .tracks
            .iter()
            .filter(|tr| {
                tr.kind == cut_core::TrackKind::Audio
                    && tr
                        .clips
                        .iter()
                        .any(|c| matches!(c, cut_core::Clip::Media(mc) if mc.asset == *id))
            })
            .map(|tr| tr.id.clone())
            .collect();
        for w in &t.words {
            if speech_text::transcript_word_ignored(&transcript_ignores, id, w.idx) {
                continue;
            }
            let src = [w.start_ms, w.end_ms];
            let ranges: Vec<[u64; 2]> = if audio_tracks.is_empty() {
                speech_text::source_to_timeline(&store.project, id, src, None)
            } else {
                audio_tracks
                    .iter()
                    .flat_map(|tid| {
                        speech_text::source_to_timeline(&store.project, id, src, Some(tid))
                    })
                    .collect()
            };
            for r in ranges {
                words.push((r[0], r[1], w.word.clone()));
            }
        }
    }
    // Belt-and-braces against any remaining duplicate placement shape (e.g.
    // the same source span deliberately layered on two audio tracks): a word
    // occupying the identical timeline range is one caption word, not two.
    words.sort();
    words.dedup();
    Ok(words)
}

/// captions.import — IMPORT an external SRT/VTT subtitle file as caption
/// clips on the `cap1` track, so subtitles ROUND-TRIP (the inverse of
/// export.srt). The file format is auto-detected (SRT comma / VTT dot timing;
/// VTT header + NOTE/STYLE blocks + inline tags handled). By default it REPLACES
/// the existing cap1 clips (set replace:false to MERGE — appended then re-sorted
/// by start and re-numbered). Commits as one lowered edit._set_timeline step,
/// exactly like captions.generate, so it replays/diffs cleanly and the imported
/// captions burn in / animate / export through the existing caption pipeline.
pub(super) async fn captions_import(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        path: String,
        /// Replace cap1's clips (default true) vs merge into them.
        replace: Option<bool>,
        /// Caption style key applied to every imported cue.
        style_ref: Option<String>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let raw_path = PathBuf::from(&a.path);
    let ext = raw_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "srt" | "vtt" | "ass" | "ssa") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported subtitle extension '{}'", ext),
            "captions.import only reads .srt, .vtt, .ass, or .ssa subtitle files",
        ));
    }
    {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?;
    }
    let path = std::fs::canonicalize(&raw_path).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "subtitle file does not exist",
            e.to_string(),
        )
    })?;
    if !path.is_file() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "subtitle path is not a file",
            format!("resolved path was {}", path.display()),
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "could not read subtitle file",
            e.to_string(),
        )
    })?;
    let format = cut_export::captions_in::detect_format(&content);
    let cues = cut_export::captions_in::parse(&content).map_err(export_error)?;
    let style_ref = a.style_ref.clone();
    let imported: Vec<cut_core::CaptionClip> = cues
        .iter()
        .enumerate()
        .map(|(i, c)| cut_core::CaptionClip {
            id: format!("cap_{:04}", i + 1),
            text: c.text.clone(),
            style_ref: style_ref.clone(),
            range_ms: [c.start_ms, c.end_ms],
        })
        .collect();
    let count = imported.len();
    let replace = a.replace.unwrap_or(true);

    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let mut tracks = store.project.tracks.clone();
    let (track_id, replaced) = {
        let track = match tracks
            .iter()
            .position(|t| t.kind == cut_core::TrackKind::Caption && t.id == "cap1")
            .or_else(|| {
                tracks.iter().position(|t| {
                    t.kind == cut_core::TrackKind::Caption && t.id != speech_text::TEXT_TRACK_ID
                })
            }) {
            Some(i) => &mut tracks[i],
            None => {
                tracks.push(cut_core::Track {
                    id: "cap1".into(),
                    kind: cut_core::TrackKind::Caption,
                    clips: vec![],
                    gain_db: 0.0,
                    gain_windows: vec![],
                    blend_mode: None,
                    visible: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    pan: 0.0,
                });
                tracks.last_mut().expect("just pushed")
            }
        };
        let replaced = track.clips.len();
        if replace {
            track.clips = imported.into_iter().map(cut_core::Clip::Caption).collect();
        } else {
            for nc in imported {
                track.clips.push(cut_core::Clip::Caption(nc));
            }
            // Merge: keep the track ordered by start, then re-number ids so two
            // imports never collide on cap_0001.
            track.clips.sort_by_key(|c| match c {
                cut_core::Clip::Caption(cc) => cc.range_ms[0],
                _ => 0,
            });
            let mut k = 0;
            for c in track.clips.iter_mut() {
                if let cut_core::Clip::Caption(cc) = c {
                    k += 1;
                    cc.id = format!("cap_{k:04}");
                }
            }
        }
        (track.id.clone(), replaced)
    };
    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": tracks,
            "markers": store.project.markers,
            "caption_styles": store.project.caption_styles,
        }),
    }];
    let extra = vec![effect(
        Some(&track_id),
        json!({"captions_imported": count, "replaced": replaced, "format": format}),
    )];
    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("import {count} {format} captions from {}", a.path));
    let op = guard_call("captions.import", || {
        store.apply_lowered(
            "captions.import",
            args,
            actor,
            Some(rationale),
            steps,
            extra,
        )
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "track_id": track_id,
            "caption_count": count,
            "format": format,
            "replaced": replaced,
        }),
        vec![op_id],
    ))
}

pub(super) async fn captions_generate(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        style_ref: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    // Gather (timeline_start, timeline_end, word) triples via the EDL.
    let words = harvest_timeline_words(state).await?;
    if words.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "no transcribed words on the timeline",
            "captions are generated from transcripts mapped through the EDL",
        )
        .with_suggested_action(
            // Two distinct causes share this branch — name both so an agent on
            // silent footage doesn't re-import in a loop.
            "if transcribe already ran and found no speech (transcript has 0 words), captions do not apply to this footage; otherwise media.import + wait for the transcribe job, then retry",
        ));
    }
    // Group into caption lines: char budget + speech-gap breaks.
    let mut clips: Vec<cut_core::CaptionClip> = Vec::new();
    let (mut text, mut start, mut end) = (String::new(), 0u64, 0u64);
    let mut n = 0usize;
    for (s, e, w) in words {
        let would = if text.is_empty() {
            w.len()
        } else {
            text.len() + 1 + w.len()
        };
        if !text.is_empty() && (would > CAPTION_MAX_CHARS || s.saturating_sub(end) > CAPTION_GAP_MS)
        {
            n += 1;
            clips.push(cut_core::CaptionClip {
                id: format!("cap_{n:04}"),
                text: std::mem::take(&mut text),
                style_ref: a.style_ref.clone(),
                range_ms: [start, end],
            });
        }
        if text.is_empty() {
            start = s;
        } else {
            text.push(' ');
        }
        text.push_str(&w);
        end = e;
    }
    if !text.is_empty() {
        n += 1;
        clips.push(cut_core::CaptionClip {
            id: format!("cap_{n:04}"),
            text,
            style_ref: a.style_ref.clone(),
            range_ms: [start, end],
        });
    }
    let count = clips.len();
    // Build the POST-state tracks (find-or-create caption track "cap1",
    // replace its clips) and commit as ONE lowered edit._set_timeline step —
    // captions have no per-clip core verb, and the full-snapshot step is
    // exactly how replay/diff reproduce this op. Lock held across build +
    // commit so no other writer interleaves.
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let mut tracks = store.project.tracks.clone();
    let (track_id, replaced) = {
        let caption_idx = tracks
            .iter()
            .position(|t| t.kind == cut_core::TrackKind::Caption && t.id == "cap1")
            .or_else(|| {
                tracks.iter().position(|t| {
                    t.kind == cut_core::TrackKind::Caption && t.id != speech_text::TEXT_TRACK_ID
                })
            });
        let track = match caption_idx {
            Some(i) => &mut tracks[i],
            None => {
                tracks.push(cut_core::Track {
                    id: "cap1".into(),
                    kind: cut_core::TrackKind::Caption,
                    clips: vec![],
                    gain_db: 0.0,
                    gain_windows: vec![],
                    blend_mode: None,
                    visible: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    pan: 0.0,
                });
                tracks.last_mut().expect("just pushed")
            }
        };
        let replaced = track.clips.len();
        track.clips = clips.into_iter().map(cut_core::Clip::Caption).collect();
        (track.id.clone(), replaced)
    };
    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": tracks,
            "markers": store.project.markers,
            "caption_styles": store.project.caption_styles,
        }),
    }];
    let extra = vec![effect(
        Some(&track_id),
        json!({"captions_generated": count, "replaced": replaced}),
    )];
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_inverse = wants_inverse(&args); // the inverse-payload contract: capture before args moves
    let op = guard_call("captions.generate", || {
        store.apply_lowered("captions.generate", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    // Contract (verbs.json): {track_id, caption_count}; op rides along (the inverse-payload contract: inverse trimmed).
    Ok(VerbResult::ok_with_ops(
        json!({"track_id": track_id, "caption_count": count, "op": op_for_result(&op, include_inverse)}),
        vec![op_id],
    ))
}

/// Select the caption track targeted by timeline-wide caption remedies.
/// Generated/imported captions own canonical `cap1`, so prefer it when it
/// exists. Otherwise operate on the first real caption-kind track, including
/// `txt1` created by captions.add_text. Track kind is authoritative: an
/// unrelated track merely named `cap1` must never become a caption target.
fn resolve_caption_mutation_track(project: &cut_core::Project) -> Option<(usize, String)> {
    let index = project
        .tracks
        .iter()
        .position(|track| track.kind == cut_core::TrackKind::Caption && track.id == "cap1")
        .or_else(|| {
            project
                .tracks
                .iter()
                .position(|track| track.kind == cut_core::TrackKind::Caption)
        })?;
    Some((index, project.tracks[index].id.clone()))
}

/// captions.shift{offset_ms} — bulk-shift EVERY cue on the preferred caption
/// track by `offset_ms` (negative = earlier) to correct a systematic sync offset
/// (the standard subtitle "delay/offset"). Generated/imported `cap1` is
/// preferred; otherwise the first caption-kind track (including timed-text
/// `txt1`) is used. Ranges clamp at 0 (never negative); a cue whose whole span
/// would land at/below 0 collapses to [0,0]-guarded minimum. Distinct from
/// captions.set_range (one clip). Committed as one lowered edit._set_timeline
/// step (captions.generate path).
pub(super) async fn captions_shift(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        offset_ms: i64,
    }
    let a: Args = parse_args(args.clone())?;
    if a.offset_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "offset_ms is 0 — nothing to shift",
            "pass a non-zero ms offset (negative shifts captions earlier)",
        ));
    }
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let (cap_idx, track_id) = resolve_caption_mutation_track(&store.project).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no caption or timed-text track to shift",
            "captions.generate, captions.import, and captions.add_text create caption tracks",
        )
        .with_suggested_action(
            "generate or import captions, or add a timed-text card, before captions.shift",
        )
    })?;
    let mut tracks = store.project.tracks.clone();
    let mut shifted = 0usize;
    for clip in &mut tracks[cap_idx].clips {
        if let cut_core::Clip::Caption(cc) = clip {
            // Saturating signed shift, clamped at 0 (a cue cannot start before
            // the timeline origin); a fully-negative span collapses to [0, dur].
            let dur = cc.range_ms[1].saturating_sub(cc.range_ms[0]);
            let new_start = cc.range_ms[0].saturating_add_signed(a.offset_ms);
            cc.range_ms = [new_start, new_start.saturating_add(dur)];
            shifted += 1;
        }
    }
    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": tracks,
            "markers": store.project.markers,
            "caption_styles": store.project.caption_styles,
        }),
    }];
    let extra = vec![effect(
        Some(&track_id),
        json!({"shifted": shifted, "offset_ms": a.offset_ms}),
    )];
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let op = guard_call("captions.shift", || {
        store.apply_lowered("captions.shift", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({"track_id": track_id, "shifted": shifted, "offset_ms": a.offset_ms}),
        vec![op_id],
    ))
}

/// captions.reflow{max_cps?, max_chars?, max_duration_ms?, min_gap_ms?} — the
/// FIX that satisfies verify.captions (measure→remedy loop). Re-wraps over-length
/// caption cues at word boundaries and extends too-fast cues into the following
/// gap to lower reading speed. Operates on canonical `cap1` when present,
/// otherwise on the first caption-kind track (including timed-text `txt1`), and
/// commits one lowered edit._set_timeline step (same path as captions.generate).
pub(super) async fn captions_reflow(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        max_cps: Option<f64>,
        max_chars: Option<usize>,
        max_duration_ms: Option<u64>,
        min_gap_ms: Option<u64>,
    }
    let a: Args = parse_args(args.clone())?;
    // Defaults mirror verify.captions / CaptionQcOpts so reflow targets exactly
    // what the check flags.
    let d = cut_perception::CaptionQcOpts::default();
    let opts = ReflowOpts {
        max_cps: a.max_cps.unwrap_or(d.max_cps),
        max_chars: a.max_chars.unwrap_or(d.max_chars),
        max_duration_ms: a.max_duration_ms.unwrap_or(d.max_duration_ms),
        min_gap_ms: a.min_gap_ms.unwrap_or(d.min_gap_ms),
    };
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    // Locate the preferred caption track; nothing to reflow if there is none.
    let (cap_idx, track_id) = resolve_caption_mutation_track(&store.project).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no caption or timed-text track to reflow",
            "captions.generate, captions.import, and captions.add_text create caption tracks",
        )
        .with_suggested_action(
            "generate or import captions, or add a timed-text card, before captions.reflow",
        )
    })?;
    let cues: Vec<cut_core::CaptionClip> = store.project.tracks[cap_idx]
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.clone()),
            _ => None,
        })
        .collect();
    let (new_cues, stats) = reflow_cues(&cues, opts);
    // Build POST-state tracks with the selected caption track's reflowed clips.
    let mut tracks = store.project.tracks.clone();
    tracks[cap_idx].clips = new_cues
        .iter()
        .cloned()
        .map(cut_core::Clip::Caption)
        .collect();
    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": tracks,
            "markers": store.project.markers,
            "caption_styles": store.project.caption_styles,
        }),
    }];
    let extra = vec![effect(Some(&track_id), stats.clone())];
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let op = guard_call("captions.reflow", || {
        store.apply_lowered("captions.reflow", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let mut result = stats;
    if let Value::Object(fields) = &mut result {
        fields.insert("track_id".into(), Value::String(track_id));
    }
    Ok(VerbResult::ok_with_ops(result, vec![op_id]))
}

// ---------------------------------------------------------------------------
// i18n: captions.translate + transcript.translate (TEXT translation; no dubbing)
// ---------------------------------------------------------------------------

pub(crate) fn translation_warnings_to_verb(warnings: &[String]) -> Vec<cut_core::VerbWarning> {
    warnings
        .iter()
        .map(|message| cut_core::VerbWarning {
            code: "translation_fallback".into(),
            message: message.clone(),
            detail: Default::default(),
        })
        .collect()
}

/// captions.translate{target_lang, source_lang?, backend?, mode?, source_track?,
/// position?, reflow?, model?, timeout_ms?, rationale?} — translate the caption
/// cues into `target_lang`, PRESERVING each cue's exact timeline range (one
/// source cue → one target cue at the SAME range_ms). The translation runs on
/// the user's subscription CLI (best quality) or the local MT fallback (see
/// run_translation). TEXT only — no dubbing.
///
/// `mode:"track"` (default) = add a NEW caption track in the target language
/// (source captions kept → bilingual; the translated track defaults to the TOP
/// position so it doesn't collide with the bottom source cues). `mode:"replace"`
/// = overwrite the source cues' text in place (monolingual swap; the effect of
/// captions.set_text applied to every cue, committed as one atomic snapshot).
/// `reflow:true` (mode:"track" only) re-wraps/retimes the translated cues to
/// reading-speed limits (reuses the captions.reflow logic) since a translation's
/// length differs from the source. Committed as ONE lowered edit._set_timeline step (the
/// captions.generate path) — the translated text is BAKED into the op, so replay
/// reproduces it without re-calling the CLI (deterministic despite the
/// non-deterministic backend).
#[derive(Debug, Clone)]
pub(super) struct CaptionTranslateSrcCue {
    pub(super) id: String,
    pub(super) range_ms: [u64; 2],
    pub(super) text: String,
}

pub(super) fn replace_caption_texts_by_identity(
    track: &mut cut_core::Track,
    source_cues: &[CaptionTranslateSrcCue],
    translated: &[cut_core::CaptionClip],
) -> Result<usize, CutError> {
    if source_cues.len() != translated.len() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the caption track changed during translation",
            format!(
                "had {} source cues but {} translated cues were produced",
                source_cues.len(),
                translated.len()
            ),
        )
        .with_suggested_action("retry captions.translate"));
    }

    let mut replacements = std::collections::BTreeMap::new();
    for (src, dst) in source_cues.iter().zip(translated.iter()) {
        if replacements
            .insert(
                src.id.clone(),
                (src.range_ms, src.text.clone(), dst.text.clone()),
            )
            .is_some()
        {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "the caption track changed during translation",
                format!("duplicate source cue id '{}'", src.id),
            )
            .with_suggested_action("retry captions.translate after normalizing caption ids"));
        }
    }

    let mut caption_count = 0usize;
    let mut replaced = 0usize;
    for clip in &mut track.clips {
        let cut_core::Clip::Caption(cc) = clip else {
            continue;
        };
        caption_count += 1;
        let Some((expected_range, expected_text, replacement_text)) = replacements.get(&cc.id)
        else {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "the caption track changed during translation",
                format!("cue '{}' was not in the source translation set", cc.id),
            )
            .with_suggested_action("retry captions.translate"));
        };
        if cc.range_ms != *expected_range {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "the caption track changed during translation",
                format!(
                    "cue '{}' range is {:?}, expected {:?}",
                    cc.id, cc.range_ms, expected_range
                ),
            )
            .with_suggested_action("retry captions.translate"));
        }
        if cc.text != *expected_text {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "the caption track changed during translation",
                format!("cue '{}' text changed while translation was running", cc.id),
            )
            .with_suggested_action("retry captions.translate"));
        }
        cc.text = replacement_text.clone();
        replaced += 1;
    }
    if caption_count != source_cues.len() || replaced != translated.len() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the caption track changed during translation",
            format!(
                "had {} source cues, now {} cues, replaced {}",
                source_cues.len(),
                caption_count,
                replaced
            ),
        )
        .with_suggested_action("retry captions.translate"));
    }
    Ok(replaced)
}

pub(super) async fn captions_translate(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        target_lang: String,
        source_lang: Option<String>,
        backend: Option<String>,
        mode: Option<String>,
        /// Source caption track to translate (default: cap1, else the first caption track).
        source_track: Option<String>,
        /// Vertical position for the NEW track's cues (mode:"track"); default "top".
        position: Option<String>,
        reflow: Option<bool>,
        model: Option<String>,
        timeout_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let target_lang = crate::translate::normalize_lang(&a.target_lang);
    if target_lang.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "target_lang is empty",
            "pass the language to translate INTO, e.g. \"es\" or \"Latvian\"",
        ));
    }
    let mode = a.mode.as_deref().unwrap_or("track");
    if !matches!(mode, "track" | "replace") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown mode '{mode}'"),
            "mode is track (default, add a target-language track) | replace (overwrite in place)",
        ));
    }
    if mode == "replace" && a.reflow.unwrap_or(false) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "captions.translate cannot combine mode:replace with reflow:true",
            "replace mode overwrites existing cue text 1:1 and preserves cue timing; reflow may change cue timing/count",
        )
        .with_suggested_action(
            "use mode:\"track\" with reflow:true, or run mode:\"replace\" without reflow",
        ));
    }
    if let Some(pos) = a.position.as_deref() {
        if !matches!(pos, "bottom" | "top" | "center") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown position '{pos}'"),
                "position is bottom|top|center",
            ));
        }
    }

    // 1) Collect the source cues (id, range_ms, text) IN START ORDER, under the
    //    read lock; release before spawning the (slow) translator.
    let (src_track_id, cues): (String, Vec<CaptionTranslateSrcCue>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        // Resolve the source caption track: explicit, else "cap1", else the first
        // caption track that is not the title-card track, else any caption track.
        let track = if let Some(t) = a.source_track.as_deref() {
            store
                .project
                .tracks
                .iter()
                .find(|tr| tr.id == t && tr.kind == cut_core::TrackKind::Caption)
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        format!("no caption track '{t}'"),
                        "source_track must be an existing caption track",
                    )
                })?
        } else {
            store
                .project
                .tracks
                .iter()
                .find(|tr| tr.kind == cut_core::TrackKind::Caption && tr.id == "cap1")
                .or_else(|| {
                    store.project.tracks.iter().find(|tr| {
                        tr.kind == cut_core::TrackKind::Caption
                            && tr.id != speech_text::TEXT_TRACK_ID
                    })
                })
                .or_else(|| {
                    store
                        .project
                        .tracks
                        .iter()
                        .find(|tr| tr.kind == cut_core::TrackKind::Caption)
                })
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        "no caption track to translate",
                        "generate or import captions first (captions.generate / captions.import)",
                    )
                    .with_suggested_action("run captions.generate, then captions.translate")
                })?
        };
        let mut cues: Vec<CaptionTranslateSrcCue> = track
            .clips
            .iter()
            .filter_map(|c| match c {
                cut_core::Clip::Caption(cc) => Some(CaptionTranslateSrcCue {
                    id: cc.id.clone(),
                    range_ms: cc.range_ms,
                    text: cc.text.clone(),
                }),
                _ => None,
            })
            .collect();
        cues.sort_by(|a, b| {
            a.range_ms[0]
                .cmp(&b.range_ms[0])
                .then_with(|| a.range_ms[1].cmp(&b.range_ms[1]))
                .then_with(|| a.id.cmp(&b.id))
        });
        (track.id.clone(), cues)
    };
    if cues.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "the caption track has no cues to translate",
            "add captions (captions.generate / captions.add_text) first",
        ));
    }

    // 2) Translate (CLI primary; local only when no CLI is available). Order + count preserved.
    let segments: Vec<String> = cues.iter().map(|c| c.text.clone()).collect();
    let outcome = crate::translate::run_translation(
        a.backend.as_deref(),
        a.source_lang.as_deref(),
        &target_lang,
        &segments,
        a.model.as_deref(),
        a.timeout_ms,
    )
    .await?;

    // 3) Map translations back onto the cues' EXACT ranges (count-checked).
    let ranges: Vec<[u64; 2]> = cues.iter().map(|c| c.range_ms).collect();
    let mapped = crate::translate::map_translations_to_cues(&ranges, &outcome.translations)?;

    // 4) Build the translated CaptionClips. Optional reflow re-wraps/retimes them
    //    (a translation's length differs from the source). For mode:"track" we
    //    assign lang-unique ids + a synthesized top style so the new track does
    //    not collide with the source on screen (bilingual) or by clip id.
    let lang_tag: String = target_lang
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut translated: Vec<cut_core::CaptionClip> = mapped
        .iter()
        .enumerate()
        .map(|(i, (range, text))| cut_core::CaptionClip {
            id: format!("xl_{lang_tag}_{:04}", i + 1),
            text: text.clone(),
            style_ref: None,
            range_ms: *range,
        })
        .collect();
    let mut reflow_stats: Option<Value> = None;
    if a.reflow.unwrap_or(false) {
        let d = cut_perception::CaptionQcOpts::default();
        let (reflowed, stats) = reflow_cues(
            &translated,
            ReflowOpts {
                max_cps: d.max_cps,
                max_chars: d.max_chars,
                max_duration_ms: d.max_duration_ms,
                min_gap_ms: d.min_gap_ms,
            },
        );
        // re-id to keep the lang-unique scheme (reflow re-ids to cap_NNNN).
        translated = reflowed
            .into_iter()
            .enumerate()
            .map(|(i, mut cc)| {
                cc.id = format!("xl_{lang_tag}_{:04}", i + 1);
                cc
            })
            .collect();
        reflow_stats = Some(stats);
    }
    let cues_translated = translated.len();

    // 5) Commit: build the POST-state tracks + commit ONE lowered
    //    edit._set_timeline step (the captions.generate/reflow path). Lock held
    //    across build + commit so no other writer interleaves.
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let mut tracks = store.project.tracks.clone();
    let mut styles = store.project.caption_styles.clone();
    let (target_track_id, replaced_text_count): (String, usize) = if mode == "replace" {
        // Overwrite the source cues' text in place. Match by cue id and verify
        // range/text are still the source values we translated; otherwise a
        // same-count concurrent edit can map text onto the wrong cue.
        let Some(track) = tracks.iter_mut().find(|t| t.id == src_track_id) else {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("source track '{src_track_id}' vanished"),
                "the caption track was removed concurrently",
            ));
        };
        let n = replace_caption_texts_by_identity(track, &cues, &translated)?;
        (src_track_id.clone(), n)
    } else {
        // mode == "track": a NEW caption track in the target language. Default
        // position = top (so the bottom source cues stay visible → bilingual).
        let pos = a.position.as_deref().unwrap_or("top").to_string();
        let style_key = format!("xlate_{lang_tag}");
        styles
            .entry(style_key.clone())
            .or_insert(cut_core::CaptionStyle {
                font: "Inter".into(),
                size: 42,
                color: "#fff".into(),
                bg: Some("#000c".into()),
                pos: Some(pos),
                extra: Default::default(),
            });
        for cc in &mut translated {
            cc.style_ref = Some(style_key.clone());
        }
        // Deterministic new-track id (cap_<lang>, de-duped if taken).
        let mut new_id = format!("cap_{lang_tag}");
        let mut k = 2;
        while tracks.iter().any(|t| t.id == new_id) {
            new_id = format!("cap_{lang_tag}_{k}");
            k += 1;
        }
        tracks.push(cut_core::Track {
            id: new_id.clone(),
            kind: cut_core::TrackKind::Caption,
            clips: translated
                .iter()
                .cloned()
                .map(cut_core::Clip::Caption)
                .collect(),
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        (new_id, 0)
    };

    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": tracks,
            "markers": store.project.markers,
            "caption_styles": styles,
        }),
    }];
    let extra = vec![effect(
        Some(&target_track_id),
        json!({
            "translated": cues_translated,
            "target_lang": target_lang,
            "backend": outcome.backend,
            "model": outcome.model,
            "mode": mode,
        }),
    )];
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let op = guard_call("captions.translate", || {
        store.apply_lowered("captions.translate", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });

    let mut result = json!({
        "source_lang": a.source_lang,
        "target_lang": target_lang,
        "backend": outcome.backend,
        "backend_proven": outcome.proven,
        "model": outcome.model,
        "agent": outcome.agent,
        "translation_warnings": outcome.warnings.clone(),
        "cues_translated": cues_translated,
        "mode": mode,
        "source_track": src_track_id,
        "target_track": target_track_id,
        "timestamps_preserved": true,
    });
    if mode == "replace" {
        result["replaced"] = json!(replaced_text_count);
    }
    if let Some(stats) = reflow_stats {
        result["reflow"] = stats;
    } else if mode == "track" {
        // Honest, optional follow-up (don't force it): a translation's length
        // differs from the source, so reading speed may drift.
        result["reflow_hint"] =
            json!("pass reflow:true (or run captions.reflow) if the translated cues read too fast");
    }
    Ok(VerbResult::ok_with_ops(result, vec![op_id])
        .with_warnings(translation_warnings_to_verb(&outcome.warnings)))
}
