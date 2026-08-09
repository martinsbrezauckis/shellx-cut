//! Speech/text-oriented dispatch handlers.
//!
//! Kept as a child module of `dispatch` so this extraction is behavior-preserving:
//! the handlers still share the same commit, error, and project-state helpers.
//! This first split owns transcript verbs, transcript-driven assemble/score
//! helpers, caption text/style handlers, and the shared word-to-timeline mapping.

use super::*;

/// One auto-selected highlight segment (assemble.repurpose).
struct RepurposeSeg {
    word_range: [usize; 2],
    range_ms: [u64; 2],
    text: String,
    score: f64,
    reason: String,
}

/// Auto-select the best `count` highlight segments from a word-level transcript — the
/// selection "intelligence" half of repurpose (long video → short clips) that the
/// existing `transcript.assemble` (build a reel from CHOSEN spans) and `transcript.search`
/// (find spans by keyword) lack. PURE heuristic v1 (no model, deterministic): split the
/// words into sentence-ish units at long pauses, greedily pack adjacent units into
/// ~`target_ms` candidates (capped at 1.5×), and score each by speech density + ASR
/// confidence + optional prompt-keyword overlap + duration fit. Returns the top `count`
/// by score. `word_range` indices map straight back into `transcript.assemble`.
fn repurpose_segments(
    words: &[cut_perception::WordSpan],
    count: usize,
    target_ms: u64,
    prompt: Option<&str>,
) -> Vec<RepurposeSeg> {
    if words.is_empty() {
        return Vec::new();
    }
    let norm = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    // 1) sentence-ish units: split where the gap between consecutive words exceeds PAUSE_MS.
    const PAUSE_MS: u64 = 600;
    let mut units: Vec<(usize, usize)> = Vec::new();
    let mut lo = 0usize;
    for i in 1..words.len() {
        if words[i].start_ms.saturating_sub(words[i - 1].end_ms) > PAUSE_MS {
            units.push((lo, i - 1));
            lo = i;
        }
    }
    units.push((lo, words.len() - 1));
    // 2) greedily pack units into ~target_ms candidate clips (cap 1.5× target).
    let max_ms = target_ms.saturating_mul(3) / 2;
    let mut cands: Vec<(usize, usize)> = Vec::new();
    let (mut clo, mut chi) = units[0];
    for &(ulo, uhi) in units.iter().skip(1) {
        if words[uhi].end_ms.saturating_sub(words[clo].start_ms) <= max_ms {
            chi = uhi;
        } else {
            cands.push((clo, chi));
            clo = ulo;
            chi = uhi;
        }
    }
    cands.push((clo, chi));
    // 3) score each candidate (all factors in [0,1]).
    let kws: Vec<String> = prompt
        .map(|p| {
            p.split_whitespace()
                .map(norm)
                .filter(|w| !w.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut segs: Vec<RepurposeSeg> = cands
        .iter()
        .map(|&(slo, shi)| {
            let s_ms = words[slo].start_ms;
            let e_ms = words[shi].end_ms.max(s_ms);
            let dur_s = ((e_ms - s_ms) as f64 / 1000.0).max(0.001);
            let n = (shi - slo + 1) as f64;
            let density = (n / dur_s / 2.5).min(1.0); // ≈2.5 words/s lively speech → 1.0
            let conf = {
                let cs: Vec<f64> = words[slo..=shi]
                    .iter()
                    .filter_map(|w| w.confidence.map(f64::from))
                    .collect();
                if cs.is_empty() {
                    0.8
                } else {
                    cs.iter().sum::<f64>() / cs.len() as f64
                }
            };
            let kw = if kws.is_empty() {
                0.0
            } else {
                let hits = words[slo..=shi]
                    .iter()
                    .filter(|w| kws.contains(&norm(&w.word)))
                    .count();
                (hits as f64 / kws.len() as f64).min(1.0)
            };
            let fit =
                1.0 - (((e_ms - s_ms) as f64 - target_ms as f64).abs() / target_ms as f64).min(1.0);
            let score = if kws.is_empty() {
                0.45 * density + 0.30 * conf + 0.25 * fit
            } else {
                0.30 * density + 0.20 * conf + 0.15 * fit + 0.35 * kw
            };
            let text = words[slo..=shi]
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let reason = if kws.is_empty() {
                format!("density {density:.2}, confidence {conf:.2}, duration-fit {fit:.2}")
            } else {
                format!("density {density:.2}, confidence {conf:.2}, fit {fit:.2}, keyword-overlap {kw:.2}")
            };
            RepurposeSeg {
                word_range: [words[slo].idx, words[shi].idx],
                range_ms: [s_ms, e_ms],
                text,
                score,
                reason,
            }
        })
        .collect();
    segs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    segs.truncate(count.max(1));
    segs
}

/// assemble.repurpose{asset, count?, target_ms?, prompt?} — NON-MUTATING auto-highlight
/// selection. Returns ranked candidate clips; the agent/user feeds a clip's word_range
/// into transcript.assemble (reel) or its range_ms into a sub-clip + render.reframe (short).
pub(super) async fn assemble_repurpose(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        count: Option<usize>,
        target_ms: Option<u64>,
        prompt: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let count = a.count.unwrap_or(5).clamp(1, 50);
    let target_ms = a.target_ms.unwrap_or(30_000).clamp(3_000, 600_000);
    let t = load_transcript(state, &a.asset).await?;
    if t.words.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("transcript for '{}' has no words", a.asset),
            "assemble.repurpose needs a speech transcript; this asset transcribed empty (no speech?)",
        ));
    }
    let segs = repurpose_segments(&t.words, count, target_ms, a.prompt.as_deref());
    let clips: Vec<Value> = segs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "rank": i + 1,
                "word_range": s.word_range,
                "range_ms": s.range_ms,
                "duration_ms": s.range_ms[1].saturating_sub(s.range_ms[0]),
                "text": s.text,
                "score": (s.score * 100.0).round() / 100.0,
                "reason": s.reason,
            })
        })
        .collect();
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "count": clips.len(),
        "target_ms": target_ms,
        "clips": clips,
        "next": "feed a clip's word_range into transcript.assemble{ranges:[[lo,hi]]} for a reel, or its range_ms into a sub-clip + render.reframe for a vertical short",
    })))
}

/// Map a supported target-aspect label to its ratio (width/height). Only the
/// short-form aspects assemble.shorts accepts are valid; anything else is
/// rejected by the caller (None) rather than silently defaulted.
fn aspect_ratio(aspect: &str) -> Option<f64> {
    match aspect {
        "9:16" => Some(9.0 / 16.0),
        "1:1" => Some(1.0),
        "4:5" => Some(4.0 / 5.0),
        "16:9" => Some(16.0 / 9.0),
        _ => None,
    }
}

/// Center-crop fractions (0..1 of the SOURCE frame) that reframe a
/// `src_w`×`src_h` frame to `target_ar` (= target width/height). PURE +
/// deterministic — no model, no I/O. Returns `[x, y, w, h]` fractions for a
/// centered crop:
///   • target narrower-or-equal than source (e.g. 16:9 → 9:16): keep full
///     height (h=1), crop width to `target_ar/source_ar`, center it (x).
///   • target wider than source (e.g. a tall source → 16:9): keep full width
///     (w=1), crop height to `source_ar/target_ar`, center it (y).
/// Values are rounded to 6 dp to keep the contract free of float noise. None
/// when the source dimensions are non-positive (probe missing/garbage) — the
/// caller then emits `reframe.crop = null` instead of panicking.
fn center_crop_fractions(src_w: f64, src_h: f64, target_ar: f64) -> Option<[f64; 4]> {
    if src_w <= 0.0 || src_h <= 0.0 || target_ar <= 0.0 {
        return None;
    }
    let source_ar = src_w / src_h;
    let r = |v: f64| (v * 1_000_000.0).round() / 1_000_000.0;
    let crop = if target_ar <= source_ar {
        let cw = (target_ar / source_ar).min(1.0);
        [r((1.0 - cw) / 2.0), 0.0, r(cw), 1.0]
    } else {
        let ch = (source_ar / target_ar).min(1.0);
        [0.0, r((1.0 - ch) / 2.0), 1.0, r(ch)]
    };
    Some(crop)
}

/// First ~8 words of a segment's text as a hook-line title: collapse internal
/// whitespace, take up to 8 words, then strip a trailing run of punctuation
/// ("no trailing punctuation salad"). May be empty if the span is punctuation
/// only — that is fine (never panics).
fn short_title(text: &str) -> String {
    let joined = text
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    joined
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}

/// assemble.shorts{asset, count?, target_ms?, aspect?, prompt?} — the one-call
/// AUTO-SHORTS PLANNER: a long transcribed video → N ranked vertical-short
/// PLANS. PURE orchestration over existing internals (NO new model, NO render):
/// it reuses `repurpose_segments` (the transcript ranker that backs
/// assemble.repurpose) for the ranked moments, and — BEST-EFFORT — the
/// `score_range` factor system (the engine behind score.clip) for a per-short
/// engagement breakdown when the asset has a perception report. NON-MUTATING:
/// returns plans only; materializing each short (insert + crop + captions) is a
/// flagged follow-up the `next` hint describes. Honest degradation mirrors
/// assemble.repurpose: an un-transcribed / empty-transcript asset is the same
/// INVALID_ARGS error. A missing probe → `reframe.crop = null` (+ a note),
/// never a panic; a missing perception report → `factors` omitted, the
/// transcript score still ranks the shorts.
pub(super) async fn assemble_shorts(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        count: Option<usize>,
        target_ms: Option<u64>,
        aspect: Option<String>,
        prompt: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let count = a.count.unwrap_or(5).clamp(1, 50);
    let target_ms = a.target_ms.unwrap_or(30_000).clamp(3_000, 600_000);
    let aspect = a.aspect.clone().unwrap_or_else(|| "9:16".to_string());
    let target_ar = aspect_ratio(&aspect).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("unsupported aspect '{aspect}'"),
            "aspect must be one of \"9:16\", \"1:1\", \"4:5\", \"16:9\"",
        )
    })?;

    // Transcript is REQUIRED — identical honest degradation to assemble.repurpose.
    let t = load_transcript(state, &a.asset).await?;
    if t.words.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("transcript for '{}' has no words", a.asset),
            "assemble.shorts needs a speech transcript; this asset transcribed empty (no speech?)",
        ));
    }
    let segs = repurpose_segments(&t.words, count, target_ms, a.prompt.as_deref());

    // Source frame dimensions for the center-crop reframe. Read under a short
    // lock that is dropped before the (separately-locking) perception load. The
    // crop is identical for every short (same source → same target aspect), so
    // compute it ONCE. Missing/garbage probe → None → reframe.crop = null.
    let (src_w, src_h) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let asset = store.project.assets.get(&a.asset);
        let dim = |k: &str| {
            asset
                .and_then(|x| x.probe.as_ref())
                .and_then(|p| p.get(k))
                .and_then(|v| v.as_u64())
        };
        (dim("width"), dim("height"))
    };
    let crop = match (src_w, src_h) {
        (Some(w), Some(h)) => center_crop_fractions(w as f64, h as f64, target_ar),
        _ => None,
    };
    let crop_json = match crop {
        Some([x, y, cw, ch]) => json!({ "x": x, "y": y, "w": cw, "h": ch }),
        None => Value::Null,
    };

    // BEST-EFFORT engagement factors: present only when the asset has a
    // perception report. A transcript-only asset still ranks (via the repurpose
    // score) and simply omits `factors` — never an error.
    let report = load_perception_report(state, &a.asset).await.ok();

    let shorts: Vec<Value> = segs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut obj = serde_json::Map::new();
            obj.insert("rank".into(), json!(i + 1));
            obj.insert("word_range".into(), json!(s.word_range));
            obj.insert("range_ms".into(), json!(s.range_ms));
            obj.insert(
                "duration_ms".into(),
                json!(s.range_ms[1].saturating_sub(s.range_ms[0])),
            );
            // repurpose score is 0..1 → present as 0..100, 1 dp (score.clip style).
            obj.insert("score".into(), json!((s.score * 1000.0).round() / 10.0));
            obj.insert("reason".into(), json!(s.reason));
            obj.insert("title".into(), json!(short_title(&s.text)));
            // Per-short factor breakdown for THIS range, when a report exists.
            if let Some(rep) = &report {
                if let Ok(sc) = score_range(rep, s.range_ms[0], s.range_ms[1]) {
                    let mut factors = serde_json::Map::new();
                    for f in &sc.factors {
                        factors.insert(f.key.into(), json!((f.value * 1000.0).round() / 1000.0));
                    }
                    obj.insert("factors".into(), Value::Object(factors));
                }
            }
            obj.insert(
                "reframe".into(),
                json!({ "aspect": aspect, "crop": crop_json }),
            );
            obj.insert("has_captions".into(), json!(true));
            Value::Object(obj)
        })
        .collect();

    let mut out = serde_json::Map::new();
    out.insert("asset".into(), json!(a.asset));
    out.insert("count".into(), json!(shorts.len()));
    out.insert("target_ms".into(), json!(target_ms));
    out.insert("aspect".into(), json!(aspect));
    out.insert("shorts".into(), Value::Array(shorts));
    out.insert("next".into(), json!(format!(
        "for each short: edit.insert its range_ms on a {aspect} project, apply reframe.crop via edit.crop, then captions from the transcript span"
    )));
    if crop.is_none() {
        out.insert(
            "note".into(),
            json!("reframe.crop is null — the asset's frame dimensions are unknown; run media.probe to enable the center-crop"),
        );
    }
    Ok(VerbResult::ok(Value::Object(out)))
}

/// Load an asset's FULL perception report (instrument facts: words, silences,
/// scenes, loudness, beats, black/frozen spans). The richer sibling of
/// `load_transcript` — `score.clip` (and any future engagement analysis) needs
/// the whole report, not just the transcript. Errors actionably when the asset
/// was never analyzed (its `perception` path is unset).
pub(super) async fn load_perception_report(
    state: &AppState,
    asset_id: &str,
) -> Result<cut_perception::PerceptionReport, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let asset = store.project.assets.get(asset_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no asset '{asset_id}'"),
            "unknown asset id",
        )
    })?;
    let rel = asset.perception.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("asset '{asset_id}' has no perception report yet"),
            "perception instruments have not run for this asset",
        )
        .with_suggested_action("call media.perception{asset} and wait for the job to finish")
    })?;
    let path = store.dir.join(rel);
    let rep: cut_perception::PerceptionReport =
        serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    Ok(rep)
}

/// One weighted scoring factor for score.clip — value normalized to 0..1. Only
/// factors whose underlying signal is PRESENT in the report contribute; the
/// weights renormalize over the present set, so a continuous talking-head (no
/// scene cuts → no `visual_dynamics` factor) is scored on its speech/energy
/// signals rather than penalized for a signal it legitimately lacks.
#[derive(Debug, Clone, Copy)]
struct ScoreFactor {
    key: &'static str,
    weight: f64,
    value: f64,
}

/// score.clip computed result (pre-serialization).
struct ClipScore {
    score: f64, // 0..100
    factors: Vec<ScoreFactor>,
    silence_ms: u64,
    words: usize,
    scenes: usize,
    mean_momentary_lufs: Option<f64>,
    dead_ms: u64,
}

/// Overlap length (ms) of [a0,a1) and [b0,b1).
fn span_overlap_ms(a0: u64, a1: u64, b0: u64, b1: u64) -> u64 {
    a1.min(b1).saturating_sub(a0.max(b0))
}

