//! diarize.rs — PURE word↔speaker alignment + report merge for `media.diarize`.
//!
//! Role: the dependency-free, deterministic half of speaker diarization that lives
//! in `cut_perception`. The heavy "who spoke when" inference is done OUT OF PROCESS
//! (the Sortformer service via `server/diarize.rs` + `py/diarize_runner.py`);
//! THIS module only takes the normalized [`SpeakerTurn`]s that come back and
//!   1. labels each transcript [`WordSpan`] with the speaker of the turn it overlaps
//!      MOST in time ([`assign_word_speakers`]), and
//!   2. merges the turns + provenance into a [`PerceptionReport`]
//!      ([`apply_diarization`]) so the receipt + the words.json carry speaker labels.
//!
//! Both functions are pure (no I/O, no clock) and unit-tested, so the alignment is
//! audit-able without a GPU or the network. The dispatch handler (server) owns the
//! file read/write around them. Alignment is overlap-MAX (sum of per-speaker
//! intersection, argmax) which beats "word midpoint in turn" at turn boundaries and
//! handles overlapped speech; a word in a gap snaps to the nearest turn within
//! [`ASSIGN_TOL_MS`], else stays `None`.
//!
//! Dependencies: types.rs only. Primary callers: server `media.diarize` handler.

use crate::types::{Diarization, PerceptionReport, SpeakerTurn, WordSpan};

/// A word that overlaps NO turn snaps to the nearest turn within this tolerance
/// (ms); beyond it the word is left unlabeled (`None`). Covers the common case of
/// a word whose timestamp sits a hair outside a turn edge (ASR vs diarizer frame
/// granularity) without mislabeling words in a genuine silence gap.
pub const ASSIGN_TOL_MS: u64 = 200;

/// Time-overlap (ms) of `[a0,a1)` and `[b0,b1)` — 0 when disjoint.
fn overlap_ms(a0: u64, a1: u64, b0: u64, b1: u64) -> u64 {
    let lo = a0.max(b0);
    let hi = a1.min(b1);
    hi.saturating_sub(lo)
}

/// Gap (ms) between a word `[ws,we)` and a turn `[ts,te)` when they DON'T overlap
/// (the word is wholly before or after the turn). Returns 0 if they touch/overlap.
fn gap_ms(ws: u64, we: u64, ts: u64, te: u64) -> u64 {
    if we <= ts {
        ts - we // word ends before the turn starts
    } else {
        // word starts after the turn (ws >= te) → ws - te; an overlap (ws < te) → 0.
        ws.saturating_sub(te)
    }
}

/// The canonical speaker label for one word `[start_ms, end_ms)` given the diarized
/// `turns` (assumed sorted by `start_ms`). Overlap-MAX: the speaker whose turns sum
/// the most intersection with the word wins; ties → the EARLIER-arriving speaker
/// (the one whose first contributing turn appears first in `turns`). A word that
/// overlaps no turn snaps to the nearest turn within [`ASSIGN_TOL_MS`], else `None`.
fn word_speaker(ws: u64, we: u64, turns: &[SpeakerTurn]) -> Option<String> {
    // Per-speaker total overlap, in first-appearance order (so ties resolve to the
    // earlier-arriving speaker deterministically). Speakers are few (≤4-ish) → a
    // linear Vec is simpler and faster than a hash map here.
    let mut acc: Vec<(&str, u64)> = Vec::new();
    for t in turns {
        let ov = overlap_ms(ws, we, t.start_ms, t.end_ms);
        if ov == 0 {
            continue;
        }
        match acc.iter_mut().find(|(s, _)| *s == t.speaker.as_str()) {
            Some(entry) => entry.1 += ov,
            None => acc.push((t.speaker.as_str(), ov)),
        }
    }
    // Best by overlap; strictly-greater replaces, so the first speaker to reach the
    // max (earliest appearance) keeps it on a tie.
    if let Some((best, _)) =
        acc.iter()
            .fold(None, |best: Option<(&str, u64)>, &(s, o)| match best {
                Some((_, bo)) if bo >= o => best,
                _ => Some((s, o)),
            })
    {
        return Some(best.to_string());
    }

    // No overlap anywhere → nearest turn within tolerance (first wins on a tie).
    let mut nearest: Option<(u64, &str)> = None;
    for t in turns {
        let g = gap_ms(ws, we, t.start_ms, t.end_ms);
        if nearest.is_none_or(|(bg, _)| g < bg) {
            nearest = Some((g, t.speaker.as_str()));
        }
    }
    match nearest {
        Some((g, s)) if g <= ASSIGN_TOL_MS => Some(s.to_string()),
        _ => None,
    }
}

