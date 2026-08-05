//! candidates.rs — clip.candidates ranking (social repurposing pipeline).
//!
//! Role: from an asset's word-level transcript, propose the windows most likely
//! to work as standalone short-form clips, ranked by a HOOK score (does the
//! opening grab?) and a RETENTION proxy (is the pacing tight, low on fillers?).
//! Pure analysis over the transcript (+ a filler lexicon) — no render, no model,
//! no op. Honest heuristic v1: every result is labelled scoring:"heuristic" by
//! the caller; this is a deterministic editorial prior, NOT a trained virality
//! model, and the `reason` string always says WHY a window ranked where it did
//! so an editor/agent can audit and override.
//!
//! Design: the transcript is split into "sentences" at punctuation or a long
//! inter-word gap; candidate windows grow from each sentence start to fit
//! [min_ms, max_ms]; windows are scored, sorted, then non-maximally suppressed
//! by time-overlap so the top-N are DISTINCT moments, not N slices of the same
//! 20 seconds. Deps: types.rs (Transcript/WordSpan). Callers: server
//! clip.candidates verb.

use crate::types::Transcript;
use serde::{Deserialize, Serialize};

/// A gap larger than this between two words ends a "sentence" even without
/// terminal punctuation — a held pause is a natural clip boundary (ASR often
/// drops punctuation, so timing carries the structure).
const SENTENCE_GAP_MS: u64 = 700;

/// Words whose presence in the OPENING line strengthens the hook. Lowercased,
/// punctuation-stripped match. Grounded in short-form retention guidance
/// (question/number/curiosity-gap openers out-perform flat statements).
const HOOK_POWER_WORDS: &[&str] = &[
    "secret", "mistake", "never", "always", "best", "worst", "top", "free", "proven", "stop",
    "biggest", "truth", "nobody", "everyone", "warning", "avoid", "hack", "easy", "fast", "rule",
    "reason", "why", "how",
];

/// Opening question words — a candidate that OPENS on one of these reads as a
/// curiosity-gap hook.
const QUESTION_OPENERS: &[&str] = &[
    "how", "why", "what", "whats", "when", "where", "who", "which", "can", "should", "is", "are",
    "do",
];

/// Ideal speaking pace band (words/sec) for an engaging short — ~120–190 WPM.
/// Inside the band scores 1.0; outside, a linear falloff.
const IDEAL_WPS_LO: f64 = 2.0;
const IDEAL_WPS_HI: f64 = 3.2;

/// Tunable bounds for candidate generation (clip.candidates args).
#[derive(Debug, Clone, Copy)]
pub struct CandidateOpts {
    /// How many ranked candidates to return (after overlap-suppression).
    pub count: usize,
    /// Minimum candidate length — a clip shorter than this is rarely a complete
    /// thought.
    pub min_ms: u64,
    /// Maximum candidate length — the short-form ceiling.
    pub max_ms: u64,
}

impl Default for CandidateOpts {
    fn default() -> Self {
        Self {
            count: 5,
            min_ms: 12_000,
            max_ms: 60_000,
        }
    }
}

/// One ranked candidate clip (read-only — the caller turns it into a render via
/// render.bundle). `word_range` is [lo, hi) into the source transcript; `at_ms`
/// / `dur_ms` are SOURCE-time so the caller can map to the timeline or feed
/// render.bundle directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipCandidate {
    pub asset: String,
    pub word_range: [usize; 2],
    pub at_ms: u64,
    pub dur_ms: u64,
    /// 0..1 — strength of the opening line (question/number/power-word/punch).
    pub hook_score: f64,
    /// 0..1 — pacing + filler-density retention proxy.
    pub retention_score: f64,
    /// 0..1 — combined rank key (0.6·hook + 0.4·retention; hook dominates shorts).
    pub score: f64,
    /// Human-auditable WHY (what drove the score) — never a bare number.
    pub reason: String,
    /// First ~120 chars of the clip's words, for the picker card.
    pub transcript_excerpt: String,
}

/// Internal sentence unit: inclusive word index span + its time span.
struct Sentence {
    w_lo: usize,
    w_hi: usize, // inclusive
    start_ms: u64,
    end_ms: u64,
}