/// Score a SOURCE range of an asset 0..100 for "keep-worthiness" from its
/// perception report. PURE + deterministic (no model). Each engagement signal
/// becomes a 0..1 factor; the final score is the weight-renormalized blend of
/// the PRESENT factors, then scaled down by the fraction of the range that is
/// dead footage (black/frozen). Weights and the per-factor values are returned
/// so the verdict is fully auditable and an agent can re-weight.
fn score_range(
    report: &cut_perception::PerceptionReport,
    lo: u64,
    hi: u64,
) -> Result<ClipScore, CutError> {
    // Conversational pace where word_rate saturates (words/sec).
    const IDEAL_WPS: f64 = 2.6;
    // Scene cuts/min where visual_dynamics saturates.
    const IDEAL_CPM: f64 = 15.0;
    // Momentary-loudness window mapped onto 0..1 energy.
    const LUFS_FLOOR: f64 = -40.0;
    const LUFS_CEIL: f64 = -12.0;

    let range = hi.saturating_sub(lo);
    if range == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "empty range",
            "range must have hi > lo",
        ));
    }
    let secs = range as f64 / 1000.0;
    let mut factors: Vec<ScoreFactor> = Vec::new();

    let has_audio =
        report.loudness.is_some() || report.words.is_some() || !report.silences.is_empty();

    // --- speech density (1 - silence fraction); audio assets only ----------
    let silence_ms: u64 = report
        .silences
        .iter()
        .map(|s| span_overlap_ms(s.start_ms, s.end_ms, lo, hi))
        .sum();
    if has_audio {
        let density = 1.0 - (silence_ms as f64 / range as f64);
        factors.push(ScoreFactor {
            key: "speech_density",
            weight: 0.30,
            value: density.clamp(0.0, 1.0),
        });
    }

    // --- word rate + ASR confidence (transcript present) -------------------
    let mut words = 0usize;
    if let Some(t) = &report.words {
        let in_range: Vec<&cut_perception::WordSpan> = t
            .words
            .iter()
            .filter(|w| w.start_ms < hi && w.end_ms > lo)
            .collect();
        words = in_range.len();
        let rate = words as f64 / secs.max(1e-3);
        factors.push(ScoreFactor {
            key: "word_rate",
            weight: 0.15,
            value: (rate / IDEAL_WPS).clamp(0.0, 1.0),
        });
        let confs: Vec<f64> = in_range
            .iter()
            .filter_map(|w| w.confidence.map(|c| c as f64))
            .collect();
        if !confs.is_empty() {
            let mean = confs.iter().sum::<f64>() / confs.len() as f64;
            factors.push(ScoreFactor {
                key: "confidence",
                weight: 0.15,
                value: mean.clamp(0.0, 1.0),
            });
        }
    }

    // --- loudness energy (momentary windows in range) ----------------------
    let mut mean_momentary = None;
    if let Some(l) = &report.loudness {
        let win: Vec<f64> = l
            .windows
            .iter()
            .filter(|w| w.at_ms >= lo && w.at_ms < hi)
            .map(|w| w.momentary_lufs)
            .filter(|v| v.is_finite()) // ebur128 emits -inf on full silence
            .collect();
        if !win.is_empty() {
            let mean = win.iter().sum::<f64>() / win.len() as f64;
            mean_momentary = Some(mean);
            let unit = ((mean - LUFS_FLOOR) / (LUFS_CEIL - LUFS_FLOOR)).clamp(0.0, 1.0);
            factors.push(ScoreFactor {
                key: "energy",
                weight: 0.20,
                value: unit,
            });
        }
    }

    // --- visual dynamics — ONLY when the asset is actually multi-shot. A
    //     single continuous take has zero cuts; that is not "unengaging", so
    //     we drop the factor entirely (rather than score it 0) unless the asset
    //     has scene cuts somewhere.
    let scenes = report
        .scenes
        .iter()
        .filter(|s| s.at_ms >= lo && s.at_ms < hi)
        .count();
    if !report.scenes.is_empty() {
        let cpm = scenes as f64 / (secs / 60.0).max(1e-3);
        factors.push(ScoreFactor {
            key: "visual_dynamics",
            weight: 0.20,
            value: (cpm / IDEAL_CPM).clamp(0.0, 1.0),
        });
    }

    // --- dead-footage penalty (black + frozen spans) -----------------------
    let dead_ms: u64 = report
        .black_spans
        .iter()
        .chain(report.frozen_spans.iter())
        .map(|s| span_overlap_ms(s.start_ms, s.end_ms, lo, hi))
        .sum();
    let dead_frac = (dead_ms as f64 / range as f64).clamp(0.0, 1.0);

    if factors.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "perception report has no scoreable signals in this range",
            "the asset has neither audio nor visual instrument facts here",
        )
        .with_suggested_action("re-run media.perception{asset} with the full instrument set"));
    }
    let wsum: f64 = factors.iter().map(|f| f.weight).sum();
    let raw: f64 = factors.iter().map(|f| f.weight * f.value).sum::<f64>() / wsum;
    let score = ((raw * (1.0 - dead_frac)) * 1000.0).round() / 10.0; // 0..100, 1 dp
    Ok(ClipScore {
        score,
        factors,
        silence_ms,
        words,
        scenes,
        mean_momentary_lufs: mean_momentary,
        dead_ms,
    })
}

/// score.clip{clip?|asset?, range_ms?} — score a clip or asset time-range 0..100
/// for "keep-worthiness" from the perception report, so an agent (or
/// assemble.repurpose-style flow) can rank footage WITHOUT a model. NON-MUTATING.
/// Pass `clip` (its SOURCE range is scored) OR `asset` (+ optional `range_ms`,
/// default the whole asset by its probed duration). Blends speech density, word
/// rate, ASR confidence, loudness energy and — for multi-shot footage — visual
/// dynamics, renormalized over whichever signals exist, minus a dead-footage
/// (black/frozen) penalty. Returns the score with its full factor + weight +
/// raw-signal breakdown. Requires media.perception to have run on the asset.
pub(super) async fn score_clip(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        clip: Option<String>,
        asset: Option<String>,
        range_ms: Option<[u64; 2]>,
    }
    let a: Args = parse_args(args)?;

    // Resolve (asset_id, lo, hi) under a short read lock, then drop it before
    // load_perception_report takes its own.
    let (asset_id, lo, hi) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        if let Some(clip_id) = &a.clip {
            let mc = store
                .project
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .find_map(|c| match c {
                    cut_core::Clip::Media(m) if m.id == *clip_id => Some(m),
                    _ => None,
                })
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        format!("no clip '{clip_id}'"),
                        "unknown timeline clip id",
                    )
                })?;
            (mc.asset.clone(), mc.src_in_ms, mc.src_out_ms)
        } else {
            let asset_id = a.asset.clone().ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "score.clip needs `clip` or `asset`",
                    "pass a timeline clip id or an asset id",
                )
            })?;
            let asset = store.project.assets.get(&asset_id).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no asset '{asset_id}'"),
                    "unknown asset id",
                )
            })?;
            let (lo, hi) = match a.range_ms {
                Some([l, h]) => (l, h),
                None => {
                    let dur = asset
                        .probe
                        .as_ref()
                        .and_then(|p| p.get("duration_ms"))
                        .and_then(|d| d.as_u64())
                        .unwrap_or(0);
                    (0, dur)
                }
            };
            (asset_id, lo, hi)
        }
    };
    if hi <= lo {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "empty or inverted range",
            "range_ms must be [lo,hi] with hi>lo (the asset may be unprobed — duration unknown)",
        )
        .with_suggested_action("pass an explicit range_ms, or run media.import/probe first"));
    }
    let report = load_perception_report(state, &asset_id).await?;
    let sc = score_range(&report, lo, hi)?;

    let mut factors = serde_json::Map::new();
    let mut weights = serde_json::Map::new();
    for f in &sc.factors {
        factors.insert(f.key.into(), json!((f.value * 1000.0).round() / 1000.0));
        weights.insert(f.key.into(), json!(f.weight));
    }
    Ok(VerbResult::ok(json!({
        "asset": asset_id,
        "range_ms": [lo, hi],
        "duration_ms": hi - lo,
        "score": sc.score,
        "factors": factors,
        "weights": weights,
        "signals": {
            "words": sc.words,
            "silence_ms": sc.silence_ms,
            "scenes": sc.scenes,
            "mean_momentary_lufs": sc.mean_momentary_lufs,
            "dead_ms": sc.dead_ms,
        },
    })))
}

/// One script line matched against the transcript (assemble.from_script).
struct ScriptSeg {
    line_idx: usize,
    script_line: String,
    word_range: Option<[usize; 2]>,
    range_ms: Option<[u64; 2]>,
    text: String,
    score: f64,
    matched: bool,
}

/// Match a written SCRIPT to a word-level transcript: for each script line find
/// the best-overlapping contiguous span of transcript words, IN ORDER (a
/// forward cursor advances past each match, so segments never overlap and the
/// assembly follows the script). PURE + deterministic (no model): scoring is
/// token-set overlap F1 — `2*|A∩B| / (|A|+|B|)` of the normalized token sets of
/// the script line vs the candidate window, searched over window lengths within
/// ±2 of the line's token count. A line whose best window scores below
/// `min_score` is reported unmatched (kept in order, no range) rather than
/// force-fit. The matched `word_range`s feed straight into `transcript.assemble`.
fn match_script_to_words(
    words: &[cut_perception::WordSpan],
    script: &str,
    min_score: f64,
) -> Vec<ScriptSeg> {
    let norm = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    // Pre-normalize the transcript tokens once.
    let toks: Vec<String> = words.iter().map(|w| norm(&w.word)).collect();

    // Token-set F1 of two token sets.
    let f1 = |a: &std::collections::HashSet<&str>, b: &std::collections::HashSet<&str>| -> f64 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        let inter = a.iter().filter(|t| b.contains(**t)).count();
        2.0 * inter as f64 / (a.len() + b.len()) as f64
    };

    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (line_idx, raw_line) in script.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // Normalized lowercase token set of the script line.
        let script_lc: Vec<String> = line
            .split_whitespace()
            .map(norm)
            .filter(|t| !t.is_empty())
            .collect();
        let script_ref: std::collections::HashSet<&str> =
            script_lc.iter().map(|s| s.as_str()).collect();
        if script_ref.is_empty() {
            continue;
        }
        let l = script_ref.len();

        // Search forward from the cursor for the best-overlapping window.
        let lo_len = l.saturating_sub(2).max(1);
        let hi_len = l + 2;
        let mut best = (0.0f64, cursor, 0usize); // (score, start, len)
        let mut s = cursor;
        while s < toks.len() {
            let max_len = hi_len.min(toks.len() - s);
            for len in lo_len..=max_len {
                let win: std::collections::HashSet<&str> =
                    toks[s..s + len].iter().map(|t| t.as_str()).collect();
                let score = f1(&script_ref, &win);
                if score > best.0 {
                    best = (score, s, len);
                }
            }
            s += 1;
        }

        if best.0 >= min_score && best.2 > 0 {
            let (start, len) = (best.1, best.2);
            let end = start + len - 1;
            let text = words[start..=end]
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            out.push(ScriptSeg {
                line_idx,
                script_line: line.to_string(),
                word_range: Some([start, end]),
                range_ms: Some([words[start].start_ms, words[end].end_ms]),
                text,
                score: (best.0 * 1000.0).round() / 1000.0,
                matched: true,
            });
            cursor = end + 1;
        } else {
            out.push(ScriptSeg {
                line_idx,
                script_line: line.to_string(),
                word_range: None,
                range_ms: None,
                text: String::new(),
                score: (best.0 * 1000.0).round() / 1000.0,
                matched: false,
            });
        }
    }
    out
}

/// assemble.from_script{asset, script, min_score?} — SCRIPT-TO-TIMELINE matching:
/// given a written script and a transcribed asset, find the transcript span that
/// best matches each script line (in order) so the chosen spans can be strung
/// into a cut that follows the script. NON-MUTATING: returns the ordered match
/// plan; never edits the timeline. The matched `word_range`s feed straight into
/// `transcript.assemble` (the actual reel build). Deterministic token-overlap
/// matcher (no model) — a low-confidence line is reported unmatched, not
/// force-fit. Requires the asset to be transcribed (media.transcribe).
pub(super) async fn assemble_from_script(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        script: String,
        min_score: Option<f64>,
    }
    let a: Args = parse_args(args)?;
    let min_score = a.min_score.unwrap_or(0.35).clamp(0.0, 1.0);
    if a.script.split_whitespace().next().is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "empty script",
            "pass a non-empty `script` (one talking point per line)",
        ));
    }
    let transcript = load_transcript(state, &a.asset).await?;
    if transcript.words.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("asset '{}' transcribed empty (no speech?)", a.asset),
            "the transcript has no words to match the script against",
        )
        .with_suggested_action("transcribe a speech asset with media.transcribe"));
    }
    let segs = match_script_to_words(&transcript.words, &a.script, min_score);
    let matched = segs.iter().filter(|s| s.matched).count();
    let total_lines = segs.len();
    let assemble_ranges: Vec<[usize; 2]> = segs.iter().filter_map(|s| s.word_range).collect();
    let segments: Vec<Value> = segs
        .iter()
        .map(|s| {
            json!({
                "line_idx": s.line_idx,
                "script_line": s.script_line,
                "matched": s.matched,
                "score": s.score,
                "word_range": s.word_range,
                "range_ms": s.range_ms,
                "text": s.text,
            })
        })
        .collect();
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "min_score": min_score,
        "total_lines": total_lines,
        "matched": matched,
        "segments": segments,
        "assemble_ranges": assemble_ranges,
        "next": "feed assemble_ranges into transcript.assemble to build the reel in script order",
    })))
}

#[cfg(test)]
mod from_script_tests {
    use super::*;
    fn ws(idx: usize, word: &str, start_ms: u64) -> cut_perception::WordSpan {
        cut_perception::WordSpan {
            idx,
            word: word.into(),
            start_ms,
            end_ms: start_ms + 300,
            confidence: Some(0.9),
            speaker: None,
        }
    }
    fn transcript(text: &str) -> Vec<cut_perception::WordSpan> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| ws(i, w, i as u64 * 400))
            .collect()
    }

    #[test]
    fn matches_lines_in_order_and_skips_unmatched() {
        let words = transcript(
            "hello everyone today we talk about the future of artificial intelligence \
             and then some totally unrelated chatter about lunch and the weather outside \
             finally thanks so much for watching please subscribe",
        );
        let script = "the future of artificial intelligence\n\
                      a line that appears nowhere zebra qux wibble\n\
                      thanks for watching please subscribe";
        let segs = match_script_to_words(&words, script, 0.35);
        assert_eq!(segs.len(), 3);
        assert!(segs[0].matched, "score {}", segs[0].score);
        assert!(segs[0]
            .text
            .to_lowercase()
            .contains("artificial intelligence"));
        assert!(!segs[1].matched, "nonsense line, score {}", segs[1].score);
        assert!(segs[2].matched, "score {}", segs[2].score);
        assert!(segs[2].text.to_lowercase().contains("subscribe"));
        assert!(
            segs[2].word_range.unwrap()[0] > segs[0].word_range.unwrap()[1],
            "later script line must match later footage (forward cursor)"
        );
    }

    #[test]
    fn empty_script_yields_no_segments() {
        let words = transcript("a b c d e f");
        assert!(match_script_to_words(&words, "   \n  \n", 0.35).is_empty());
    }
}

#[cfg(test)]
mod score_tests {
    use super::*;
    use cut_perception::{
        Loudness, LoudnessWindow, PerceptionReport, SceneCut, Transcript, VideoSpan, WordSpan,
    };

    fn empty_report() -> PerceptionReport {
        PerceptionReport {
            schema: "shellx-cut/perception/1".into(),
            asset_hash: "sha256:test".into(),
            source_path: "/m/x.mp4".into(),
            instruments_run: vec![],
            words: None,
            silences: vec![],
            scenes: vec![],
            beats: None,
            loudness: None,
            black_spans: vec![],
            frozen_spans: vec![],
            content_bbox: None,
            subject_track: None,
            speaker_turns: vec![],
            diarization: None,
        }
    }

    fn factor<'a>(sc: &'a ClipScore, key: &str) -> Option<&'a ScoreFactor> {
        sc.factors.iter().find(|f| f.key == key)
    }

    /// Talking-head: dense clear speech + good loudness, NO scene cuts. Should
    /// score high on speech/energy and NOT carry a visual_dynamics factor (the
    /// renormalization must not penalize a continuous take for having no cuts).
    #[test]
    fn talking_head_scores_high_without_visual_factor() {
        let mut r = empty_report();
        // 26 words across 0..10s → ~2.6 wps (saturates word_rate), conf 0.9.
        let words: Vec<WordSpan> = (0..26)
            .map(|i| WordSpan {
                idx: i,
                word: "w".into(),
                start_ms: (i as u64) * 380,
                end_ms: (i as u64) * 380 + 300,
                confidence: Some(0.9),
                speaker: None,
            })
            .collect();
        r.words = Some(Transcript {
            asset: "a1".into(),
            model: "test".into(),
            language: None,
            words,
        });
        // momentary loudness ~-16 LUFS → energy ~0.857.
        r.loudness = Some(Loudness {
            integrated_lufs: -16.0,
            true_peak_dbtp: -1.0,
            windows: (0..10)
                .map(|i| LoudnessWindow {
                    at_ms: i * 1000,
                    momentary_lufs: -16.0,
                })
                .collect(),
        });
        let sc = score_range(&r, 0, 10_000).unwrap();
        assert!(sc.score > 80.0, "score={}", sc.score);
        assert!(factor(&sc, "speech_density").is_some());
        assert!(factor(&sc, "word_rate").is_some());
        assert!(factor(&sc, "confidence").is_some());
        assert!(factor(&sc, "energy").is_some());
        assert!(
            factor(&sc, "visual_dynamics").is_none(),
            "no scenes → no visual factor"
        );
    }

    /// Dead footage (a black span over half the range) roughly halves the score.
    #[test]
    fn black_span_penalizes_score() {
        let mut r = empty_report();
        r.loudness = Some(Loudness {
            integrated_lufs: -16.0,
            true_peak_dbtp: -1.0,
            windows: (0..10)
                .map(|i| LoudnessWindow {
                    at_ms: i * 1000,
                    momentary_lufs: -16.0,
                })
                .collect(),
        });
        let clean = score_range(&r, 0, 10_000).unwrap().score;
        r.black_spans = vec![VideoSpan {
            start_ms: 0,
            end_ms: 5_000,
        }];
        let dead = score_range(&r, 0, 10_000).unwrap().score;
        assert!(dead < clean, "dead {dead} should be < clean {clean}");
        // half the range is dead → ~half the score (allow rounding slack).
        assert!(
            (dead - clean * 0.5).abs() < 1.0,
            "clean={clean} dead={dead}"
        );
    }

    /// Video-only asset (scenes, no audio at all) still scores — on the single
    /// available factor — proving the present-signal renormalization.
    #[test]
    fn video_only_scores_on_visual_factor_alone() {
        let mut r = empty_report();
        r.scenes = vec![SceneCut {
            at_ms: 4_000,
            score: Some(0.7),
        }];
        let sc = score_range(&r, 0, 10_000).unwrap();
        // 1 cut in 10s → 6 cpm / 15 ideal = 0.4 → score 40.
        assert_eq!(sc.factors.len(), 1);
        assert_eq!(sc.factors[0].key, "visual_dynamics");
        assert!((sc.score - 40.0).abs() < 0.6, "score={}", sc.score);
    }

    /// A range with no signals at all is an actionable error, not a silent 0.
    #[test]
    fn no_signals_is_an_error() {
        let r = empty_report();
        assert!(score_range(&r, 0, 10_000).is_err());
    }
}