/// Assign each word the speaker of the turn it overlaps MOST in time (overlap-max;
/// see [`word_speaker`]). Deterministic and IDEMPOTENT — re-running with new turns
/// overwrites prior labels, and an EMPTY `turns` clears every label (so a re-diarize
/// that found nothing doesn't leave stale labels). Returns how many words got a
/// non-`None` speaker. `turns` should be sorted by `start_ms` (callers via
/// [`apply_diarization`] guarantee this).
pub fn assign_word_speakers(words: &mut [WordSpan], turns: &[SpeakerTurn]) -> usize {
    let mut labeled = 0usize;
    for w in words.iter_mut() {
        let s = if turns.is_empty() {
            None
        } else {
            word_speaker(w.start_ms, w.end_ms, turns)
        };
        if s.is_some() {
            labeled += 1;
        }
        w.speaker = s;
    }
    labeled
}

/// Merge diarization results into a [`PerceptionReport`]: sort the turns by start,
/// label the report's transcript words (if any) by max overlap, set
/// `speaker_turns` + `diarization`, and record the `"diarize"` instrument token
/// (idempotent). Returns the number of words that received a speaker label (0 when
/// the report carries no transcript yet — the turns are still recorded, and words
/// get labeled on the next `media.diarize` after a transcript exists). Pure — the
/// caller owns persisting the mutated report + refreshing `<asset>.words.json`.
pub fn apply_diarization(
    report: &mut PerceptionReport,
    mut turns: Vec<SpeakerTurn>,
    diar: Diarization,
) -> usize {
    turns.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then(a.end_ms.cmp(&b.end_ms)));
    let labeled = match report.words.as_mut() {
        Some(t) => assign_word_speakers(&mut t.words, &turns),
        None => 0,
    };
    report.speaker_turns = turns;
    report.diarization = Some(diar);
    if !report.instruments_run.iter().any(|r| r == "diarize") {
        report.instruments_run.push("diarize".to_string());
    }
    labeled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Transcript, PERCEPTION_SCHEMA};

    fn turn(start_ms: u64, end_ms: u64, speaker: &str) -> SpeakerTurn {
        SpeakerTurn {
            start_ms,
            end_ms,
            speaker: speaker.to_string(),
        }
    }

    fn word(idx: usize, start_ms: u64, end_ms: u64) -> WordSpan {
        WordSpan {
            idx,
            word: format!("w{idx}"),
            start_ms,
            end_ms,
            confidence: None,
            speaker: None,
        }
    }

    /// A word wholly inside a turn takes that turn's speaker.
    #[test]
    fn word_inside_turn_takes_its_speaker() {
        let turns = vec![turn(0, 3000, "S1"), turn(3000, 6000, "S2")];
        let mut words = vec![word(0, 500, 900), word(1, 4000, 4500)];
        let n = assign_word_speakers(&mut words, &turns);
        assert_eq!(n, 2);
        assert_eq!(words[0].speaker.as_deref(), Some("S1"));
        assert_eq!(words[1].speaker.as_deref(), Some("S2"));
    }

    /// A word straddling the boundary between two turns goes to the speaker it
    /// overlaps MORE (overlap-max, not midpoint).
    #[test]
    fn straddling_word_goes_to_max_overlap() {
        let turns = vec![turn(0, 3000, "S1"), turn(3000, 6000, "S2")];
        // [2900,3400): 100ms in S1, 400ms in S2 → S2.
        let mut words = vec![word(0, 2900, 3400)];
        assign_word_speakers(&mut words, &turns);
        assert_eq!(words[0].speaker.as_deref(), Some("S2"));
        // [2600,3100): 400ms in S1, 100ms in S2 → S1.
        let mut words2 = vec![word(0, 2600, 3100)];
        assign_word_speakers(&mut words2, &turns);
        assert_eq!(words2[0].speaker.as_deref(), Some("S1"));
    }

    /// Overlapped speech (turns from different speakers overlapping in time): a word
    /// inside the overlap zone goes to whichever speaker covers more of it.
    #[test]
    fn overlapped_turns_pick_dominant_speaker() {
        // S1 [0,5000), S2 [3000,8000) overlap on [3000,5000).
        let turns = vec![turn(0, 5000, "S1"), turn(3000, 8000, "S2")];
        // [4500,5500): S1 covers [4500,5000)=500, S2 covers [4500,5500)=1000 → S2.
        let mut words = vec![word(0, 4500, 5500)];
        assign_word_speakers(&mut words, &turns);
        assert_eq!(words[0].speaker.as_deref(), Some("S2"));
    }

    /// A word in a gap snaps to the nearest turn within tolerance, else stays None.
    #[test]
    fn gap_word_snaps_within_tolerance_else_none() {
        let turns = vec![turn(0, 1000, "S1"), turn(5000, 6000, "S2")];
        // 150ms after S1's end → within 200ms tol → S1.
        let mut near = vec![word(0, 1150, 1300)];
        assign_word_speakers(&mut near, &turns);
        assert_eq!(near[0].speaker.as_deref(), Some("S1"));
        // Smack in the middle of the gap → beyond tol → None.
        let mut far = vec![word(0, 2500, 2800)];
        let n = assign_word_speakers(&mut far, &turns);
        assert_eq!(n, 0);
        assert_eq!(far[0].speaker, None);
    }

    /// Tie on overlap resolves to the earlier-arriving speaker (determinism).
    #[test]
    fn equal_overlap_breaks_to_earlier_speaker() {
        let turns = vec![turn(0, 2000, "S1"), turn(2000, 4000, "S2")];
        // [1500,2500): 500ms each → tie → earlier (S1).
        let mut words = vec![word(0, 1500, 2500)];
        assign_word_speakers(&mut words, &turns);
        assert_eq!(words[0].speaker.as_deref(), Some("S1"));
    }

    /// Empty turns clears any prior labels (idempotent re-diarize that found nothing).
    #[test]
    fn empty_turns_clears_prior_labels() {
        let mut words = vec![word(0, 100, 200)];
        words[0].speaker = Some("S9".into());
        let n = assign_word_speakers(&mut words, &[]);
        assert_eq!(n, 0);
        assert_eq!(words[0].speaker, None);
    }

    fn bare_report(words: Option<Transcript>) -> PerceptionReport {
        PerceptionReport {
            schema: PERCEPTION_SCHEMA.into(),
            asset_hash: "sha256:cafe".into(),
            source_path: "/x.wav".into(),
            instruments_run: vec!["words".into()],
            words,
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

    /// apply_diarization records turns + provenance, labels words, and is idempotent
    /// on the "diarize" instrument token.
    #[test]
    fn apply_sets_turns_provenance_and_labels_words() {
        let transcript = Transcript {
            asset: "a1".into(),
            model: "whisperx-small".into(),
            language: Some("en".into()),
            words: vec![word(0, 100, 900), word(1, 3500, 3900)],
        };
        let mut report = bare_report(Some(transcript));
        // Pass UNSORTED turns to prove apply_diarization sorts them.
        let turns = vec![turn(3000, 6000, "S2"), turn(0, 3000, "S1")];
        let diar = Diarization {
            backend: "sortformer".into(),
            model: "sortformer-v2".into(),
            device: "cuda".into(),
            num_speakers: 2,
        };
        let labeled = apply_diarization(&mut report, turns, diar);
        assert_eq!(labeled, 2);
        assert_eq!(report.speaker_turns.len(), 2);
        assert_eq!(report.speaker_turns[0].start_ms, 0, "turns must be sorted");
        assert_eq!(report.speaker_turns[0].speaker, "S1");
        assert_eq!(report.diarization.as_ref().unwrap().num_speakers, 2);
        assert!(report.instruments_run.iter().any(|r| r == "diarize"));
        let w = &report.words.as_ref().unwrap().words;
        assert_eq!(w[0].speaker.as_deref(), Some("S1"));
        assert_eq!(w[1].speaker.as_deref(), Some("S2"));
        // Idempotent: a second apply doesn't duplicate the instrument token.
        apply_diarization(
            &mut report,
            vec![turn(0, 3000, "S1")],
            Diarization {
                backend: "sortformer".into(),
                model: "sortformer-v2".into(),
                device: "cuda".into(),
                num_speakers: 1,
            },
        );
        assert_eq!(
            report
                .instruments_run
                .iter()
                .filter(|r| *r == "diarize")
                .count(),
            1
        );
    }

    /// No transcript yet → turns are still recorded, 0 words labeled (the
    /// document-the-order path).
    #[test]
    fn apply_without_transcript_records_turns_only() {
        let mut report = bare_report(None);
        let labeled = apply_diarization(
            &mut report,
            vec![turn(0, 3000, "S1"), turn(3000, 6000, "S2")],
            Diarization {
                backend: "sortformer".into(),
                model: "sortformer-v2".into(),
                device: "cuda".into(),
                num_speakers: 2,
            },
        );
        assert_eq!(labeled, 0);
        assert_eq!(report.speaker_turns.len(), 2);
        assert!(report.diarization.is_some());
    }
}