/// Normalise a token for matching: lowercase, keep only alphanumerics.
fn norm(w: &str) -> String {
    w.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn is_filler(word: &str, lexicon: &[String]) -> bool {
    let clean = norm(word);
    !clean.is_empty() && lexicon.contains(&clean)
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Split the transcript into sentences at terminal punctuation OR a > SENTENCE_GAP_MS
/// inter-word pause.
fn split_sentences(t: &Transcript) -> Vec<Sentence> {
    let mut out = Vec::new();
    if t.words.is_empty() {
        return out;
    }
    let mut lo = 0usize;
    for i in 0..t.words.len() {
        let w = &t.words[i];
        let ends_punct = w.word.trim_end().ends_with(['.', '?', '!']);
        let gap_after = t
            .words
            .get(i + 1)
            .map(|n| n.start_ms.saturating_sub(w.end_ms) > SENTENCE_GAP_MS)
            .unwrap_or(true); // last word always closes a sentence
        if ends_punct || gap_after {
            out.push(Sentence {
                w_lo: lo,
                w_hi: i,
                start_ms: t.words[lo].start_ms,
                end_ms: w.end_ms,
            });
            lo = i + 1;
        }
    }
    out
}

/// Score the opening line (the first sentence's words) → hook 0..1 + reasons.
fn hook_score(t: &Transcript, first: &Sentence) -> (f64, Vec<String>) {
    let words: Vec<String> = (first.w_lo..=first.w_hi)
        .map(|i| norm(&t.words[i].word))
        .collect();
    let raw: Vec<&str> = (first.w_lo..=first.w_hi)
        .map(|i| t.words[i].word.as_str())
        .collect();
    let mut score = 0.0;
    let mut reasons = Vec::new();

    if let Some(first_word) = words.first() {
        if QUESTION_OPENERS.contains(&first_word.as_str()) {
            score += 0.30;
            reasons.push("opens on a question word".into());
        }
    }
    if raw.iter().any(|w| w.contains('?')) {
        score += 0.22;
        reasons.push("contains a question".into());
    }
    // A number — digit token or a number-ish word.
    let has_number = words.iter().any(|w| w.chars().any(|c| c.is_ascii_digit()))
        || words.iter().any(|w| {
            matches!(
                w.as_str(),
                "one" | "two" | "three" | "five" | "ten" | "hundred" | "thousand" | "million"
            )
        });
    if has_number {
        score += 0.15;
        reasons.push("leads with a number".into());
    }
    let power_hits: Vec<&String> = words
        .iter()
        .filter(|w| HOOK_POWER_WORDS.contains(&w.as_str()))
        .collect();
    if !power_hits.is_empty() {
        score += (0.10 * power_hits.len() as f64).min(0.25);
        reasons.push(format!("curiosity-gap word(s): {}", power_hits.len()));
    }
    // A short, punchy opener holds attention better than a rambling one.
    let opener_len = first.w_hi - first.w_lo + 1;
    if opener_len <= 12 {
        score += 0.12;
        reasons.push("punchy opener".into());
    } else if opener_len > 30 {
        score -= 0.08;
        reasons.push("long, unfocused opener".into());
    }
    (score.clamp(0.0, 1.0), reasons)
}

/// Retention proxy over the whole candidate window: pacing inside the ideal
/// band + low filler density.
fn retention_score(
    t: &Transcript,
    w_lo: usize,
    w_hi: usize,
    dur_ms: u64,
    fillers: &[String],
) -> (f64, Vec<String>) {
    let n_words = w_hi.saturating_sub(w_lo) + 1;
    let mut reasons = Vec::new();
    if dur_ms == 0 || n_words == 0 {
        return (0.0, vec!["empty window".into()]);
    }
    let wps = n_words as f64 / (dur_ms as f64 / 1000.0);
    // Pace score: 1.0 inside [LO,HI], linear falloff to 0 at ±1.5 wps outside.
    let pace = if (IDEAL_WPS_LO..=IDEAL_WPS_HI).contains(&wps) {
        1.0
    } else if wps < IDEAL_WPS_LO {
        (1.0 - (IDEAL_WPS_LO - wps) / 1.5).max(0.0)
    } else {
        (1.0 - (wps - IDEAL_WPS_HI) / 1.5).max(0.0)
    };
    let filler_count = (w_lo..=w_hi)
        .filter(|&i| is_filler(&t.words[i].word, fillers))
        .count();
    let filler_ratio = filler_count as f64 / n_words as f64;
    let filler_penalty = (filler_ratio * 2.0).min(0.5);
    let score = (0.15 + 0.85 * pace - filler_penalty).clamp(0.0, 1.0);
    reasons.push(format!("{:.1} words/s pace", wps));
    if filler_count > 0 {
        reasons.push(format!("{:.0}% fillers", filler_ratio * 100.0));
    }
    (score, reasons)
}

/// Build the excerpt text (first ~120 chars, ellipsised).
fn excerpt(t: &Transcript, w_lo: usize, w_hi: usize) -> String {
    let s: String = (w_lo..=w_hi)
        .map(|i| t.words[i].word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= 120 {
        s
    } else {
        format!("{}…", s.chars().take(120).collect::<String>())
    }
}

/// Rank candidate clips for one asset's transcript. Returns up to `opts.count`
/// DISTINCT windows (overlap-suppressed), best first. Empty transcript → empty.
pub fn clip_candidates(
    t: &Transcript,
    fillers: &[String],
    opts: CandidateOpts,
) -> Vec<ClipCandidate> {
    let sentences = split_sentences(t);
    if sentences.is_empty() {
        return Vec::new();
    }
    let mut cands: Vec<ClipCandidate> = Vec::new();
    // Grow a window from each sentence start until it reaches min_ms (or runs
    // out / would exceed max_ms). One candidate per viable start.
    for si in 0..sentences.len() {
        let start = &sentences[si];
        let mut sj = si;
        while sj + 1 < sentences.len() {
            let dur = sentences[sj].end_ms.saturating_sub(start.start_ms);
            if dur >= opts.min_ms {
                break;
            }
            // Adding the next sentence must not blow past max_ms (unless we
            // still haven't reached min_ms with a single long sentence).
            let next_dur = sentences[sj + 1].end_ms.saturating_sub(start.start_ms);
            if next_dur > opts.max_ms && dur >= opts.min_ms {
                break;
            }
            sj += 1;
        }
        let w_lo = start.w_lo;
        let w_hi = sentences[sj].w_hi;
        let at_ms = start.start_ms;
        let end_ms = sentences[sj].end_ms;
        let dur_ms = end_ms.saturating_sub(at_ms);
        // Skip windows that can't reach a complete-thought minimum (unless the
        // whole asset is shorter — then keep the single best-effort window).
        if dur_ms < opts.min_ms && sentences.len() > 1 {
            continue;
        }
        let (hook, mut hook_reasons) = hook_score(t, start);
        let (ret, ret_reasons) = retention_score(t, w_lo, w_hi, dur_ms, fillers);
        let score = round2(0.6 * hook + 0.4 * ret);
        hook_reasons.extend(ret_reasons);
        let reason = if hook_reasons.is_empty() {
            "flat opener, average pacing".to_string()
        } else {
            hook_reasons.join("; ")
        };
        cands.push(ClipCandidate {
            asset: t.asset.clone(),
            word_range: [w_lo, w_hi + 1],
            at_ms,
            dur_ms,
            hook_score: round2(hook),
            retention_score: round2(ret),
            score,
            reason,
            transcript_excerpt: excerpt(t, w_lo, w_hi),
        });
    }
    // Sort best-first, then non-maximally suppress by time overlap so the top-N
    // are distinct moments (a window and its near-twin starting one sentence
    // later shouldn't both survive).
    cands.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut chosen: Vec<ClipCandidate> = Vec::new();
    for c in cands {
        let c_end = c.at_ms + c.dur_ms;
        let overlaps = chosen.iter().any(|k| {
            let k_end = k.at_ms + k.dur_ms;
            let inter = c_end.min(k_end).saturating_sub(c.at_ms.max(k.at_ms));
            let shorter = c.dur_ms.min(k.dur_ms).max(1);
            inter as f64 / shorter as f64 > 0.5
        });
        if !overlaps {
            chosen.push(c);
        }
        if chosen.len() >= opts.count {
            break;
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WordSpan;

    /// Build a transcript from (word, start_ms, end_ms) triples.
    fn tx(asset: &str, ws: &[(&str, u64, u64)]) -> Transcript {
        Transcript {
            asset: asset.into(),
            model: "test".into(),
            language: Some("en".into()),
            words: ws
                .iter()
                .enumerate()
                .map(|(i, (w, s, e))| WordSpan {
                    idx: i,
                    word: (*w).into(),
                    start_ms: *s,
                    end_ms: *e,
                    confidence: Some(1.0),
                    speaker: None,
                })
                .collect(),
        }
    }

    /// Generate `n` words at `wps` words/sec starting at `t0`, each word `text`.
    fn run(text: &str, t0: u64, n: usize, wps: f64) -> Vec<(String, u64, u64)> {
        let step = (1000.0 / wps) as u64;
        (0..n)
            .map(|i| {
                let s = t0 + i as u64 * step;
                (text.to_string(), s, s + step - 10)
            })
            .collect()
    }

    #[test]
    fn empty_transcript_yields_no_candidates() {
        let t = tx("a1", &[]);
        assert!(clip_candidates(&t, &[], CandidateOpts::default()).is_empty());
    }

    #[test]
    fn question_opener_outranks_flat_statement() {
        // Two ~15s windows separated by a long gap so they're distinct sentences:
        // A: opens "How" (+ ends '?') — strong hook. B: flat "the the…".
        let mut words: Vec<(String, u64, u64)> = Vec::new();
        // Window A: "How do you win?" then filler-free body, 2.6 wps, ~14s.
        words.push(("How".into(), 0, 380));
        words.push(("do".into(), 400, 780));
        words.push(("you".into(), 800, 1180));
        words.push(("win?".into(), 1200, 1580));
        words.extend(run("clearly", 1700, 32, 2.6));
        // Big gap → sentence/clip boundary.
        let b0 = 30_000;
        words.extend(run("the", b0, 40, 2.6));
        let owned: Vec<(&str, u64, u64)> =
            words.iter().map(|(w, s, e)| (w.as_str(), *s, *e)).collect();
        let t = tx("a1", &owned);
        let cands = clip_candidates(
            &t,
            &[],
            CandidateOpts {
                count: 5,
                min_ms: 10_000,
                max_ms: 60_000,
            },
        );
        assert!(
            cands.len() >= 2,
            "expected ≥2 distinct windows, got {}",
            cands.len()
        );
        // The question-opener window ranks first and starts at 0.
        assert_eq!(
            cands[0].at_ms, 0,
            "question-opener window should rank #1: {cands:?}"
        );
        assert!(
            cands[0].hook_score > cands[1].hook_score,
            "hook must dominate: {cands:?}"
        );
        assert!(cands[0].score > 0.0);
        assert!(cands[0].reason.contains("question"));
        // word_range is [lo, hi) and the excerpt is non-empty.
        assert_eq!(cands[0].word_range[0], 0);
        assert!(!cands[0].transcript_excerpt.is_empty());
    }

    #[test]
    fn fillers_lower_retention() {
        // Same pace, one window peppered with "um".
        let clean: Vec<(String, u64, u64)> = run("word", 0, 40, 2.6);
        let mut filled = clean.clone();
        for (i, w) in filled.iter_mut().enumerate() {
            if i % 3 == 0 {
                w.0 = "um".into();
            }
        }
        let to = |v: &Vec<(String, u64, u64)>| -> Transcript {
            let owned: Vec<(&str, u64, u64)> =
                v.iter().map(|(w, s, e)| (w.as_str(), *s, *e)).collect();
            tx("a1", &owned)
        };
        let lex = vec!["um".to_string()];
        let c_clean = clip_candidates(
            &to(&clean),
            &lex,
            CandidateOpts {
                count: 1,
                min_ms: 10_000,
                max_ms: 60_000,
            },
        );
        let c_filled = clip_candidates(
            &to(&filled),
            &lex,
            CandidateOpts {
                count: 1,
                min_ms: 10_000,
                max_ms: 60_000,
            },
        );
        assert!(!c_clean.is_empty() && !c_filled.is_empty());
        assert!(
            c_clean[0].retention_score > c_filled[0].retention_score,
            "fillers must lower retention: clean={} filled={}",
            c_clean[0].retention_score,
            c_filled[0].retention_score
        );
    }

    #[test]
    fn ranking_is_deterministic() {
        let words: Vec<(String, u64, u64)> = run("steady", 0, 80, 2.6);
        let owned: Vec<(&str, u64, u64)> =
            words.iter().map(|(w, s, e)| (w.as_str(), *s, *e)).collect();
        let t = tx("a1", &owned);
        let a = clip_candidates(&t, &[], CandidateOpts::default());
        let b = clip_candidates(&t, &[], CandidateOpts::default());
        assert_eq!(a, b, "same input must yield identical ranking");
    }
}