#[cfg(test)]
mod repurpose_tests {
    use super::*;
    fn w(idx: usize, word: &str, start_ms: u64, end_ms: u64) -> cut_perception::WordSpan {
        cut_perception::WordSpan {
            idx,
            word: word.into(),
            start_ms,
            end_ms,
            confidence: Some(0.9),
            speaker: None,
        }
    }
    #[test]
    fn repurpose_ranks_keyword_segment_first_and_caps_count() {
        // Three pause-separated units; the middle one mentions the prompt keyword.
        let words = vec![
            w(0, "the", 0, 300),
            w(1, "weather", 300, 800),
            w(2, "today", 800, 1300),
            // >600ms gap → new unit
            w(3, "our", 3000, 3300),
            w(4, "rocket", 3300, 3900),
            w(5, "launched", 3900, 4500),
            // >600ms gap → new unit
            w(6, "and", 7000, 7300),
            w(7, "landed", 7300, 7900),
        ];
        let segs = repurpose_segments(&words, 2, 5000, Some("rocket"));
        assert_eq!(segs.len(), 2, "count cap honored");
        assert!(
            segs[0].text.contains("rocket"),
            "keyword segment ranks first, got: {:?}",
            segs[0].text
        );
        assert!(segs[0].score >= segs[1].score, "sorted by score desc");
        assert!(segs[0].word_range[0] <= segs[0].word_range[1]);
    }
    #[test]
    fn repurpose_empty_transcript_is_empty() {
        assert!(repurpose_segments(&[], 5, 30_000, None).is_empty());
    }
}

#[cfg(test)]
mod shorts_tests {
    use super::*;

    /// 16:9 source (1920×1080) → 9:16 center crop: keep full height, crop a
    /// centered ~0.3164-wide column.
    #[test]
    fn reframe_16x9_to_9x16_center_crop() {
        let [x, y, w, h] =
            center_crop_fractions(1920.0, 1080.0, aspect_ratio("9:16").unwrap()).unwrap();
        assert!((h - 1.0).abs() < 1e-9, "h must be 1.0, got {h}");
        assert_eq!(y, 0.0, "y must be exactly 0, got {y}");
        assert!((w - 0.3164).abs() < 0.001, "w≈0.3164, got {w}");
        assert!((x - 0.3418).abs() < 0.001, "x≈0.3418, got {x}");
    }

    /// 16:9 source → 1:1 (square) center crop: keep full height, crop a centered
    /// 0.5625-wide column.
    #[test]
    fn reframe_16x9_to_square_center_crop() {
        let [_x, _y, w, h] =
            center_crop_fractions(1920.0, 1080.0, aspect_ratio("1:1").unwrap()).unwrap();
        assert!((w - 0.5625).abs() < 0.001, "w≈0.5625, got {w}");
        assert!((h - 1.0).abs() < 1e-9, "h must be 1.0, got {h}");
    }

    /// Non-positive / unknown source dims → None (caller emits crop: null, no panic).
    #[test]
    fn reframe_unknown_dims_is_none() {
        assert!(center_crop_fractions(0.0, 1080.0, 0.5625).is_none());
        assert!(center_crop_fractions(1920.0, 0.0, 0.5625).is_none());
    }
}

/// transcript.search{asset, query, case_sensitive?} — find every occurrence of a
/// word or phrase in the transcript, returning the [start_idx,end_idx] word
/// ranges (+ text + at_ms). Feed a match straight into transcript.cut_words or
/// transcript.assemble. Matching is word-sequence exact after normalization
/// (lowercase + strip surrounding punctuation, unless case_sensitive).
pub(super) async fn transcript_search(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: String,
        query: String,
        case_sensitive: Option<bool>,
    }
    let a: Args = parse_args(args)?;
    let cs = a.case_sensitive.unwrap_or(false);
    let norm = |w: &str| -> String {
        let t = w.trim_matches(|c: char| !c.is_alphanumeric());
        if cs {
            t.to_string()
        } else {
            t.to_lowercase()
        }
    };
    let q: Vec<String> = a
        .query
        .split_whitespace()
        .map(&norm)
        .filter(|w| !w.is_empty())
        .collect();
    if q.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "query is empty",
            "pass a word or phrase to find in the transcript",
        ));
    }
    let t = load_transcript(state, &a.asset).await?;
    let words: Vec<String> = t.words.iter().map(|w| norm(&w.word)).collect();
    let mut matches: Vec<Value> = Vec::new();
    if words.len() >= q.len() {
        for i in 0..=(words.len() - q.len()) {
            if (0..q.len()).all(|j| words[i + j] == q[j]) {
                let (lo, hi) = (i, i + q.len() - 1);
                let text: String = t.words[lo..=hi]
                    .iter()
                    .map(|w| w.word.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                matches.push(json!({
                    "word_range": [lo, hi], "text": text, "at_ms": t.words[lo].start_ms,
                }));
            }
        }
    }
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "query": a.query,
        "match_count": matches.len(),
        "matches": matches,
    })))
}

// ---------------------------------------------------------------------------
// transcript.chapters — deterministic TextTiling-style topic chaptering
// ---------------------------------------------------------------------------

/// Fixed token-block size (words) for the TextTiling lexical-cohesion pass.
/// Small blocks give fine boundary RESOLUTION; over-segmentation in TIME is
/// guarded separately by `min_gap_ms`, so a tight window is safe and lets short
/// videos still produce enough windows to detect a valley (a 40-token window
/// would collapse a one-minute clip into a single block).
const CHAPTER_WINDOW: usize = 10;

/// Minimum TextTiling depth (summed left+right rise out of a cohesion valley)
/// for a gap to count as a topic boundary — rejects shallow within-topic noise
/// so a single coherent topic yields exactly one chapter.
const CHAPTER_DEPTH_THRESHOLD: f64 = 0.15;

/// Common English function words dropped before computing lexical cohesion and
/// chapter titles. Tokens of ≤2 chars are dropped separately, so 1-2 char
/// function words ("a"/"to"/"of"/"in") need not be listed here.
const CHAPTER_STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "are",
    "but",
    "not",
    "you",
    "all",
    "any",
    "can",
    "her",
    "was",
    "one",
    "our",
    "out",
    "has",
    "had",
    "his",
    "how",
    "its",
    "let",
    "put",
    "say",
    "she",
    "too",
    "use",
    "that",
    "this",
    "with",
    "have",
    "from",
    "they",
    "will",
    "your",
    "what",
    "when",
    "said",
    "there",
    "their",
    "would",
    "which",
    "were",
    "been",
    "into",
    "than",
    "them",
    "then",
    "some",
    "just",
    "like",
    "over",
    "also",
    "such",
    "only",
    "very",
    "more",
    "most",
    "much",
    "many",
    "here",
    "about",
    "could",
    "should",
    "these",
    "those",
    "where",
    "while",
    "because",
    "going",
    "really",
    "actually",
    "basically",
    "okay",
];

/// Lowercased CONTENT token of a word: strip surrounding non-alphanumerics,
/// lowercase, then drop stopwords and ≤2-char tokens. `None` ⇒ the word carries
/// no topical signal (function word / filler / punctuation) and is ignored by
/// both cohesion scoring and title building.
fn chapter_token(word: &str) -> Option<String> {
    let t = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if t.chars().count() <= 2 || CHAPTER_STOPWORDS.contains(&t.as_str()) {
        None
    } else {
        Some(t)
    }
}

/// Cosine similarity of two content-word count vectors (TextTiling lexical
/// cohesion). Returns 0.0 when either side is empty; otherwise each L2 norm is
/// ≥1 (counts are ≥1), so the division can never hit zero.
fn chapter_cosine(
    a: &std::collections::HashMap<String, u32>,
    b: &std::collections::HashMap<String, u32>,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .map(|(k, &va)| b.get(k).map_or(0.0, |&vb| f64::from(va) * f64::from(vb)))
        .sum();
    let na: f64 = a
        .values()
        .map(|&v| f64::from(v) * f64::from(v))
        .sum::<f64>()
        .sqrt();
    let nb: f64 = b
        .values()
        .map(|&v| f64::from(v) * f64::from(v))
        .sum::<f64>()
        .sqrt();
    dot / (na * nb)
}

/// One adjacent-window cohesion sample. `boundary_word` is the word index at
/// which a chapter would START if this gap is chosen as a boundary (= the first
/// word of the right-hand window).
struct ChapterGap {
    boundary_word: usize,
    depth: f64,
}

/// TextTiling pass: window the words into fixed `window`-token blocks, score the
/// lexical cohesion of each ADJACENT block pair, then assign each interior gap a
/// DEPTH score (how far its cohesion valley sits below the nearest peak on each
/// side). PURE + deterministic, no model. Empty when there are <2 windows (the
/// transcript is too short to hold an interior boundary).
fn chapter_gaps(words: &[cut_perception::WordSpan], window: usize) -> Vec<ChapterGap> {
    let n = words.len();
    let w = window.max(1);
    // Window edges (exclusive hi); a chapter boundary can only fall on one.
    let mut wins: Vec<(usize, usize)> = Vec::new();
    let mut lo = 0usize;
    while lo < n {
        let hi = (lo + w).min(n);
        wins.push((lo, hi));
        lo = hi;
    }
    if wins.len() < 2 {
        return Vec::new();
    }
    // Content-word count vector per window.
    let vecs: Vec<std::collections::HashMap<String, u32>> = wins
        .iter()
        .map(|&(wlo, whi)| {
            let mut m = std::collections::HashMap::new();
            for ws in &words[wlo..whi] {
                if let Some(tok) = chapter_token(&ws.word) {
                    *m.entry(tok).or_insert(0u32) += 1;
                }
            }
            m
        })
        .collect();
    // Cohesion at each interior gap k (between window k and window k+1).
    let coh: Vec<f64> = (0..wins.len() - 1)
        .map(|k| chapter_cosine(&vecs[k], &vecs[k + 1]))
        .collect();
    // Depth score per gap: rise from the valley up to the nearest peak on each
    // side (classic TextTiling). ≈0 at a cohesion peak, large at a deep valley.
    let mut gaps = Vec::new();
    for (k, &ck) in coh.iter().enumerate() {
        let mut lpeak = ck;
        let mut i = k;
        while i > 0 && coh[i - 1] >= lpeak {
            lpeak = coh[i - 1];
            i -= 1;
        }
        let mut rpeak = ck;
        let mut j = k;
        while j + 1 < coh.len() && coh[j + 1] >= rpeak {
            rpeak = coh[j + 1];
            j += 1;
        }
        gaps.push(ChapterGap {
            boundary_word: wins[k + 1].0,
            depth: (lpeak - ck) + (rpeak - ck),
        });
    }
    gaps
}

/// Select chapter-boundary word indices from the gap depths: keep the DEEPEST
/// valleys above the depth threshold, enforcing `min_gap_ms` between boundaries
/// (and from the transcript head/tail so the first/last chapter isn't tiny) and
/// capping at `max_chapters - 1` boundaries. Returned ascending. Empty ⇒ a
/// single whole-transcript chapter.
fn chapter_boundaries(
    words: &[cut_perception::WordSpan],
    window: usize,
    max_chapters: usize,
    min_gap_ms: u64,
) -> Vec<usize> {
    if max_chapters <= 1 || words.is_empty() {
        return Vec::new();
    }
    let first_start = words[0].start_ms;
    let last_end = words[words.len() - 1].end_ms;
    let mut cands: Vec<ChapterGap> = chapter_gaps(words, window)
        .into_iter()
        .filter(|g| g.depth >= CHAPTER_DEPTH_THRESHOLD)
        .collect();
    // Deepest first; ties broken by earliest boundary for determinism.
    cands.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.boundary_word.cmp(&b.boundary_word))
    });
    let mut chosen: Vec<usize> = Vec::new();
    for g in &cands {
        if chosen.len() >= max_chapters - 1 {
            break;
        }
        let b_ms = words[g.boundary_word].start_ms;
        if b_ms.saturating_sub(first_start) < min_gap_ms
            || last_end.saturating_sub(b_ms) < min_gap_ms
        {
            continue;
        }
        let too_close = chosen.iter().any(|&c| {
            let c_ms = words[c].start_ms;
            c_ms.max(b_ms) - c_ms.min(b_ms) < min_gap_ms
        });
        if too_close {
            continue;
        }
        chosen.push(g.boundary_word);
    }
    chosen.sort_unstable();
    chosen
}

/// First ≤6 CONTENT words of a chapter's opening as a readable, title-cased
/// label. Falls back to the raw opening words when the start is all
/// stopwords/punctuation, so a title is NEVER empty.
fn chapter_title(words: &[cut_perception::WordSpan]) -> String {
    let mut picked: Vec<String> = Vec::new();
    for ws in words {
        if let Some(tok) = chapter_token(&ws.word) {
            let mut chars = tok.chars();
            let titled = match chars.next() {
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                None => continue,
            };
            picked.push(titled);
            if picked.len() >= 6 {
                break;
            }
        }
    }
    if picked.is_empty() {
        return words
            .iter()
            .take(6)
            .map(|w| w.word.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    picked.join(" ")
}

fn normalize_transcript_ignores(
    mut ranges: Vec<cut_core::TranscriptIgnore>,
) -> Vec<cut_core::TranscriptIgnore> {
    ranges.retain(|r| r.word_range[0] <= r.word_range[1] && !r.asset.trim().is_empty());
    ranges.sort_by(|a, b| {
        a.asset
            .cmp(&b.asset)
            .then(a.word_range[0].cmp(&b.word_range[0]))
            .then(a.word_range[1].cmp(&b.word_range[1]))
    });
    let mut out: Vec<cut_core::TranscriptIgnore> = Vec::with_capacity(ranges.len());
    for r in ranges {
        if let Some(last) = out.last_mut() {
            if last.asset == r.asset && r.word_range[0] <= last.word_range[1].saturating_add(1) {
                last.word_range[1] = last.word_range[1].max(r.word_range[1]);
                continue;
            }
        }
        out.push(r);
    }
    out
}

fn remove_transcript_ignore_range(
    ranges: &[cut_core::TranscriptIgnore],
    asset: &str,
    remove: [usize; 2],
) -> Vec<cut_core::TranscriptIgnore> {
    let mut out: Vec<cut_core::TranscriptIgnore> = Vec::new();
    let [rm_lo, rm_hi] = remove;
    for r in ranges {
        let [lo, hi] = r.word_range;
        if r.asset != asset || hi < rm_lo || lo > rm_hi {
            out.push(r.clone());
            continue;
        }
        if lo < rm_lo {
            out.push(cut_core::TranscriptIgnore {
                asset: r.asset.clone(),
                word_range: [lo, rm_lo.saturating_sub(1)],
            });
        }
        if rm_hi < hi {
            out.push(cut_core::TranscriptIgnore {
                asset: r.asset.clone(),
                word_range: [rm_hi.saturating_add(1), hi],
            });
        }
    }
    normalize_transcript_ignores(out)
}

pub(super) fn transcript_word_ignored(
    ranges: &[cut_core::TranscriptIgnore],
    asset: &str,
    idx: usize,
) -> bool {
    ranges
        .iter()
        .any(|r| r.asset == asset && idx >= r.word_range[0] && idx <= r.word_range[1])
}

fn split_word_range_by_ignores(
    ranges: &[cut_core::TranscriptIgnore],
    asset: &str,
    word_range: [usize; 2],
) -> Vec<[usize; 2]> {
    let [lo, hi] = word_range;
    let mut cursor = lo;
    let mut out = Vec::new();
    let mut asset_ranges: Vec<[usize; 2]> = ranges
        .iter()
        .filter(|r| r.asset == asset)
        .map(|r| r.word_range)
        .collect();
    asset_ranges.sort_unstable();
    for [ig_lo, ig_hi] in asset_ranges {
        if ig_hi < cursor {
            continue;
        }
        if ig_lo > hi {
            break;
        }
        if ig_lo > cursor {
            out.push([cursor, ig_lo - 1]);
        }
        cursor = cursor.max(ig_hi.saturating_add(1));
        if cursor > hi {
            break;
        }
    }
    if cursor <= hi {
        out.push([cursor, hi]);
    }
    out
}

/// One topic chapter (internal; serialized field-by-field by the handler).
struct Chapter {
    index: usize,
    start_ms: u64,
    end_ms: u64,
    title: String,
    word_range: [usize; 2],
}

/// Segment a word transcript into topic chapters (PURE, deterministic, no
/// model). Always returns ≥1 chapter for a non-empty transcript: when no strong
/// boundary is found the whole transcript is one chapter.
fn segment_chapters(
    words: &[cut_perception::WordSpan],
    max_chapters: usize,
    min_gap_ms: u64,
) -> Vec<Chapter> {
    if words.is_empty() {
        return Vec::new();
    }
    let n = words.len();
    let boundaries = chapter_boundaries(words, CHAPTER_WINDOW, max_chapters, min_gap_ms);
    let mut starts = Vec::new();
    starts.push(0usize);
    starts.extend(boundaries.iter().copied());
    starts
        .iter()
        .enumerate()
        .map(|(i, &lo)| {
            let hi = starts.get(i + 1).copied().unwrap_or(n); // exclusive
            let last = hi - 1;
            Chapter {
                index: i,
                start_ms: words[lo].start_ms,
                end_ms: words[last].end_ms,
                title: chapter_title(&words[lo..hi]),
                word_range: [words[lo].idx, words[last].idx],
            }
        })
        .collect()
}

/// transcript.chapters{asset, max_chapters?, min_gap_ms?} — NON-MUTATING
/// deterministic auto-chaptering for podcast and platform workflows. Splits a
/// transcribed video into topic chapters by TextTiling
/// lexical cohesion (deterministic, no model) and returns the list; the
/// agent/user drops a marker per chapter (edit.add_marker) or writes them with
/// export.chapters. Honest degradation: an un-transcribed / empty transcript is
/// the SAME INVALID_ARGS as assemble.repurpose.
pub(super) async fn transcript_chapters(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        max_chapters: Option<usize>,
        min_gap_ms: Option<u64>,
    }
    let a: Args = parse_args(args)?;
    let max_chapters = a.max_chapters.unwrap_or(12).clamp(1, 50);
    let min_gap_ms = a.min_gap_ms.unwrap_or(20_000);
    let t = load_transcript(state, &a.asset).await?;
    if t.words.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("transcript for '{}' has no words", a.asset),
            "transcript.chapters needs a speech transcript; this asset transcribed empty (no speech?)",
        ));
    }
    let chapters = segment_chapters(&t.words, max_chapters, min_gap_ms);
    let out: Vec<Value> = chapters
        .iter()
        .map(|c| {
            json!({
                "index": c.index,
                "start_ms": c.start_ms,
                "end_ms": c.end_ms,
                "title": c.title,
                "word_range": c.word_range,
            })
        })
        .collect();
    Ok(VerbResult::ok(json!({
        "asset": a.asset,
        "count": out.len(),
        "chapters": out,
        "next": "drop a marker at each start_ms (edit.add_marker), or write them all with export.chapters",
    })))
}

#[cfg(test)]
mod chapters_tests {
    use super::*;
    fn cw(idx: usize, word: &str, start_ms: u64) -> cut_perception::WordSpan {
        cut_perception::WordSpan {
            idx,
            word: word.into(),
            start_ms,
            end_ms: start_ms + 400,
            confidence: None,
            speaker: None,
        }
    }

    /// Two clearly distinct topics (cooking → space) must split into ≥2
    /// chapters with the boundary falling BETWEEN the topics and each title
    /// reflecting its topic.
    #[test]
    fn segments_two_topics_at_the_boundary() {
        let cooking = ["cooking", "recipes", "pasta", "sauce"];
        let space = ["space", "rockets", "orbit", "launch"];
        let mut words = Vec::new();
        for i in 0..30usize {
            words.push(cw(i, cooking[i % 4], i as u64 * 1000));
        }
        for i in 0..30usize {
            let g = 30 + i;
            words.push(cw(g, space[i % 4], g as u64 * 1000));
        }
        let chapters = segment_chapters(&words, 12, 20_000);
        assert!(
            chapters.len() >= 2,
            "expected ≥2 chapters across two topics, got {}",
            chapters.len()
        );
        // The second chapter starts exactly where the topic switches (word 30).
        assert_eq!(
            chapters[1].word_range[0], 30,
            "boundary should fall between the two topics"
        );
        assert!(
            chapters[0].title.to_lowercase().contains("cooking"),
            "chapter 1 title should reflect the cooking topic, got {:?}",
            chapters[0].title
        );
        assert!(
            chapters[1].title.to_lowercase().contains("space"),
            "chapter 2 title should reflect the space topic, got {:?}",
            chapters[1].title
        );
    }

    /// A single coherent topic yields exactly one whole-transcript chapter.
    #[test]
    fn single_topic_yields_one_chapter() {
        let cooking = ["cooking", "recipes", "pasta", "sauce"];
        let words: Vec<_> = (0..30usize)
            .map(|i| cw(i, cooking[i % 4], i as u64 * 1000))
            .collect();
        let chapters = segment_chapters(&words, 12, 20_000);
        assert_eq!(chapters.len(), 1, "one topic ⇒ one chapter");
        assert_eq!(chapters[0].word_range, [0, 29]);
    }
}

/// Map a SOURCE-time range of an asset to TIMELINE ranges by walking the
/// current EDL (a source span may appear 0..n times on the timeline).
/// `track` narrows the walk to one track's segments under the scope contract;
/// None scans the whole timeline.
pub(super) fn source_to_timeline(
    project: &cut_core::Project,
    asset_id: &str,
    src: [u64; 2],
    track: Option<&str>,
) -> Vec<[u64; 2]> {
    let edl = cut_core::edl_from_project(project);
    let mut out = Vec::new();
    for seg in &edl.segments {
        if seg.asset.as_deref() != Some(asset_id) {
            continue;
        }
        if track.is_some_and(|t| t != seg.track) {
            continue;
        }
        let (Some(s_in), Some(s_out)) = (seg.src_in_ms, seg.src_out_ms) else {
            continue;
        };
        let lo = src[0].max(s_in);
        let hi = src[1].min(s_out);
        if lo < hi {
            // Source offsets into the clip map to timeline through the clip's
            // speed (identity at 1.0) — the centralized time-remap invariant.
            let t0 = seg.timeline_in_ms + cut_core::src_off_to_tl(lo - s_in, seg.speed);
            let t1 = seg.timeline_in_ms + cut_core::src_off_to_tl(hi - s_in, seg.speed);
            out.push([t0, t1]);
        }
    }
    out.sort();
    out
}

/// Map a SOURCE-time range to the timeline range of ONE specific clip (by id),
/// the SELECTED-CLIP scope: the word is cut from THIS clip only, not from
/// every other clip of the same asset. Returns None when the clip is gone, is not
/// a media clip, or the source span falls outside it. The single timeline range is
/// then ripple-deleted ALL-TRACKS (the existing cut_words path), which also carries
/// the linked audio clip sharing that timeline window — V and A stay in sync.
fn clip_source_to_timeline(
    project: &cut_core::Project,
    clip_id: &str,
    src: [u64; 2],
) -> Option<[u64; 2]> {
    let edl = cut_core::edl_from_project(project);
    let seg = edl
        .segments
        .iter()
        .find(|s| s.clip_id.as_deref() == Some(clip_id))?;
    let (s_in, s_out) = (seg.src_in_ms?, seg.src_out_ms?);
    let lo = src[0].max(s_in);
    let hi = src[1].min(s_out);
    if lo >= hi {
        return None;
    }
    let t0 = seg.timeline_in_ms + cut_core::src_off_to_tl(lo - s_in, seg.speed);
    let t1 = seg.timeline_in_ms + cut_core::src_off_to_tl(hi - s_in, seg.speed);
    Some([t0, t1])
}

/// transcript.get{asset} — words + indices (read-only).
pub(super) async fn transcript_get(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
    }
    let a: Args = parse_args(args)?;
    let t = load_transcript(state, &a.asset).await?;
    Ok(VerbResult::ok(serde_json::to_value(&t)?))
}

/// transcript.timeline{clip?, track?} — the EDL-AWARE transcript: map each
/// asset word to its position ON THE TIMELINE through the current EDL, so the
/// panel, captions, and search all read ONE shared mapping instead of raw
/// per-asset blobs that still show words already cut. For every media segment (a
/// trimmed clip on a video/audio track) it emits the asset words inside the
/// clip's source window [src_in, src_out], each carrying its owning `clip_id` +
/// `track` and its `timeline_start_ms`/`timeline_end_ms` (= clip.timeline_in +
/// src_off_to_tl(word − src_in, speed) — the same remap `source_to_timeline`
/// uses, so cut/search/captions can't drift). A word reused across several clips
/// appears once PER clip. `clip` narrows to ONE clip (the SELECTED-CLIP panel
/// view); `track` narrows to one track; BOTH omitted = the PROGRAM transcript
/// (the output line in timeline order, with the linked video/audio pair
/// de-duplicated to one entry per spoken word, preferring the video clip). An
/// asset with no transcript yet simply contributes no words. Read-only.
pub(super) async fn transcript_timeline(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        clip: Option<String>,
        track: Option<String>,
    }
    let a: Args = parse_args(args)?;

    // 1) Collect the target media segments as owned data UNDER the read lock, then
    //    release it — load_transcript re-acquires the lock per asset.
    struct Seg {
        clip_id: Option<String>,
        track: String,
        is_video: bool,
        asset: String,
        src_in: u64,
        src_out: u64,
        tl_in: u64,
        speed: f64,
    }
    let (segs, transcript_ignores): (Vec<Seg>, Vec<cut_core::TranscriptIgnore>) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let edl = cut_core::edl_from_project(&store.project);
        let segs = edl
            .segments
            .iter()
            .filter(|s| {
                s.asset.is_some()
                    && s.src_in_ms.is_some()
                    && s.src_out_ms.is_some()
                    && matches!(
                        s.track_kind,
                        cut_core::TrackKind::Video | cut_core::TrackKind::Audio
                    )
                    && a.clip
                        .as_deref()
                        .is_none_or(|c| s.clip_id.as_deref() == Some(c))
                    && a.track.as_deref().is_none_or(|t| s.track == t)
            })
            .map(|s| Seg {
                clip_id: s.clip_id.clone(),
                track: s.track.clone(),
                is_video: matches!(s.track_kind, cut_core::TrackKind::Video),
                asset: s.asset.clone().unwrap_or_default(),
                src_in: s.src_in_ms.unwrap_or(0),
                src_out: s.src_out_ms.unwrap_or(0),
                tl_in: s.timeline_in_ms,
                speed: s.speed,
            })
            .collect();
        (segs, store.project.transcript_ignores.clone())
    };

    // 2) Load each referenced asset's transcript ONCE (best-effort).
    let mut cache: std::collections::HashMap<String, Option<cut_perception::Transcript>> =
        std::collections::HashMap::new();
    for s in &segs {
        if !cache.contains_key(&s.asset) {
            let t = load_transcript(state, &s.asset).await.ok();
            cache.insert(s.asset.clone(), t);
        }
    }

    // 3) Map each clip's words to timeline entries.
    let mut entries: Vec<Value> = Vec::new();
    for s in &segs {
        let Some(Some(t)) = cache.get(&s.asset) else {
            continue;
        };
        for w in &t.words {
            if transcript_word_ignored(&transcript_ignores, &s.asset, w.idx) {
                continue;
            }
            // The word must overlap the clip's source window (it may straddle an edge).
            if w.end_ms <= s.src_in || w.start_ms >= s.src_out {
                continue;
            }
            let ws = w.start_ms.max(s.src_in);
            let we = w.end_ms.min(s.src_out);
            let tl_start = s.tl_in + cut_core::src_off_to_tl(ws - s.src_in, s.speed);
            let tl_end = s.tl_in + cut_core::src_off_to_tl(we - s.src_in, s.speed);
            entries.push(json!({
                "clip_id": s.clip_id,
                "track": s.track,
                "track_kind": if s.is_video { "video" } else { "audio" },
                "asset": s.asset,
                "word_index": w.idx,
                "word": w.word,
                "src_start_ms": w.start_ms,
                "src_end_ms": w.end_ms,
                "timeline_start_ms": tl_start,
                "timeline_end_ms": tl_end,
                // Speaker label (media.diarize); omitted when the word is not
                // speaker-labeled, so the panel/captions only show it once diarized.
                "speaker": w.speaker,
            }));
        }
    }

    // 4) PROGRAM view (no clip/track filter): de-dup the linked video/audio pair so
    //    each spoken word appears once. Sort by (asset, word_index, timeline_start)
    //    with VIDEO first, then drop the audio duplicate.
    if a.clip.is_none() && a.track.is_none() {
        entries.sort_by(|x, y| {
            let kx = (
                x["asset"].as_str(),
                x["word_index"].as_u64(),
                x["timeline_start_ms"].as_u64(),
            );
            let ky = (
                y["asset"].as_str(),
                y["word_index"].as_u64(),
                y["timeline_start_ms"].as_u64(),
            );
            kx.cmp(&ky).then_with(|| {
                let vx = x["track_kind"] == "video";
                let vy = y["track_kind"] == "video";
                vy.cmp(&vx) // video (true) sorts before audio (false)
            })
        });
        entries.dedup_by(|x, y| {
            x["asset"] == y["asset"]
                && x["word_index"] == y["word_index"]
                && x["timeline_start_ms"] == y["timeline_start_ms"]
        });
    }

    // 5) Final timeline order (stable).
    entries.sort_by_key(|e| e["timeline_start_ms"].as_u64().unwrap_or(0));

    Ok(VerbResult::ok(json!({
        "clip": a.clip,
        "track": a.track,
        "word_count": entries.len(),
        "entries": entries,
    })))
}

/// Ripple-delete a list of timeline ranges, ONE op per range (public verb contract:
/// skim-reviewable). Ranges are applied in REVERSE timeline order so earlier
/// ranges stay valid while later ones are removed. Each entry may carry
/// extra OpEffects recorded on its op — word-addressed removals attach
/// `{asset, word_range}` so the transcript panel can strike the exact words
/// (panels/Review/shared.ts activeCutSpans reads it off effects).
/// `count_field` names the span count in the result per the verbs.json
/// contract ("spans_removed" for remove_silences, "fillers_removed" for
/// remove_fillers, …); `total_removed_ms` always rides alongside.
async fn ripple_ranges(
    state: &AppState,
    verb: &str,
    base_args: &Value,
    actor: Actor,
    mut ranges: Vec<([u64; 2], Vec<OpEffect>)>,
    count_field: &str,
) -> Result<VerbResult, CutError> {
    if ranges.is_empty() {
        let mut r = Map::new();
        r.insert(count_field.into(), json!(0));
        r.insert("total_removed_ms".into(), json!(0));
        r.insert("note".into(), json!("nothing matched"));
        return Ok(VerbResult::ok(Value::Object(r)));
    }
    ranges.sort_by_key(|(r, _)| *r);
    ranges.dedup_by_key(|(r, _)| *r);
    let total_removed_ms: u64 = ranges.iter().map(|(r, _)| r[1].saturating_sub(r[0])).sum();
    let mut op_ids = Vec::new();
    for (range, extra) in ranges.iter().rev() {
        let mut args = base_args.clone();
        args["range_ms"] = json!(range);
        // One op per span (skim-reviewable), each lowered to its exact
        // edit.ripple_delete so replay/diff reproduce it (apply_record).
        let steps = vec![InverseOp {
            verb: "edit.ripple_delete".into(),
            args: json!({"range_ms": range}),
        }];
        let r = commit_lowered(state, verb, args, actor.clone(), steps, extra.clone()).await?;
        if let Some(ids) = r.op_ids {
            op_ids.extend(ids);
        }
    }
    op_ids.reverse();
    let mut result = Map::new();
    result.insert(count_field.into(), json!(ranges.len()));
    result.insert("total_removed_ms".into(), json!(total_removed_ms));
    Ok(VerbResult::ok_with_ops(Value::Object(result), op_ids))
}

/// transcript.cut_words{asset, word_range:[a,b]} — ripple-cut the timeline
/// ranges covering words a..=b, padded ±40ms to word edges (never inside a
/// word, public verb contract).
pub(super) async fn transcript_cut_words(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        word_range: [usize; 2],
        /// SELECTED-CLIP scope: cut from this clip only, not every
        /// occurrence of the asset on the timeline. Omit for the legacy behavior.
        #[serde(default)]
        clip: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let t = load_transcript(state, &a.asset).await?;
    let (lo, hi) = (a.word_range[0], a.word_range[1]);
    if lo > hi || hi >= t.words.len() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("word_range [{lo},{hi}] out of bounds"),
            format!(
                "transcript has {} words (indices 0..={})",
                t.words.len(),
                t.words.len().saturating_sub(1)
            ),
        ));
    }
    // Source span padded to word edges ±40ms (cut OUTSIDE the words).
    let src = [
        t.words[lo]
            .start_ms
            .saturating_sub(cut_perception::CUT_ON_WORD_TOLERANCE_MS),
        t.words[hi].end_ms + cut_perception::CUT_ON_WORD_TOLERANCE_MS,
    ];
    let ranges: Vec<([u64; 2], Vec<OpEffect>)> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        // SELECTED-CLIP scope: one timeline range (this clip only); else the legacy
        // every-occurrence walk. Attach the word addressing to each op's effects
        // (transcript strike rendering reads {asset, word_range}; args carry it too,
        // but the effects entry keeps cut_words/remove_fillers shapes uniform).
        let tl_ranges: Vec<[u64; 2]> = match a.clip.as_deref() {
            Some(clip_id) => clip_source_to_timeline(&store.project, clip_id, src)
                .into_iter()
                .collect(),
            None => source_to_timeline(&store.project, &a.asset, src, None),
        };
        tl_ranges
            .into_iter()
            .map(|r| {
                (
                    r,
                    vec![effect(
                        None,
                        json!({"asset": a.asset, "word_range": [lo, hi]}),
                    )],
                )
            })
            .collect()
    };
    if ranges.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "those words are not on the timeline",
            format!(
                "source span {src:?} of '{}' maps to no current timeline range",
                a.asset
            ),
        )
        .with_suggested_action("the span may already be cut — check project.state"));
    }
    // Contract (verbs.json): {removed_ms, word_range, text} — the cut span.
    let text: String = t.words[lo..=hi]
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let envelope = ripple_ranges(
        state,
        "transcript.cut_words",
        &args,
        actor,
        ranges,
        "spans_removed",
    )
    .await?;
    let mut result = envelope.result.clone().unwrap_or_else(|| json!({}));
    result["removed_ms"] = result["total_removed_ms"].clone();
    result["word_range"] = json!([lo, hi]);
    result["text"] = json!(text);
    Ok(VerbResult {
        result: Some(result),
        ..envelope
    })
}

/// transcript.ignore_words{asset, word_range:[a,b], remove?} — NON-DESTRUCTIVE
/// transcript-state ignore. Unlike cut_words it does not ripple; unlike
/// mute_words it does not change audio. Consumers that build from transcript
/// text (captions.generate, transcript.assemble) skip ignored source words by
/// default while the source view can still show them quietly.
pub(super) async fn transcript_ignore_words(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        word_range: [usize; 2],
        #[serde(default)]
        remove: bool,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let t = load_transcript(state, &a.asset).await?;
    let (lo, hi) = (a.word_range[0], a.word_range[1]);
    if lo > hi || hi >= t.words.len() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("word_range [{lo},{hi}] out of bounds"),
            format!(
                "transcript has {} words (indices 0..={})",
                t.words.len(),
                t.words.len().saturating_sub(1)
            ),
        ));
    }
    let text: String = t.words[lo..=hi]
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let before = normalize_transcript_ignores(store.project.transcript_ignores.clone());
    let next = if a.remove {
        remove_transcript_ignore_range(&before, &a.asset, [lo, hi])
    } else {
        let mut ranges = before.clone();
        ranges.push(cut_core::TranscriptIgnore {
            asset: a.asset.clone(),
            word_range: [lo, hi],
        });
        normalize_transcript_ignores(ranges)
    };
    let changed = before != next;
    let action = if a.remove { "remove" } else { "add" };
    if !changed {
        return Ok(VerbResult::ok(json!({
            "asset": a.asset,
            "word_range": [lo, hi],
            "text": text,
            "action": action,
            "changed": false,
            "transcript_ignores": next,
        })));
    }

    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": store.project.tracks,
            "markers": store.project.markers,
            "caption_styles": store.project.caption_styles,
            "adjustments": store.project.adjustments,
            "nests": store.project.nests,
            "transcript_ignores": next.clone(),
        }),
    }];
    let extra = vec![effect(
        None,
        json!({
            "asset": a.asset.clone(),
            "word_range": [lo, hi],
            "text": text.clone(),
            "action": action,
            "transcript_ignores": next.clone(),
        }),
    )];
    let rationale = a.rationale.or_else(|| {
        Some(if a.remove {
            format!("unignore transcript words [{lo}..={hi}]: \"{text}\"")
        } else {
            format!("ignore transcript words [{lo}..={hi}]: \"{text}\"")
        })
    });
    let op = guard_call("transcript.ignore_words", || {
        store.apply_lowered(
            "transcript.ignore_words",
            args,
            actor,
            rationale,
            steps,
            extra,
        )
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({
            "asset": a.asset,
            "word_range": [lo, hi],
            "text": text,
            "action": action,
            "changed": true,
            "transcript_ignores": next,
        }),
        vec![op_id],
    ))
}

/// transcript.mute_words{asset, word_range:[a,b], clip?} — NON-DESTRUCTIVE word
/// mute: silence the padded source span of words a..=b wherever it
/// is audible on the timeline WITHOUT cutting or rippling anything — the
/// non-destructive sibling of transcript.cut_words. The span (±40ms word-edge
/// padding, same tolerance as cut_words) is SOURCE time, and lands as
/// `MediaClip.mute_ranges` entries via one edit.mute_range op per matching
/// AUDIO-track clip (same asset, visible window intersects; `clip` scopes to
/// one clip) — so the mute stays glued to the words through later trims /
/// slips / splits. PLAN-THEN-APPLY: every target is validated before the first
/// op commits (the assemble rule: a bad target never commits a partial mute).
/// Speed-RAMPED clips are reported in `skipped[]`, never silently ignored.
/// Undo per op via project.undo; unmute via edit.mute_range{clip, clear:true}.
pub(super) async fn transcript_mute_words(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: String,
        word_range: [usize; 2],
        /// SELECTED-CLIP scope: mute in this clip only.
        #[serde(default)]
        clip: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let t = load_transcript(state, &a.asset).await?;
    let (lo, hi) = (a.word_range[0], a.word_range[1]);
    if lo > hi || hi >= t.words.len() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("word_range [{lo},{hi}] out of bounds"),
            format!(
                "transcript has {} words (indices 0..={})",
                t.words.len(),
                t.words.len().saturating_sub(1)
            ),
        ));
    }
    // Padded SOURCE span — mute OUTSIDE the word edges (public verb contract, same as cut).
    let src = [
        t.words[lo]
            .start_ms
            .saturating_sub(cut_perception::CUT_ON_WORD_TOLERANCE_MS),
        t.words[hi].end_ms + cut_perception::CUT_ON_WORD_TOLERANCE_MS,
    ];
    let text: String = t.words[lo..=hi]
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    // PLAN: audio-track media clips of this asset whose visible window overlaps
    // the span. Ramped clips are recorded as skipped (edit.mute_range would
    // refuse them); nothing applies until the whole plan is known-good.
    let mut targets: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut clip_seen = false;
    for track in store
        .project
        .tracks
        .iter()
        .filter(|t| t.kind == cut_core::TrackKind::Audio)
    {
        for c in track.clips.iter().filter_map(|c| match c {
            cut_core::Clip::Media(m) if m.asset == a.asset => Some(m),
            _ => None,
        }) {
            if let Some(want) = &a.clip {
                if &c.id != want {
                    continue;
                }
            }
            clip_seen = true;
            if src[1].min(c.src_out_ms) <= src[0].max(c.src_in_ms) {
                continue; // words not inside this clip's visible window
            }
            if c.speed_ramp.is_some() {
                skipped.push(json!({
                    "clip": c.id,
                    "reason": "speed ramp — mute ranges need a linear source→output mapping",
                }));
                continue;
            }
            targets.push(c.id.clone());
        }
    }
    if targets.is_empty() {
        let (msg, ctx) = if let Some(want) = &a.clip {
            if clip_seen {
                (
                    format!("clip '{want}' does not play words [{lo},{hi}]"),
                    format!(
                        "source span {src:?} is outside the clip's visible window, or the clip is ramped"
                    ),
                )
            } else {
                (
                    format!("clip '{want}' is not an audio clip of asset '{}'", a.asset),
                    "mute targets audio-track clips (only audio tracks reach the mix)".to_string(),
                )
            }
        } else {
            (
                "those words are not audible on the timeline".to_string(),
                format!(
                    "no audio-track clip of '{}' plays source span {src:?}",
                    a.asset
                ),
            )
        };
        return Err(CutError::new(error_codes::NOT_FOUND, msg, ctx).with_suggested_action(
            "the span may be cut, video-only (edit.detach_audio first), or ramped — check project.state",
        ));
    }
    // APPLY: one edit.mute_range op per clip (each individually undoable).
    let rationale = format!("mute words [{lo}..={hi}]: \"{text}\"");
    let mut op_ids: Vec<String> = Vec::new();
    let mut ops: Vec<cut_core::OpRecord> = Vec::new();
    let mut muted: Vec<Value> = Vec::new();
    for clip in &targets {
        let op = guard_call("edit.mute_range", || {
            store.apply(
                "edit.mute_range",
                json!({"clip": clip, "range_ms": src}),
                actor.clone(),
                Some(rationale.clone()),
            )
        })?;
        op_ids.push(op.op_id.clone());
        ops.push(op);
        muted.push(json!({"clip": clip, "range_ms": src}));
    }
    for op in ops {
        state.events.publish(Event::OpApplied { op });
    }
    Ok(VerbResult::ok_with_ops(
        json!({
            "muted": muted,
            "skipped": skipped,
            "word_range": [lo, hi],
            "text": text,
            "range_ms": src,
        }),
        op_ids,
    ))
}

/// transcript.assemble{asset, word_ranges:[[lo,hi]...], track?, at_ms?, pad_ms?,
/// rationale?} — builds a sequence from non-contiguous transcript
/// spans — a highlight reel. Each word_range maps to a padded source span; the
/// spans are placed SEQUENTIALLY in the GIVEN order (reordering allowed), on the
/// `track` video target (default base video), MIRRORED onto `audio_track`
/// (default base audio) when the asset has audio — picture AND sound, paired
/// clips synced under later transcript ripples like auto-place. `audio_track` is
/// caller-controllable so a reel on an overlay video track targets a matched
/// audio track instead of dumping sound into the base narration. Additive at
/// `at_ms` (default = the LATER end of the two targets, so it appends after both
/// without overlap); assemble onto a fresh track pair / cleared timeline for a
/// pure reel. Lowers to one edit.insert op per span per track — skim-reviewable,
/// reuses all insert machinery. All ranges validated up front so a bad index
/// never commits a partial reel.
pub(super) async fn transcript_assemble(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    /// One source's spans (multi-source assemble): stitch the OUTPUT line
    /// from spans across SEVERAL clips/assets — the inverse of "cut from each
    /// source line."
    #[derive(serde::Deserialize)]
    struct Source {
        asset: String,
        word_ranges: Vec<[usize; 2]>,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        // Legacy single-source form.
        asset: Option<String>,
        word_ranges: Option<Vec<[usize; 2]>>,
        // Multi-source form: spans from several assets, concatenated in
        // the GIVEN order onto the output line.
        sources: Option<Vec<Source>>,
        track: Option<String>,       // video target (default base video)
        audio_track: Option<String>, // audio mirror target (default base audio)
        at_ms: Option<u64>,
        pad_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let pad = a.pad_ms.unwrap_or(cut_perception::CUT_ON_WORD_TOLERANCE_MS);

    // Normalize to an ordered source list: multi-source `sources` wins, else the
    // legacy single `asset`+`word_ranges`.
    let sources: Vec<Source> = match a.sources {
        Some(s) if !s.is_empty() => s,
        _ => match (a.asset.clone(), a.word_ranges.clone()) {
            (Some(asset), Some(word_ranges)) if !word_ranges.is_empty() => {
                vec![Source { asset, word_ranges }]
            }
            _ => {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "nothing to assemble",
                    "pass `sources:[{asset, word_ranges}]` (multi-source) or \
                     `asset`+`word_ranges` (single source)",
                ));
            }
        },
    };

    let transcript_ignores = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        store.project.transcript_ignores.clone()
    };

    // Load + validate each source's transcript → ordered (asset, src_span) items.
    let mut items: Vec<(String, [u64; 2])> = Vec::new();
    for s in &sources {
        if s.word_ranges.is_empty() {
            continue;
        }
        let t = load_transcript(state, &s.asset).await?;
        for &[lo, hi] in &s.word_ranges {
            if lo > hi || hi >= t.words.len() {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!(
                        "word_range [{lo},{hi}] out of bounds for asset '{}'",
                        s.asset
                    ),
                    format!(
                        "transcript has {} words (indices 0..={})",
                        t.words.len(),
                        t.words.len().saturating_sub(1)
                    ),
                ));
            }
            for [seg_lo, seg_hi] in
                split_word_range_by_ignores(&transcript_ignores, &s.asset, [lo, hi])
            {
                items.push((
                    s.asset.clone(),
                    [
                        t.words[seg_lo].start_ms.saturating_sub(pad),
                        t.words[seg_hi].end_ms + pad,
                    ],
                ));
            }
        }
    }
    if items.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "no spans to assemble",
            "every source had empty word_ranges",
        ));
    }

    // Resolve target video track + audio mirror + per-asset has_audio + start.
    let (vtrack, atrack, has_audio_map, start) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let vtrack = match &a.track {
            Some(tr) => tr.clone(),
            None => store
                .project
                .tracks
                .iter()
                .find(|t| t.kind == cut_core::TrackKind::Video)
                .map(|t| t.id.clone())
                .ok_or_else(|| {
                    CutError::new(
                        error_codes::NOT_FOUND,
                        "no video track to assemble onto",
                        "create a project first (project.create makes v1 + a1t)",
                    )
                })?,
        };
        // Audio mirror target: explicit arg, else the base (first) audio track.
        // Keeping it CALLER-controllable avoids the mismatched-pair bug where a
        // reel assembled onto an overlay video track would dump its audio into
        // the base narration track.
        let atrack = match &a.audio_track {
            Some(tr) => Some(tr.clone()),
            None => store
                .project
                .tracks
                .iter()
                .find(|t| t.kind == cut_core::TrackKind::Audio)
                .map(|t| t.id.clone()),
        };
        let mut has_audio_map: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        for (asset, _) in &items {
            if !has_audio_map.contains_key(asset) {
                let ha = store
                    .project
                    .assets
                    .get(asset)
                    .and_then(|x| x.probe.as_ref())
                    .and_then(|p| p.get("has_audio"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                has_audio_map.insert(asset.clone(), ha);
            }
        }
        // Default start = the LATER end of the two target tracks, so the reel
        // appends cleanly after BOTH (never overlaps existing content on either).
        let v_end = store
            .project
            .track(&vtrack)
            .map(|t| t.duration_ms())
            .unwrap_or(0);
        let a_end = atrack
            .as_ref()
            .and_then(|id| store.project.track(id))
            .map(|t| t.duration_ms())
            .unwrap_or(0);
        let start = a.at_ms.unwrap_or(v_end.max(a_end));
        (vtrack, atrack, has_audio_map, start)
    };
    let rationale = a
        .rationale
        .unwrap_or_else(|| "transcript.assemble: highlight span".into());
    let mut at = start;
    let mut op_ids: Vec<String> = Vec::new();
    let mut audio_mirrored_any = false;
    for (asset, span) in &items {
        let len = span[1] - span[0];
        let vargs = json!({
            "asset": asset, "track": vtrack, "at_ms": at,
            "src_range_ms": span, "ripple": false, "rationale": rationale,
        });
        let r = commit_core(state, "edit.insert", vargs, actor.clone()).await?;
        if let Some(ids) = r.op_ids {
            op_ids.extend(ids);
        }
        if *has_audio_map.get(asset).unwrap_or(&false) {
            if let Some(at_track) = &atrack {
                let aargs = json!({
                    "asset": asset, "track": at_track, "at_ms": at,
                    "src_range_ms": span, "ripple": false,
                    "rationale": "transcript.assemble: highlight audio",
                });
                let r = commit_core(state, "edit.insert", aargs, actor.clone()).await?;
                if let Some(ids) = r.op_ids {
                    op_ids.extend(ids);
                }
                audio_mirrored_any = true;
            }
        }
        at += len;
    }
    Ok(VerbResult::ok_with_ops(
        json!({
            // `asset` kept for back-compat (first source); `sources` is the count.
            "asset": items.first().map(|(a, _)| a.clone()),
            "sources": sources.len(),
            "spans_placed": items.len(),
            "track": vtrack,
            "audio_mirrored": audio_mirrored_any,
            "at_ms": start,
            "total_ms": at - start,
        }),
        op_ids,
    ))
}

/// Aggressiveness presets (the required-argument contract: REQUIRED, named-preset discipline).
/// Returns (min_silence_ms, keep_padding_ms).
fn silence_preset(name: &str) -> Result<(u64, u64), CutError> {
    match name {
        "calm" => Ok((1200, 200)),
        "natural" => Ok((700, 120)),
        "jumpy" => Ok((350, 60)),
        other => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown aggressiveness '{other}'"),
            "must be one of calm|natural|jumpy",
        )),
    }
}

/// Validate scope-narrowing args against the open project: a named
/// asset/track that does not exist is an actionable not_found, never a
/// silent "nothing matched" (the agent typo'd an id, not an empty timeline).
pub(super) fn check_scope(
    project: &cut_core::Project,
    asset: Option<&str>,
    track: Option<&str>,
) -> Result<(), CutError> {
    if let Some(aid) = asset {
        if !project.assets.contains_key(aid) {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("no asset '{aid}' to narrow to"),
                "the asset arg must be an existing asset id",
            )
            .with_suggested_action("project.state lists asset ids"));
        }
    }
    if let Some(tid) = track {
        if project.track(tid).is_none() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("no track '{tid}' to narrow to"),
                "the track arg must be an existing track id",
            )
            .with_suggested_action("project.state lists track ids"));
        }
    }
    Ok(())
}

/// transcript.remove_silences{aggressiveness, min_ms?, padding_ms?, asset?,
/// track?} — timeline-wide by default (the scope contract); `asset`/`track` narrow
/// DETECTION scope (which silence spans qualify); the cut itself still
/// ripples ALL tracks so AV stays in sync (same semantics as `asset`
/// narrowing — a single-track ripple would silently desync everything after
/// the cut; use edit.ripple_delete{track} for deliberate one-track surgery).
/// Silence facts come from each asset's perception.json.
pub(super) async fn transcript_remove_silences(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        aggressiveness: String,
        min_ms: Option<u64>,
        padding_ms: Option<u64>,
        asset: Option<String>,
        track: Option<String>,
        #[serde(default)]
        allow_extreme: bool,
    }
    let a: Args = parse_args(args.clone())?;
    let (preset_min, preset_pad) = silence_preset(&a.aggressiveness)?;
    let (min_ms, pad_ms) = (
        a.min_ms.unwrap_or(preset_min),
        a.padding_ms.unwrap_or(preset_pad),
    );
    // Collect timeline ranges from every (or the named) asset's silences,
    // walking only the named track's segments when narrowed. Assets WITHOUT
    // silence facts (no perception report, or a report whose instruments_run
    // lacks "silence" — e.g. a words-only run, or a legacy failed-chain
    // shape) are collected so a zero-span result can be told apart from
    // "facts exist and genuinely show no silences" (missing-silence-facts guard: the silent
    // ok/note no-op was an honesty violation).
    let mut missing_facts: Vec<String> = Vec::new();
    let (ranges, timeline_ms) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        check_scope(&store.project, a.asset.as_deref(), a.track.as_deref())?;
        let receipts = store.receipts_dir();
        // Assets actually PLACED in the narrowed scope: an imported-but-
        // unused asset (or one only on other tracks under `track` narrowing)
        // can't contribute spans regardless of facts, so it must neither
        // error nor warn as "missing facts".
        let edl = cut_core::edl_from_project(&store.project);
        let placed: std::collections::BTreeSet<&str> = edl
            .segments
            .iter()
            .filter(|s| a.track.as_deref().is_none_or(|t| t == s.track))
            .filter_map(|s| s.asset.as_deref())
            .collect();
        let mut out = Vec::new();
        for (id, asset) in store.project.assets.iter() {
            if a.asset.as_ref().map(|x| x != id).unwrap_or(false) {
                continue;
            }
            if !placed.contains(id.as_str()) {
                continue;
            }
            // Still images carry no audio — they can't have silence facts
            // and must not trigger the missing-facts error.
            let kind = asset
                .probe
                .as_ref()
                .and_then(|p| p.get("kind"))
                .and_then(|k| k.as_str())
                .unwrap_or("video");
            if kind == "image" {
                continue;
            }
            let report = cut_perception::load_report(&receipts, id)?;
            // instruments_run is the provenance; legacy reports (pre-field)
            // with actual spans recorded still count as facts.
            let has_silence_facts = report.as_ref().is_some_and(|r| {
                r.instruments_run.iter().any(|i| i == "silence") || !r.silences.is_empty()
            });
            if !has_silence_facts {
                missing_facts.push(id.clone());
                continue;
            }
            let report = report.expect("checked above");
            // Word spans (same Full report) let us keep silence cuts OUT of
            // spoken words. A plosive closure — the brief gap before "scrip[t]"
            // — reads as silence to silero/ffmpeg while the ASR still times the
            // burst as speech; cutting at the raw (padded) silence edge then
            // clips the word tail. Pull each boundary out to the
            // word edge so remove_silences never truncates a word.
            let words: &[cut_perception::WordSpan] = report
                .words
                .as_ref()
                .map(|t| t.words.as_slice())
                .unwrap_or(&[]);
            let tol = cut_perception::CUT_ON_WORD_TOLERANCE_MS;
            for s in &report.silences {
                let len = s.end_ms.saturating_sub(s.start_ms);
                if len < min_ms || len <= 2 * pad_ms {
                    continue; // too short to cut once padding is kept
                }
                // Keep pad_ms of breathing room on each side of the cut.
                let mut lo = s.start_ms + pad_ms;
                let mut hi = s.end_ms.saturating_sub(pad_ms);
                // Word-aware clamp: a boundary strictly inside a word (mirrors the
                // cut_on_word tolerance) is pulled to that word's far edge so no
                // spoken audio is cut. Words are non-overlapping → one pass.
                for w in words {
                    if lo > w.start_ms + tol && lo < w.end_ms.saturating_sub(tol) {
                        lo = w.end_ms; // start of cut moves PAST the word it hit
                    }
                    if hi > w.start_ms + tol && hi < w.end_ms.saturating_sub(tol) {
                        hi = w.start_ms; // end of cut moves BEFORE the word it hit
                    }
                }
                // Word-boundary ALIGNMENT for the trailing edge: if a word begins
                // inside the remaining trailing silence (hi, se] — i.e. the silence
                // detector over-extended into the word's onset, common around the
                // plosive/soft start of the next word — cut right up to that word's
                // start. Otherwise the few-ms fragment between hi and a later
                // word-aligned cut (remove_fillers/cut_words) becomes a SUB-FRAME
                // sliver clip, which the frame-based exporter rejects by design.
                // Aligning every transcript cut to a word edge (edges are ≥1 frame
                // apart) makes such slivers impossible. Parakeet's precise timing
                // surfaced this.
                if let Some(ws) = words
                    .iter()
                    .map(|w| w.start_ms)
                    .filter(|&st| st >= hi && st <= s.end_ms)
                    .min()
                {
                    hi = ws;
                }
                if hi <= lo {
                    continue; // word clamping absorbed the whole gap — nothing to cut
                }
                let src = [lo, hi];
                out.extend(
                    source_to_timeline(&store.project, id, src, a.track.as_deref())
                        .into_iter()
                        .map(|r| (r, vec![])),
                );
            }
        }
        (out, store.project.duration_ms())
    };
    // missing-silence-facts guard: zero spans with missing facts is NOT "your audio has no
    // silences" — it's "nothing was measured". Error, actionably, instead of
    // the indistinguishable {ok, spans_removed:0, note:"nothing matched"}.
    if ranges.is_empty() && !missing_facts.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("no silence facts for asset(s) {}", missing_facts.join(", ")),
            "perception has not produced silence facts for these assets (never run, \
             still running, or a failed/partial chain — e.g. a words-only report); \
             a zero-span result would be indistinguishable from 'genuinely no silences'",
        )
        .with_suggested_action(
            "run media.perception{asset} and wait for the job (jobs.status), then retry; \
             audio-only assets get silence facts from the audio battery; \
             check media.import chain job state if the import just happened",
        ));
    }
    // Totality guard (the totality guard, HIGH): on fully-silent footage this
    // verb happily ripple-deleted 99.4% of the timeline. Refuse when the pass
    // would remove >80% of the current composition unless the caller says
    // allow_extreme:true explicitly. Overlap-merged sum so stacked per-asset
    // spans can't double-count.
    const TOTALITY_GUARD_PCT: u64 = 80;
    if !a.allow_extreme && timeline_ms > 0 {
        let mut sorted: Vec<[u64; 2]> = ranges.iter().map(|(r, _)| *r).collect();
        sorted.sort();
        let mut removed: u64 = 0;
        let mut cursor: u64 = 0; // end of the merged region so far
        for [s, e] in sorted {
            let s = s.max(cursor);
            if e > s {
                removed += e - s;
                cursor = e;
            }
        }
        if removed * 100 > timeline_ms * TOTALITY_GUARD_PCT {
            let pct = (removed * 100) / timeline_ms;
            return Err(CutError::new(
                error_codes::GUARDRAIL,
                format!("refusing to remove {pct}% of the timeline as silence"),
                format!(
                    "this pass would ripple-delete {removed} of {timeline_ms} ms (> {TOTALITY_GUARD_PCT}% guard); \
                     near-total removal usually means the footage is silent by design (e.g. a screen recording), \
                     not that the cut is wanted"
                ),
            )
            .with_suggested_action(
                "if this is intentional, retry with allow_extreme:true (edit.restore / project.revert undo per span); \
                 otherwise narrow with asset/track, raise min_ms, or skip silence removal on silent-by-design footage",
            ));
        }
    }
    // Spans found but SOME in-scope assets had no facts: proceed (the cuts
    // are real) with an in-band warning naming the unmeasured assets.
    let mut res = ripple_ranges(
        state,
        "transcript.remove_silences",
        &args,
        actor,
        ranges,
        "spans_removed",
    )
    .await?;
    if !missing_facts.is_empty() {
        res = res.with_warnings(vec![cut_core::VerbWarning {
            code: "missing_silence_facts".into(),
            message: format!(
                "asset(s) {} have no silence facts (perception incomplete) — their silences \
                 were NOT considered; run media.perception and re-run to cover them",
                missing_facts.join(", ")
            ),
            detail: Default::default(),
        }]);
    }
    Ok(res)
}

/// Default filler lexicon (lowercase, punctuation-stripped match).
pub(super) const FILLERS: &[&str] = &["um", "uh", "erm", "ah", "hmm", "mhm"];

fn normalize_filler_token(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// transcript.remove_fillers{lexicon?, asset?, track?} — cut consecutive
/// filler-word runs, one op per run, ±40ms padded to word edges. `asset`/
/// `track` narrow DETECTION scope per the scope contract; the cut still ripples ALL
/// tracks (AV sync — same semantics as remove_silences narrowing).
pub(super) async fn transcript_remove_fillers(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        lexicon: Option<Vec<String>>,
        asset: Option<String>,
        track: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let lexicon: Vec<String> = a
        .lexicon
        .unwrap_or_else(|| FILLERS.iter().map(|s| s.to_string()).collect())
        .iter()
        .map(|w| normalize_filler_token(w))
        .filter(|w| !w.is_empty())
        .collect();
    let asset_ids: Vec<String> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        check_scope(&store.project, a.asset.as_deref(), a.track.as_deref())?;
        store
            .project
            .assets
            .keys()
            .filter(|id| a.asset.as_ref().map(|x| &x == id).unwrap_or(true))
            .cloned()
            .collect()
    };
    let mut ranges = Vec::new();
    let pad = cut_perception::CUT_ON_WORD_TOLERANCE_MS;
    for id in &asset_ids {
        let Ok(t) = load_transcript(state, id).await else {
            continue;
        };
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        // Group consecutive filler words into runs → one source span per run.
        let is_filler = |w: &str| {
            let clean = normalize_filler_token(w);
            lexicon.contains(&clean)
        };
        // Run = (start_ms, end_ms, first_word_idx, last_word_idx). The word
        // indices are recorded as an {asset, word_range} effect on each op so
        // the transcript panel strikes the exact filler words (shared.ts
        // activeCutSpans' effects path for struck spans).
        let mut run: Option<(u64, u64, usize, usize)> = None;
        let flush = |run: (u64, u64, usize, usize), ranges: &mut Vec<([u64; 2], Vec<OpEffect>)>| {
            let (s, e, wi, wj) = run;
            ranges.extend(
                source_to_timeline(
                    &store.project,
                    id,
                    [s.saturating_sub(pad), e + pad],
                    a.track.as_deref(),
                )
                .into_iter()
                .map(|r| {
                    (
                        r,
                        vec![effect(None, json!({"asset": id, "word_range": [wi, wj]}))],
                    )
                }),
            );
        };
        for (idx, w) in t.words.iter().enumerate() {
            if is_filler(&w.word) {
                run = Some(match run {
                    None => (w.start_ms, w.end_ms, idx, idx),
                    Some((s, _, wi, _)) => (s, w.end_ms, wi, idx),
                });
            } else if let Some(r) = run.take() {
                flush(r, &mut ranges);
            }
        }
        if let Some(r) = run {
            flush(r, &mut ranges);
        }
    }
    ripple_ranges(
        state,
        "transcript.remove_fillers",
        &args,
        actor,
        ranges,
        "fillers_removed",
    )
    .await
}

// ---------------------------------------------------------------------------
// transcript.remove_retakes — auto-cut repeated line ATTEMPTS (retakes)
// ---------------------------------------------------------------------------
//
// A common talking-head editing chore: a presenter flubs a line, pauses, and
// says it again ("…convert sunlight into— let me redo that. …convert sunlight
// into energy."). The good take is usually the LAST attempt; the earlier
// attempts are dead weight. This verb segments the transcript into utterances,
// finds runs of consecutive near-identical utterances (the retakes), keeps one
// per run, and ripple-cuts the rest — reusing the exact cut machinery of
// transcript.remove_fillers (source_to_timeline → ripple_ranges).
//
// The detection core (segment + similarity + keep policy) is PURE and unit-
// tested below; the handler is only scope + cut + receipt.

/// Which take to KEEP from a run of similar attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetakeKeep {
    /// Keep the FIRST attempt (the cleanest start, before drift).
    First,
    /// Keep the LAST attempt (default — the good final take after warming up).
    Last,
    /// Keep the LONGEST attempt by content-token count (ties → earliest).
    Longest,
}

impl RetakeKeep {
    /// Parse the `keep` arg; `None` ⇒ Last (the default good-final-take policy).
    fn parse(s: Option<&str>) -> Result<Self, CutError> {
        match s.map(str::to_lowercase).as_deref() {
            None | Some("last") => Ok(RetakeKeep::Last),
            Some("first") => Ok(RetakeKeep::First),
            Some("longest") => Ok(RetakeKeep::Longest),
            Some(other) => Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown keep policy '{other}'"),
                "keep must be one of: \"last\" (default), \"first\", \"longest\"",
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            RetakeKeep::First => "first",
            RetakeKeep::Last => "last",
            RetakeKeep::Longest => "longest",
        }
    }
}

/// Lowercased alphanumeric CONTENT tokens of an inclusive word-slice range.
/// Surrounding punctuation is stripped and empty tokens dropped, so "Take." →
/// "take" and a bare "—" → nothing. Order is preserved (the similarity metric
/// is sequence-aware).
fn retake_tokens(words: &[cut_perception::WordSpan], lo: usize, hi: usize) -> Vec<String> {
    words[lo..=hi]
        .iter()
        .filter_map(|w| {
            let t: String = w
                .word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
        .collect()
}

/// Levenshtein edit distance between two token SEQUENCES (Wagner–Fischer, O(a·b)
/// time, O(b) space). Sequence-aware so a partial restart ("the key thing is—" →
/// "the key thing is that…") scores as a near-match (a few insertions), unlike a
/// bag-of-words Jaccard which would also reward an unrelated reshuffle.
fn token_levenshtein(a: &[String], b: &[String]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ta) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, tb) in b.iter().enumerate() {
            let cost = usize::from(ta != tb);
            cur[j + 1] = (prev[j + 1] + 1) // deletion
                .min(cur[j] + 1) // insertion
                .min(prev[j] + cost); // substitution/match
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Normalized token-sequence similarity in [0,1]: `1 − levenshtein / max(len)`.
/// 1.0 = identical token sequences, 0.0 = no shared structure. Two empty
/// sequences are trivially identical (1.0). This is the retake-detection metric.
fn retake_similarity(a: &[String], b: &[String]) -> f64 {
    let maxlen = a.len().max(b.len());
    if maxlen == 0 {
        return 1.0;
    }
    1.0 - token_levenshtein(a, b) as f64 / maxlen as f64
}

/// Split a word transcript into UTTERANCES — inclusive `[lo, hi]` slice-index
/// ranges. A boundary falls after a word that ends a sentence (`.?!`) OR before
/// a word that follows a pause gap > `pause_ms` (the typical "…flubbed line.
/// <pause> let me say that again…" retake seam). PURE + deterministic.
pub(super) fn retake_utterances(
    words: &[cut_perception::WordSpan],
    pause_ms: u64,
) -> Vec<[usize; 2]> {
    let mut out = Vec::new();
    if words.is_empty() {
        return out;
    }
    let mut lo = 0usize;
    for i in 0..words.len() {
        let ends_sentence = words[i]
            .word
            .trim_end()
            .chars()
            .last()
            .is_some_and(|c| matches!(c, '.' | '?' | '!'));
        let is_last = i + 1 == words.len();
        let pause_after =
            !is_last && words[i + 1].start_ms.saturating_sub(words[i].end_ms) > pause_ms;
        if ends_sentence || pause_after || is_last {
            out.push([lo, i]);
            lo = i + 1;
        }
    }
    out
}

/// One detected retake cluster: the kept take + the removed attempts, all as
/// inclusive word-slice index ranges into the source `words`.
#[derive(Debug, Clone, PartialEq)]
struct RetakeCluster {
    /// The take that survives (per the keep policy).
    kept: [usize; 2],
    /// The earlier/other attempts to ripple-cut, ascending by start index.
    removed: Vec<[usize; 2]>,
}

/// The retake-detection CORE (PURE, deterministic, no model). Segments the
/// words into utterances, chains consecutive ELIGIBLE utterances (≥ `min_words`
/// content tokens) whose adjacent similarity ≥ `similarity` into runs, and for
/// every run of ≥2 attempts applies the `keep` policy to decide which take
/// survives. Short utterances (< `min_words` tokens — "ok", "no wait") never
/// count as retakes and are skipped when chaining, so a brief aside between two
/// attempts doesn't break the run. Returns one `RetakeCluster` per detected run.
fn detect_retakes(
    words: &[cut_perception::WordSpan],
    pause_ms: u64,
    similarity: f64,
    keep: RetakeKeep,
    min_words: usize,
) -> Vec<RetakeCluster> {
    // (slice range, content tokens) for utterances long enough to be a "line".
    let eligible: Vec<([usize; 2], Vec<String>)> = retake_utterances(words, pause_ms)
        .into_iter()
        .map(|r| {
            let toks = retake_tokens(words, r[0], r[1]);
            (r, toks)
        })
        .filter(|(_, toks)| toks.len() >= min_words)
        .collect();

    // Chain adjacent eligible utterances with similarity ≥ threshold into runs.
    // A run of ≥2 utterances is a retake cluster (the same line attempted again).
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for k in 0..eligible.len() {
        if let Some(&prev) = cur.last() {
            if retake_similarity(&eligible[prev].1, &eligible[k].1) >= similarity {
                cur.push(k);
                continue;
            }
            if cur.len() >= 2 {
                runs.push(cur.clone());
            }
            cur.clear();
        }
        cur.push(k);
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }

    runs.into_iter()
        .map(|members| {
            // Position WITHIN the run of the take to keep.
            let keep_pos = match keep {
                RetakeKeep::First => 0,
                RetakeKeep::Last => members.len() - 1,
                RetakeKeep::Longest => {
                    let mut best = 0usize;
                    let mut best_len = eligible[members[0]].1.len();
                    for (pos, &m) in members.iter().enumerate().skip(1) {
                        let len = eligible[m].1.len();
                        if len > best_len {
                            best = pos;
                            best_len = len;
                        }
                    }
                    best
                }
            };
            let kept = eligible[members[keep_pos]].0;
            let mut removed: Vec<[usize; 2]> = members
                .iter()
                .enumerate()
                .filter(|(pos, _)| *pos != keep_pos)
                .map(|(_, &m)| eligible[m].0)
                .collect();
            removed.sort_unstable();
            RetakeCluster { kept, removed }
        })
        .collect()
}

/// transcript.remove_retakes{asset?, track?, similarity?, pause_ms?, keep?,
/// min_words?} — auto-remove repeated line ATTEMPTS, keeping the best take.
/// Mirrors transcript.remove_fillers' scope + cut path (source_to_timeline →
/// ripple_ranges, the cut ripples all tracks for AV sync); the only new part is
/// retake DETECTION (see detect_retakes). Honest degradation: no transcribed
/// asset in scope → the same actionable "transcribe first" error remove_fillers'
/// loader raises; no retakes found → ok with removed_takes:0 (a clean no-op).
pub(super) async fn transcript_remove_retakes(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        asset: Option<String>,
        track: Option<String>,
        /// Min token-sequence similarity (0..1) for two utterances to be the
        /// same line; default 0.6.
        similarity: Option<f64>,
        /// Pause gap (ms) that ends an utterance; default 600.
        pause_ms: Option<u64>,
        /// "last" (default) | "first" | "longest".
        keep: Option<String>,
        /// Utterances with fewer content tokens are ignored (never a retake);
        /// default 3.
        min_words: Option<usize>,
    }
    let a: Args = parse_args(args.clone())?;
    let similarity = a.similarity.unwrap_or(0.6).clamp(0.0, 1.0);
    let pause_ms = a.pause_ms.unwrap_or(600);
    let min_words = a.min_words.unwrap_or(3).max(1);
    let keep = RetakeKeep::parse(a.keep.as_deref())?;

    let asset_ids: Vec<String> = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        check_scope(&store.project, a.asset.as_deref(), a.track.as_deref())?;
        store
            .project
            .assets
            .keys()
            .filter(|id| a.asset.as_ref().map(|x| &x == id).unwrap_or(true))
            .cloned()
            .collect()
    };

    let pad = cut_perception::CUT_ON_WORD_TOLERANCE_MS;
    let mut ranges: Vec<([u64; 2], Vec<OpEffect>)> = Vec::new();
    let mut clusters_out: Vec<Value> = Vec::new();
    let mut removed_takes = 0usize;
    let mut kept_takes = 0usize;
    let mut words_removed = 0usize;
    let mut any_transcript = false;

    for id in &asset_ids {
        let Ok(t) = load_transcript(state, id).await else {
            continue;
        };
        any_transcript = true;
        let clusters = detect_retakes(&t.words, pause_ms, similarity, keep, min_words);
        if clusters.is_empty() {
            continue;
        }
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        // Render one removed-take span → timeline ranges (source_to_timeline,
        // exactly like remove_fillers) and a JSON detail object for the receipt.
        let take_detail = |t: &cut_perception::Transcript, r: [usize; 2]| -> Value {
            json!({
                "word_range": [t.words[r[0]].idx, t.words[r[1]].idx],
                "at_ms": [t.words[r[0]].start_ms, t.words[r[1]].end_ms],
                "text": t.words[r[0]..=r[1]]
                    .iter()
                    .map(|w| w.word.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            })
        };
        for cluster in &clusters {
            let mut removed_detail = Vec::new();
            for &rem in &cluster.removed {
                let s = t.words[rem[0]].start_ms;
                let e = t.words[rem[1]].end_ms;
                // Only count a take as removed when it actually maps onto the
                // timeline — detection runs on the SOURCE transcript, so a take
                // already cut (or never placed) maps to nothing and is a no-op,
                // keeping the verb idempotent + the receipt honest (mirrors
                // remove_fillers, whose count is the timeline spans it cut).
                let mapped = source_to_timeline(
                    &store.project,
                    id,
                    [s.saturating_sub(pad), e + pad],
                    a.track.as_deref(),
                );
                if mapped.is_empty() {
                    continue;
                }
                removed_takes += 1;
                words_removed += rem[1] - rem[0] + 1;
                ranges.extend(mapped.into_iter().map(|r| {
                    (
                        r,
                        vec![effect(
                            None,
                            json!({"asset": id, "word_range": [t.words[rem[0]].idx, t.words[rem[1]].idx]}),
                        )],
                    )
                }));
                removed_detail.push(take_detail(&t, rem));
            }
            // A cluster counts as a kept take only when it actually cut ≥1 of its
            // earlier attempts (otherwise nothing changed for this line).
            if !removed_detail.is_empty() {
                kept_takes += 1;
                clusters_out.push(json!({
                    "asset": id,
                    "kept": take_detail(&t, cluster.kept),
                    "removed": removed_detail,
                }));
            }
        }
    }

    // No transcript anywhere in scope → the actionable "transcribe first" error.
    // A named-but-untranscribed asset gets the loader's precise per-asset error.
    if !any_transcript {
        if let Some(id) = &a.asset {
            load_transcript(state, id).await?;
        }
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "no transcribed asset in scope",
            "transcribe the footage first, then re-run",
        )
        .with_suggested_action("call media.transcribe{asset} and wait for the job to finish"));
    }

    // Reuse the filler/silence cut machinery for the actual ripple-deletes, then
    // enrich its envelope with the honest retake receipt.
    let cut = ripple_ranges(
        state,
        "transcript.remove_retakes",
        &args,
        actor,
        ranges,
        "spans_cut",
    )
    .await?;
    let ms_removed = cut
        .result
        .as_ref()
        .and_then(|r| r.get("total_removed_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let op_ids = cut.op_ids.unwrap_or_default();

    let mut receipt = Map::new();
    receipt.insert("removed_takes".into(), json!(removed_takes));
    receipt.insert("kept".into(), json!(kept_takes));
    receipt.insert("keep_policy".into(), json!(keep.as_str()));
    receipt.insert("words_removed".into(), json!(words_removed));
    receipt.insert("ms_removed".into(), json!(ms_removed));
    receipt.insert("clusters".into(), json!(clusters_out));
    if removed_takes == 0 {
        receipt.insert(
            "note".into(),
            json!("no repeated line attempts detected (clean no-op)"),
        );
    }
    Ok(VerbResult::ok_with_ops(Value::Object(receipt), op_ids))
}

#[cfg(test)]
mod retakes_tests {
    use super::*;
    /// Build a word with idx == position and a fixed 400ms duration.
    fn rw(idx: usize, word: &str, start_ms: u64) -> cut_perception::WordSpan {
        cut_perception::WordSpan {
            idx,
            word: word.into(),
            start_ms,
            end_ms: start_ms + 400,
            confidence: None,
            speaker: None,
        }
    }

    /// Lay out space-separated phrases as utterances separated by a >1s pause,
    /// 500ms apart within an utterance (no in-utterance pause boundary).
    fn lay_out(phrases: &[&str]) -> Vec<cut_perception::WordSpan> {
        let mut words = Vec::new();
        let mut t = 0u64;
        let mut idx = 0usize;
        for (p, phrase) in phrases.iter().enumerate() {
            if p > 0 {
                t += 2000; // > default pause_ms (600) → utterance boundary
            }
            for w in phrase.split_whitespace() {
                words.push(rw(idx, w, t));
                idx += 1;
                t += 500;
            }
        }
        words
    }

    /// The core proof: two attempts at the same line ("…take one" / "…take two")
    /// followed by an unrelated sentence → exactly one retake cluster; with the
    /// default keep="last" the FIRST attempt's word-range is removed and the
    /// second is kept; the different sentence is untouched.
    #[test]
    fn detects_retake_keeps_last_attempt() {
        let words = lay_out(&[
            "hello world this is take one",
            "hello world this is take two",
            "completely different sentence here",
        ]);
        let clusters = detect_retakes(&words, 600, 0.6, RetakeKeep::Last, 3);
        assert_eq!(clusters.len(), 1, "exactly one retake cluster");
        // First "hello world…" is words 0..=5, second is 6..=11.
        assert_eq!(clusters[0].kept, [6, 11], "second attempt kept");
        assert_eq!(
            clusters[0].removed,
            vec![[0, 5]],
            "first attempt selected for removal"
        );
    }

    /// A transcript with no repeated lines yields zero clusters (clean no-op).
    #[test]
    fn no_retakes_yields_zero() {
        let words = lay_out(&[
            "the weather today is sunny",
            "i went to the market",
            "rockets launch into deep orbit",
        ]);
        let clusters = detect_retakes(&words, 600, 0.6, RetakeKeep::Last, 3);
        assert!(clusters.is_empty(), "no similar lines ⇒ no retakes");
    }

    /// keep="first"/"last"/"longest" each select the documented take. The two
    /// attempts differ in length so "longest" is distinct from "first"/"last".
    #[test]
    fn keep_policies_select_correctly() {
        // Attempt A (7 tokens) is longer than attempt B (6 tokens); similar.
        let words = lay_out(&[
            "let me introduce the topic today carefully",
            "let me introduce the topic today",
        ]);
        let first = detect_retakes(&words, 600, 0.6, RetakeKeep::First, 3);
        let last = detect_retakes(&words, 600, 0.6, RetakeKeep::Last, 3);
        let longest = detect_retakes(&words, 600, 0.6, RetakeKeep::Longest, 3);
        assert_eq!(first.len(), 1);
        // A = words 0..=6, B = words 7..=12.
        assert_eq!(first[0].kept, [0, 6], "keep=first ⇒ first attempt");
        assert_eq!(first[0].removed, vec![[7, 12]]);
        assert_eq!(last[0].kept, [7, 12], "keep=last ⇒ last attempt");
        assert_eq!(last[0].removed, vec![[0, 6]]);
        assert_eq!(
            longest[0].kept,
            [0, 6],
            "keep=longest ⇒ the 7-token attempt"
        );
        assert_eq!(longest[0].removed, vec![[7, 12]]);
    }

    /// Three consecutive attempts at one line chain into a single cluster; with
    /// keep="last" the two earlier attempts are both removed (the real "third
    /// time's the charm" case).
    #[test]
    fn three_attempts_chain_into_one_cluster() {
        let words = lay_out(&[
            "so photosynthesis converts sunlight into sugar",
            "so photosynthesis converts sunlight into energy",
            "so photosynthesis converts sunlight into glucose",
        ]);
        let clusters = detect_retakes(&words, 600, 0.6, RetakeKeep::Last, 3);
        assert_eq!(clusters.len(), 1, "all three attempts in one cluster");
        assert_eq!(clusters[0].kept, [12, 17], "third attempt kept");
        assert_eq!(
            clusters[0].removed,
            vec![[0, 5], [6, 11]],
            "first two attempts removed"
        );
    }

    /// A short aside ("no wait") between two attempts must NOT break the run:
    /// the 2-token aside is below min_words=3 so it's skipped when chaining.
    #[test]
    fn short_aside_does_not_break_run() {
        let words = lay_out(&[
            "welcome back to the channel everyone",
            "no wait",
            "welcome back to the channel folks",
        ]);
        let clusters = detect_retakes(&words, 600, 0.6, RetakeKeep::Last, 3);
        assert_eq!(clusters.len(), 1, "aside skipped, two attempts still chain");
        // Attempt 1 = 0..=5, aside = 6..=7 (ignored), attempt 2 = 8..=13.
        assert_eq!(clusters[0].kept, [8, 13]);
        assert_eq!(clusters[0].removed, vec![[0, 5]]);
    }
}

/// transcript.translate{asset?, target_lang, source_lang?, backend?, model?,
/// timeout_ms?, rationale?} — translate an asset's word-level transcript into
/// `target_lang`, writing a SIBLING transcript artifact
/// `receipts/<asset>.<lang>.words.json` (the source transcript is untouched).
/// The translator works at SENTENCE-ISH SEGMENT grain (better LLM/MT quality
/// than per-word); segment-level timing is exact, and within each segment the
/// translated tokens are LINEARLY INTERPOLATED across the span (HONEST LIMIT:
/// true per-word timing would need forced alignment on the translated audio,
/// out of scope). Read-only w.r.t. the op-log (writes to receipts/, the evidence
/// store, like media.transcribe) — feeds export/search/captions in the target
/// language. `asset` is optional when exactly one asset has a transcript.
pub(super) async fn transcript_translate(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        asset: Option<String>,
        target_lang: String,
        source_lang: Option<String>,
        backend: Option<String>,
        model: Option<String>,
        timeout_ms: Option<u64>,
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let target_lang = crate::translate::normalize_lang(&a.target_lang);
    if target_lang.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "target_lang is empty",
            "pass the language to translate INTO, e.g. \"es\"",
        ));
    }
    validate_path_component_arg("target_lang", &target_lang)?;

    // Resolve the asset: explicit, else the unique transcribed asset.
    let (asset_id, dir): (String, PathBuf) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let id = if let Some(id) = a.asset.clone() {
            id
        } else {
            let with_t: Vec<&String> = store
                .project
                .assets
                .iter()
                .filter(|(_, v)| v.transcript.is_some())
                .map(|(k, _)| k)
                .collect();
            match with_t.as_slice() {
                [one] => (*one).clone(),
                [] => {
                    return Err(CutError::new(
                        error_codes::NOT_FOUND,
                        "no transcribed asset in the project",
                        "media.transcribe an asset first",
                    ));
                }
                many => {
                    return Err(CutError::new(
                        error_codes::INVALID_ARGS,
                        "several assets have transcripts — name one",
                        format!("pass asset; candidates: {many:?}"),
                    ));
                }
            }
        };
        (id, store.dir.clone())
    };

    // Load the source transcript.
    let src = load_transcript(state, &asset_id).await?;
    if src.words.is_empty() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("asset '{asset_id}' transcript has no words"),
            "nothing to translate (silent footage / 0-word transcript)",
        ));
    }
    let source_lang = a.source_lang.clone().or_else(|| src.language.clone());

    // Group words → sentence-ish segments (the translation unit).
    let words_tuples: Vec<(u64, u64, String)> = src
        .words
        .iter()
        .map(|w| (w.start_ms, w.end_ms, w.word.clone()))
        .collect();
    let segments = crate::translate::group_words_into_segments(&words_tuples, 600, 200);
    let seg_texts: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();

    // Translate (CLI primary; local only when no CLI is available).
    let outcome = crate::translate::run_translation(
        a.backend.as_deref(),
        source_lang.as_deref(),
        &target_lang,
        &seg_texts,
        a.model.as_deref(),
        a.timeout_ms,
    )
    .await?;

    // Rebuild a translated Transcript: distribute each segment's translation
    // across its [start,end] span as WordSpans (segment timing exact; per-word
    // interpolated). idx is re-numbered 0..N.
    let mut out_words: Vec<cut_perception::WordSpan> = Vec::new();
    for (seg, tr) in segments.iter().zip(outcome.translations.iter()) {
        for (s, e, tok) in crate::translate::distribute_tokens(seg.start_ms, seg.end_ms, tr) {
            out_words.push(cut_perception::WordSpan {
                idx: out_words.len(),
                word: tok,
                start_ms: s,
                end_ms: e,
                confidence: None,
                speaker: None,
            });
        }
    }
    let translated = cut_perception::Transcript {
        asset: asset_id.clone(),
        model: format!("translate:{}:{}", outcome.backend, outcome.model),
        language: Some(target_lang.clone()),
        words: out_words,
    };

    // Persist the sibling artifact (does NOT overwrite the source transcript).
    let rel = format!("receipts/{asset_id}.{lang}.words.json", lang = target_lang);
    let path = dir.join(&rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CutError::new(error_codes::IO, "create receipts dir", e.to_string()))?;
    }
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&translated)
            .map_err(|e| CutError::new(error_codes::IO, "serialize transcript", e.to_string()))?,
    )
    .map_err(|e| {
        CutError::new(
            error_codes::IO,
            "write translated transcript",
            e.to_string(),
        )
    })?;

    Ok(VerbResult::ok(json!({
        "asset": asset_id,
        "source_lang": source_lang,
        "target_lang": target_lang,
        "backend": outcome.backend,
        "backend_proven": outcome.proven,
        "model": outcome.model,
        "agent": outcome.agent,
        "translation_warnings": outcome.warnings.clone(),
        "segments_translated": segments.len(),
        "words": translated.words.len(),
        "transcript": rel,
        "word_timing": "interpolated",
        "note": "segment-level timing is exact; per-word timing within a segment is linearly interpolated (no forced alignment on the translated text)",
    }))
    .with_warnings(translation_warnings_to_verb(&outcome.warnings)))
}

/// Dedicated caption track for timed text cards (captions.add_text). Kept
/// separate from "cap1" so captions.generate's replace-all regeneration can
/// never wipe title cards.
pub(super) const TEXT_TRACK_ID: &str = "txt1";

/// captions.add_text{text, range_ms, style_ref?, position?} — place a timed
/// text clip (title/intro/outro card) on the dedicated "txt1" caption track,
/// burned in by the existing caption pipeline. HONEST SCOPE: this
/// is a styled static text clip — full motion-graphics titles (animation,
/// per-word styling, fades) are a later feature. `position` without a
/// style_ref auto-creates a built-in title style `txt_<position>` (Inter 64,
/// white on translucent black); neither arg → the renderer's Default
/// bottom-positioned caption style.
pub(super) async fn captions_add_text(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        text: String,
        range_ms: [u64; 2],
        style_ref: Option<String>,
        position: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    if a.text.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "text must not be empty",
            "an empty caption clip would render nothing",
        ));
    }
    if a.range_ms[0] >= a.range_ms[1] {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "range_ms [{}, {}) is empty or inverted",
                a.range_ms[0], a.range_ms[1]
            ),
            "range start must be strictly less than range end",
        ));
    }
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    // Resolve the style: explicit ref must exist; position synthesizes a
    // built-in; both together is ambiguous.
    let mut styles = store.project.caption_styles.clone();
    let style_ref = match (&a.style_ref, &a.position) {
        (Some(_), Some(_)) => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "pass either style_ref or position, not both",
                "a style already carries its position (pos field)",
            ));
        }
        (Some(r), None) => {
            if !styles.contains_key(r) {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no caption style '{r}'"),
                    "style refs come from captions.set_style / project.state caption_styles",
                )
                .with_suggested_action(
                    "create it with captions.set_style, or use position instead",
                ));
            }
            Some(r.clone())
        }
        (None, Some(pos)) => {
            if !matches!(pos.as_str(), "bottom" | "top" | "center") {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("unknown position '{pos}'"),
                    "must be bottom|top|center",
                ));
            }
            let key = format!("txt_{pos}");
            styles.entry(key.clone()).or_insert(cut_core::CaptionStyle {
                font: "Inter".into(),
                size: 64, // title-card scale (generated captions default 42)
                color: "#fff".into(),
                bg: Some("#000c".into()),
                pos: Some(pos.clone()),
                extra: Default::default(),
            });
            Some(key)
        }
        (None, None) => None, // renderer's Default style (bottom)
    };
    // Find-or-create the txt1 track and place the clip, kept ordered by
    // range start (caption ranges are absolute; overlaps with cap1 are fine
    // — different tracks, ASS renders both).
    let mut tracks = store.project.tracks.clone();
    if !tracks.iter().any(|t| t.id == TEXT_TRACK_ID) {
        tracks.push(cut_core::Track {
            id: TEXT_TRACK_ID.into(),
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
    }
    let track = tracks
        .iter_mut()
        .find(|t| t.id == TEXT_TRACK_ID)
        .expect("just ensured");
    // Deterministic id: max existing txt_N + 1 (replay-safe by construction).
    let n = track
        .clips
        .iter()
        .filter_map(|c| c.id()?.strip_prefix("txt_")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let clip_id = format!("txt_{:04}", n + 1);
    let clip = cut_core::CaptionClip {
        id: clip_id.clone(),
        text: a.text.clone(),
        style_ref: style_ref.clone(),
        range_ms: a.range_ms,
    };
    let at = track
        .clips
        .iter()
        .position(|c| matches!(c, cut_core::Clip::Caption(cc) if cc.range_ms[0] > a.range_ms[0]))
        .unwrap_or(track.clips.len());
    track.clips.insert(at, cut_core::Clip::Caption(clip));
    // Warn (in-band) when the card extends past the media — the composition
    // grows and the renderer shows black behind the text (a legitimate outro
    // card, but the agent should know it changed the duration).
    let media_end = store
        .project
        .tracks
        .iter()
        .filter(|t| t.kind != cut_core::TrackKind::Caption)
        .map(|t| t.duration_ms())
        .max()
        .unwrap_or(0);
    let extends = a.range_ms[1] > media_end && media_end > 0;
    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": tracks,
            "markers": store.project.markers,
            "caption_styles": styles,
        }),
    }];
    let extra = vec![effect(
        Some(TEXT_TRACK_ID),
        json!({"added_text": clip_id, "range_ms": a.range_ms, "style_ref": style_ref}),
    )];
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_legacy_inverse = wants_legacy_inverse(&args);
    let op = guard_call("captions.add_text", || {
        store.apply_lowered("captions.add_text", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    let mut res = VerbResult::ok_with_ops(
        json!({
            "clip_id": clip_id,
            "track_id": TEXT_TRACK_ID,
            "range_ms": a.range_ms,
            "style_ref": style_ref,
            "op": op_for_result(&op, include_legacy_inverse),
        }),
        vec![op_id],
    );
    if extends {
        res = res.with_warnings(vec![cut_core::VerbWarning {
            code: "text_extends_composition".into(),
            message: format!(
                "text card ends at {}ms but media ends at {media_end}ms — the composition grows and black renders behind the text",
                a.range_ms[1]
            ),
            detail: Default::default(),
        }]);
    }
    Ok(res)
}

/// captions.set_style{ref, style} — upsert a named caption style (live now).
/// BUILT-IN caption looks (the TITLE_TEMPLATES pattern: a shipped read-only
/// catalog surfaced by captions.list_styles, applied by captions.apply_style).
/// Only fields the ASS writer actually renders (font/size/color/bg/pos) — no
/// aspirational knobs. User presets may not shadow these names.
fn caption_style_builtins() -> Vec<(&'static str, cut_core::CaptionStyle)> {
    let mk =
        |font: &str, size: u32, color: &str, bg: Option<&str>, pos: &str| cut_core::CaptionStyle {
            font: font.into(),
            size,
            color: color.into(),
            bg: bg.map(String::from),
            pos: Some(pos.into()),
            extra: Default::default(),
        };
    vec![
        ("clean", mk("Inter", 42, "#fff", None, "bottom")),
        ("bold pop", mk("Inter", 56, "#fff", Some("#000c"), "bottom")),
        (
            "subtle card",
            mk("Inter", 36, "#fff", Some("#0008"), "bottom"),
        ),
        ("top banner", mk("Inter", 40, "#fff", Some("#000a"), "top")),
        ("center title", mk("Inter", 64, "#fff", None, "center")),
        (
            "broadcast yellow",
            mk("Inter", 44, "#ffe14d", None, "bottom"),
        ),
    ]
}

/// captions.save_style{name, ref?|style?} — save a named CAPTION STYLE PRESET.
/// Snapshot from an existing project style
/// key (`ref` — "I styled my captions, keep this look") or save an inline
/// `style` object directly. Name-keyed upsert; built-in names are reserved.
/// Replay-safe metadata op off the undo cursor.
pub(super) async fn captions_save_style(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
        #[serde(rename = "ref")]
        style_ref: Option<String>,
        style: Option<cut_core::CaptionStyle>,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let name = a.name.trim().to_string();
    if name.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "preset needs a non-empty name",
            "the name is the gallery key (re-save replaces)",
        ));
    }
    if caption_style_builtins().iter().any(|(n, _)| *n == name) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("'{name}' is a built-in look"),
            "built-in catalog names are reserved; pick another name",
        ));
    }
    if a.style_ref.is_some() == a.style.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "captions.save_style needs exactly one of `ref` or `style`",
            "ref snapshots an existing project style key; style saves an inline object",
        ));
    }
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let style = match (&a.style_ref, a.style) {
        (Some(r), None) => store
            .project
            .caption_styles
            .get(r)
            .cloned()
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no caption style '{r}' in the project"),
                    "project.state lists caption_styles keys; captions.set_style creates them",
                )
            })?,
        (None, Some(s)) => s,
        _ => unreachable!(),
    };
    let preset = cut_core::CaptionStylePreset { name, style };
    let (replaced, op) = guard_call("captions.save_style", || {
        store.save_caption_style_preset(preset.clone(), actor, rationale)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "preset": preset,
            "replaced": replaced,
            "op": op_for_result(&op, wants_legacy_inverse(&args)),
        }),
        vec![op_id],
    ))
}

/// captions.apply_style{name, ref?} — apply a preset (user gallery first, then
/// the built-in catalog) by LOWERING to plain captions.set_style ops (concrete
/// style recorded per op → replay-safe + undoable, independent of the preset's
/// continued existence — the grade.apply doctrine). `ref` targets ONE project
/// style key; omitted = restyle EVERY key referenced by current caption clips
/// ("apply this look to my captions"). Refuses NOT_FOUND when omitted and no
/// caption clip carries a style_ref (nothing to restyle — pass ref).
pub(super) async fn captions_apply_style(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
        #[serde(rename = "ref")]
        style_ref: Option<String>,
    }
    let a: Args = parse_args(args.clone())?;
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("apply caption look '{}'", a.name));
    // Resolve the preset + the target refs under a read lock; apply after.
    let (style, refs) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let style = store
            .project
            .caption_style_presets
            .iter()
            .find(|p| p.name == a.name)
            .map(|p| p.style.clone())
            .or_else(|| {
                caption_style_builtins()
                    .into_iter()
                    .find(|(n, _)| *n == a.name)
                    .map(|(_, s)| s)
            })
            .ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("no caption style preset '{}'", a.name),
                    "captions.list_styles shows saved + built-in looks",
                )
            })?;
        let refs: Vec<String> = match &a.style_ref {
            Some(r) => vec![r.clone()],
            None => {
                // Every style key referenced by current caption clips (dedup, stable).
                let mut seen = std::collections::BTreeSet::new();
                for t in store
                    .project
                    .tracks
                    .iter()
                    .filter(|t| t.kind == cut_core::TrackKind::Caption)
                {
                    for c in &t.clips {
                        if let cut_core::Clip::Caption(cc) = c {
                            if let Some(r) = &cc.style_ref {
                                seen.insert(r.clone());
                            }
                        }
                    }
                }
                seen.into_iter().collect()
            }
        };
        if refs.is_empty() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                "no caption clips carry a style_ref — nothing to restyle",
                "generate/import captions first, or pass `ref` to create/update a specific style key",
            ));
        }
        (style, refs)
    };
    // Lower to one captions.set_style op per ref (each undoable/replay-safe).
    let mut op_ids: Vec<String> = Vec::new();
    let mut updated: Vec<String> = Vec::new();
    for r in &refs {
        let res = captions_set_style(
            state,
            json!({"ref": r, "style": style, "rationale": rationale}),
            actor.clone(),
        )
        .await?;
        if let Some(ids) = res.op_ids {
            op_ids.extend(ids);
        }
        updated.push(r.clone());
    }
    Ok(VerbResult::ok_with_ops(
        json!({
            "applied": a.name,
            "refs_updated": updated,
            "style": style,
        }),
        op_ids,
    ))
}

/// captions.list_styles{} — read-only: the caption style gallery — user-saved
/// presets first (builtin:false), then the shipped built-in catalog.
pub(super) async fn captions_list_styles(
    state: &AppState,
    _args: Value,
) -> Result<VerbResult, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let mut rows: Vec<Value> = store
        .project
        .caption_style_presets
        .iter()
        .map(|p| json!({"name": p.name, "style": p.style, "builtin": false}))
        .collect();
    for (name, style) in caption_style_builtins() {
        rows.push(json!({"name": name, "style": style, "builtin": true}));
    }
    Ok(VerbResult::ok(json!({
        "count": rows.len(),
        "presets": rows,
    })))
}

pub(super) async fn captions_set_style(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        #[serde(rename = "ref")]
        style_ref: String,
        style: cut_core::CaptionStyle,
    }
    let a: Args = parse_args(args.clone())?;
    // Committed as a lowered edit._set_timeline step (caption_styles ride on
    // the snapshot) so the op replays and is undoable like every mutation.
    let mut guard = state.project.write().await;
    let store = guard.as_mut().ok_or_else(no_project)?;
    let mut styles = store.project.caption_styles.clone();
    let replaced = styles.insert(a.style_ref.clone(), a.style).is_some();
    let steps = vec![InverseOp {
        verb: "edit._set_timeline".into(),
        args: json!({
            "tracks": store.project.tracks,
            "markers": store.project.markers,
            "caption_styles": styles,
        }),
    }];
    let extra = vec![effect(
        None,
        json!({"style_ref": a.style_ref, "replaced": replaced}),
    )];
    let rationale = args
        .get("rationale")
        .and_then(|r| r.as_str())
        .map(String::from);
    let include_legacy_inverse = wants_legacy_inverse(&args);
    let op = guard_call("captions.set_style", || {
        store.apply_lowered("captions.set_style", args, actor, rationale, steps, extra)
    })?;
    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({"style_ref": a.style_ref, "replaced": replaced, "op": op_for_result(&op, include_legacy_inverse)}),
        vec![op_id],
    ))
}
