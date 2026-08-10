//! checks.rs — the verify.checks battery (public verb contract "verify").
//!
//! Role: deterministic Rust checks over perception facts + the EDL + measured
//! render facts — NO model in the loop. Each check returns a CheckResult
//! {name, pass, details, evidence}; evidence always carries the numbers that
//! justify the verdict. The server assembles these into the RenderReceipt
//! (render.final auto-runs the battery, public verb contract).
//!
//! Design note: checks take FACTS as inputs (no ffmpeg here) — the server
//! runs cut-media/sidecar on the RENDERED file first, then calls these. That
//! keeps cut-perception free of a cut-media dependency and the checks pure.
//!
//! Honesty rule: a check that lacks the facts it needs FAILS with an
//! explanation ("no loudness measured") rather than passing vacuously —
//! "never fake a pass" (public verb contract verify.judge note generalizes to all checks).
//! The one deliberate exception: cut_on_word treats assets WITHOUT a
//! transcript as unchecked-but-listed (a no-speech timeline is legitimate);
//! the unchecked list is always in the details so the agent can see the gap.
//!
//! FOOTAGE PROFILES (silent-screen profile regression): the battery interprets
//! the same measured facts under an editorial profile. `talking_head` is the
//! original behavior; `silent_screen_demo` waives the checks that are
//! by-design properties of silent screen footage (lufs, caption_presence,
//! silence_at_edges) and swaps the frozen-frame check for a UI-tuned variant.
//! A waiver is NEVER a silent drop: the check stays in the receipt with
//! pass=true + details.waived_by_profile + the measured outcome preserved.
//! Selection is EXPLICIT (run_all_with_profile arg); auto-detection only
//! PROPOSES a profile in the receipt's `footage_profile` entry.
//!
//! Dependencies: cut-core (Edl, CheckResult), types.rs. Primary callers:
//! server verify.checks verb + render.final completion hook.

use crate::types::{BeatGrid, ContentBbox, Loudness, PerceptionReport, Transcript};
use cut_core::{check_names, CheckResult, Edl, Project, TrackKind};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Tolerance (ms) for "a cut lands inside a word": EDL boundaries may sit
/// within ±40ms of a word edge (the cut_words padding, public verb contract) and still pass.
pub const CUT_ON_WORD_TOLERANCE_MS: u64 = 40;

/// uniform_border: a content inset of MORE than this many pixels on any
/// edge of the RENDER is a baked-in uniform border (letterbox/pillarbox) and
/// FAILS the check. Matches the sidecar's CONTENT_BBOX_EDGE_TOL_PX
/// (instruments.py) — a render whose content rect sits inside the frame by
/// <= this is cropdetect jitter, not a real margin. Change in BOTH or neither.
pub const UNIFORM_BORDER_MAX_INSET_PX: u32 = 8;

/// Edge tolerance (ms) when deciding whether a detected silence touches the
/// start/end of the output (silence_at_edges): a span starting ≤ this from 0
/// counts as "at the start"; symmetric at the tail.
const EDGE_SNAP_MS: u64 = 100;

/// Inputs measured from the RENDERED OUTPUT file (server runs the
/// instruments/ffprobe over the render before checking).
#[derive(Debug, Clone)]
pub struct RenderFacts {
    /// Measured output duration, ms (ffprobe).
    pub duration_ms: u64,
    /// Loudness of the output (ebur128).
    pub loudness: Option<Loudness>,
    /// Perception run over the OUTPUT (silences at edges, black/frozen).
    pub output_report: Option<PerceptionReport>,
}

// ---------------------------------------------------------------------------
// Footage profiles (the footage-profile contract) — same facts, profile-aware interpretation.
// ---------------------------------------------------------------------------

/// Editorial profile the check battery interprets facts under.
///
/// A correct SILENT screen-demo render fails 4/6 talking-head checks BY
/// Profile signature (−70 LUFS, zero captions, all-silent edges, freezedetect spans on
/// static UI). A receipt that fails on a correct render trains operators to
/// ignore receipts — the one outcome receipts exist to prevent. Profiles fix
/// the interpretation, not the measurement: instruments stay untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FootageProfile {
    /// Spoken-footage expectations — the original battery, unchanged.
    TalkingHead,
    /// Silent-by-design screen recording (UI demos, screencasts): loudness /
    /// caption / edge-silence targets waived (recorded, never dropped);
    /// frozen-frame check switches to the UI-tuned variant.
    SilentScreenDemo,
}

impl FootageProfile {
    /// Wire name (verbs.json arg value / receipt string).
    pub fn as_str(self) -> &'static str {
        match self {
            FootageProfile::TalkingHead => "talking_head",
            FootageProfile::SilentScreenDemo => "silent_screen_demo",
        }
    }
}

impl std::str::FromStr for FootageProfile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "talking_head" => Ok(FootageProfile::TalkingHead),
            "silent_screen_demo" => Ok(FootageProfile::SilentScreenDemo),
            other => Err(format!(
                "unknown footage profile '{other}' — valid: talking_head, silent_screen_demo"
            )),
        }
    }
}

/// Receipt entry name for the profile metadata record emitted by
/// `run_all_with_profile` (always pass=true — it documents, never gates).
/// Candidate for cut_core::check_names once the server wires the arg.
pub const FOOTAGE_PROFILE_CHECK: &str = "footage_profile";

/// A frozen span covering at least this fraction of the render is treated as
/// a STUCK RENDER even under the screen-demo profile (wall-to-wall identical
/// frames = the output itself is broken, not just static UI).
pub const STUCK_RENDER_MIN_FRACTION: f64 = 0.95;

/// Auto-detected profile suggestion. NEVER applied automatically — recorded
/// in the receipt's `footage_profile` entry for the operator/agent to adopt
/// explicitly (the footage-profile contract proposes, but never silently applies).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileProposal {
    pub profile: FootageProfile,
    /// Human-auditable conditions that fired, with the measured numbers.
    pub reasons: Vec<String>,
}

/// Convert a measured check into a profile waiver: pass becomes true, the
/// measured outcome and evidence are PRESERVED, and the waiver is recorded
/// in details (`waived_by_profile`, `waiver_reason`, `measured_pass`).
/// Unconditional by design: under the profile the check is non-gating
/// whatever it measured — predictable contract, zero hidden branches.
pub(crate) fn waive(mut check: CheckResult, profile: FootageProfile, reason: &str) -> CheckResult {
    let measured_pass = check.pass;
    // details is always a JSON object for our checks; wrap defensively if not.
    if !check.details.is_object() {
        check.details = json!({ "measured_details": check.details });
    }
    let d = check.details.as_object_mut().expect("object ensured above");
    d.insert("waived_by_profile".into(), json!(profile.as_str()));
    d.insert("waiver_reason".into(), json!(reason));
    d.insert("measured_pass".into(), json!(measured_pass));
    check.pass = true;
    check
}

/// UI-tuned black_or_frozen_frames variant for `silent_screen_demo`.
///
/// Black spans still FAIL — black output is a defect in any profile. Frozen
/// spans: static UI is legitimately pixel-static between interactions, so a
/// span fails only when it covers ≥ STUCK_RENDER_MIN_FRACTION of the render
/// (stuck output); everything else is waived WITH the spans listed.
///
/// Why not retune freezedetect instead (investigated on the regression
/// render: n=-60dB → 11 spans, n=-90dB → 7, n=0 (bit-identical
/// only) → 0, d=10s → 0. The spans are static UI with sub-−60dB encoder
/// noise — and after ANY re-encode a real stuck-frame defect also picks up
/// I-frame requantization noise, so no noise threshold separates "static UI"
/// from "stuck render" reliably. Detection stays camera-tuned in the sidecar
/// (facts are facts); interpretation moves here.
///
/// RESIDUAL RISK (honest): a partial stuck-frame defect (< the stuck
/// fraction) is NOT caught under this profile — it is indistinguishable from
/// legitimate static UI at the pixel level. The judge layer (verify.judge)
/// covers that perceptually; the waived spans stay in evidence for it.
pub fn black_or_frozen_frames_screen_demo(facts: &RenderFacts) -> CheckResult {
    let Some(report) = &facts.output_report else {
        return CheckResult {
            name: check_names::BLACK_OR_FROZEN_FRAMES.into(),
            pass: false,
            details: json!({"error": "no perception report for the output — run instruments (scenes) on the render"}),
            evidence: json!({"black_spans": null, "frozen_spans": null}),
        };
    };
    let dur = facts.duration_ms.max(1) as f64;
    let (stuck, waived): (Vec<_>, Vec<_>) = report.frozen_spans.iter().partition(|s| {
        (s.end_ms.saturating_sub(s.start_ms)) as f64 / dur >= STUCK_RENDER_MIN_FRACTION
    });
    let pass = report.black_spans.is_empty() && stuck.is_empty();
    CheckResult {
        name: check_names::BLACK_OR_FROZEN_FRAMES.into(),
        pass,
        details: json!({
            "variant": "silent_screen_demo (UI-tuned)",
            "black_span_count": report.black_spans.len(),
            "stuck_span_count": stuck.len(),
            "waived_frozen_span_count": waived.len(),
            "frozen_span_policy": format!(
                "static UI is legitimate in screen-demo footage; a frozen span fails only when it covers >= {:.0}% of the render (stuck output)",
                STUCK_RENDER_MIN_FRACTION * 100.0),
            "residual_risk": "a partial stuck-frame defect below the stuck fraction is indistinguishable from legitimate static UI post-encode (freezedetect n=0 finds 0 spans on defect-free UI footage; n=-60dB finds the same spans) — verify.judge covers it perceptually",
            "detector_thresholds": {"blackdetect_min_s": 0.3, "freezedetect_min_s": 2.0},
        }),
        evidence: json!({
            "black_spans": report.black_spans,
            "stuck_frozen_spans": stuck,
            "waived_frozen_spans": waived,
        }),
    }
}

/// Auto-detect: does the OUTPUT look like a silent screen demo?
///
/// Fires only when ALL of: no speech (0 transcript words in the output
/// report AND in every source transcript), silent (silence covers ≥ 90% of
/// the duration OR integrated ≤ −50 LUFS), low motion (frozen spans cover
/// ≥ 40% of the duration — the static-UI signature; the regression
/// render measured 86%). Returns the conditions WITH numbers so the receipt
/// proposal is auditable. PROPOSES only — never applied automatically.
pub fn propose_profile(
    transcripts: &[&Transcript],
    facts: &RenderFacts,
) -> Option<ProfileProposal> {
    let report = facts.output_report.as_ref()?;
    let dur = facts.duration_ms.max(1);
    let dur_f = dur as f64;

    let output_words = report.words.as_ref().map_or(0, |t| t.words.len());
    let source_words: usize = transcripts.iter().map(|t| t.words.len()).sum();
    if output_words + source_words > 0 {
        return None; // speech present — talking-head territory
    }

    let silence_ms: u64 = report
        .silences
        .iter()
        .map(|s| s.end_ms.min(dur).saturating_sub(s.start_ms))
        .sum();
    let silence_frac = silence_ms as f64 / dur_f;
    let lufs = report.loudness.as_ref().map(|l| l.integrated_lufs);
    let silent = silence_frac >= 0.9 || lufs.is_some_and(|l| l <= -50.0);
    if !silent {
        return None;
    }

    let frozen_ms: u64 = report
        .frozen_spans
        .iter()
        .map(|s| s.end_ms.min(dur).saturating_sub(s.start_ms))
        .sum();
    let frozen_frac = frozen_ms as f64 / dur_f;
    if frozen_frac < 0.4 {
        return None; // moving picture — not the static-UI signature
    }

    Some(ProfileProposal {
        profile: FootageProfile::SilentScreenDemo,
        reasons: vec![
            "no speech: 0 transcript words in output and sources".into(),
            format!(
                "silent: silence covers {:.0}% of {} ms{}",
                silence_frac * 100.0,
                dur,
                lufs.map_or(String::new(), |l| format!(", integrated {l} LUFS"))
            ),
            format!(
                "low motion: frozen spans cover {:.0}% of the duration",
                frozen_frac * 100.0
            ),
        ],
    })
}

/// The receipt's profile metadata record: which profile interpreted the
/// battery, how it was selected, and what auto-detect proposes. pass=true
/// always — this entry documents, it never gates the receipt.
fn footage_profile_record(
    active: FootageProfile,
    explicit: bool,
    proposal: Option<&ProfileProposal>,
) -> CheckResult {
    CheckResult {
        name: FOOTAGE_PROFILE_CHECK.into(),
        pass: true,
        details: json!({
            "active_profile": active.as_str(),
            "selection": if explicit { "explicit" } else { "default" },
            "proposed_profile": proposal.map(|p| p.profile.as_str()),
            "note": "auto-detect only PROPOSES a profile; switching requires the explicit profile arg",
        }),
        evidence: json!({
            "proposal_reasons": proposal.map(|p| p.reasons.clone()),
        }),
    }
}

/// cut_on_word: no EDL cut boundary lands INSIDE a word.
///
/// Word timestamps are SOURCE-time (whisperX runs on the source asset), so
/// the comparison happens in source coordinates: every media segment's
/// `src_in_ms`/`src_out_ms` is a cut into that asset, and must not fall
/// strictly inside any word span of that asset's transcript (± tolerance:
/// a cut within CUT_ON_WORD_TOLERANCE_MS of a word edge still passes).
///
/// Seamless splits are exempt: when two segments are adjacent on the
/// timeline AND contiguous in source (same asset, prev.src_out == next.src_in),
/// the shared boundary produces no audible/visible cut and is skipped.
///
/// `transcripts` = source-asset transcripts (matched by Transcript::asset).
/// Assets used in the EDL that have no transcript are reported as unchecked.
pub fn cut_on_word(edl: &Edl, transcripts: &[&Transcript]) -> CheckResult {
    let mut violations: Vec<serde_json::Value> = Vec::new();
    let mut checked_boundaries = 0usize;
    let mut unchecked_assets: Vec<String> = Vec::new();
    let mut skipped_overlay_tracks: Vec<String> = Vec::new();

    // Per-track walk so we can detect seamless (source-contiguous) joins.
    // EDL groups segments per track in order, so dedup() yields the track set.
    let track_ids: Vec<String> = {
        let mut ids: Vec<String> = edl.segments.iter().map(|s| s.track.clone()).collect();
        ids.dedup();
        ids
    };

    for track in &track_ids {
        // Editorial scope (compositing flag: only AUDIO-BEARING
        // tracks are checked — audio tracks + the base video track. OVERLAY
        // video tracks (2nd) composite PiP above the base and never cut the
        // program audio; their boundaries landing mid-word is by design, not
        // a truncated word. Skipped overlays are listed in details (honesty:
        // visible gap, never a silent drop).
        if !edl.is_audio_bearing_track(track) {
            if edl.track_segments(track).any(|s| s.asset.is_some()) {
                skipped_overlay_tracks.push(track.clone());
            }
            continue;
        }
        let segs: Vec<_> = edl
            .track_segments(track)
            .filter(|s| s.asset.is_some())
            .collect();
        for (i, seg) in segs.iter().enumerate() {
            let asset = seg.asset.as_deref().unwrap_or_default();
            let Some(transcript) = transcripts.iter().find(|t| t.asset == asset) else {
                if !unchecked_assets.iter().any(|a| a == asset) {
                    unchecked_assets.push(asset.to_string());
                }
                continue;
            };
            // Boundary exemptions for seamless joins with the neighbor segments.
            let seamless_with_prev = i > 0 && {
                let p = segs[i - 1];
                p.asset == seg.asset
                    && p.src_out_ms == seg.src_in_ms
                    && p.timeline_out_ms == seg.timeline_in_ms
            };
            let seamless_with_next = i + 1 < segs.len() && {
                let n = segs[i + 1];
                n.asset == seg.asset
                    && seg.src_out_ms == n.src_in_ms
                    && seg.timeline_out_ms == n.timeline_in_ms
            };
            for (boundary, cut_src_ms, skip) in [
                ("src_in", seg.src_in_ms, seamless_with_prev),
                ("src_out", seg.src_out_ms, seamless_with_next),
            ] {
                let (Some(cut), false) = (cut_src_ms, skip) else {
                    continue;
                };
                checked_boundaries += 1;
                // Violation: cut strictly inside a word, > tolerance from both edges.
                if let Some(w) = transcript.words.iter().find(|w| {
                    cut > w.start_ms.saturating_add(CUT_ON_WORD_TOLERANCE_MS)
                        && cut < w.end_ms.saturating_sub(CUT_ON_WORD_TOLERANCE_MS)
                }) {
                    violations.push(json!({
                        "track": seg.track, "clip_id": seg.clip_id, "boundary": boundary,
                        "cut_src_ms": cut, "word": w.word, "word_idx": w.idx,
                        "word_span_ms": [w.start_ms, w.end_ms],
                    }));
                }
            }
        }
    }

    CheckResult {
        name: check_names::CUT_ON_WORD.into(),
        pass: violations.is_empty(),
        details: json!({
            "checked_boundaries": checked_boundaries,
            "tolerance_ms": CUT_ON_WORD_TOLERANCE_MS,
            "unchecked_assets": unchecked_assets,
            "violation_count": violations.len(),
            // Overlay video tracks with media clips, exempt by design (PiP
            // compositing never cuts program audio).
            "skipped_overlay_tracks": skipped_overlay_tracks,
        }),
        evidence: json!({ "violations": violations }),
    }
}

/// Default tolerance for "a cut lands on a beat" (ms). A frame at 30fps ≈ 33ms;
/// a perceptually tight music cut sits within ~1 frame of the beat.
pub const CUT_ON_BEAT_TOLERANCE_MS: u64 = 50;

/// cut_on_beat measures how close each PROGRAM cut in a music-driven edit (base
/// video track segment boundary, timeline coords) lands to a beat of the placed
/// music. Beats are detected in the music ASSET's source time; they are mapped
/// to TIMELINE time through each music clip's placement (timeline_in − src_in),
/// so a music clip trimmed/moved still aligns. For every cut, the nearest-beat
/// distance is recorded.
///
/// This is a MEASUREMENT receipt, not a gate: it NEVER fails a non-beat-aligned
/// edit (a talking-head cut isn't meant to land on a music beat) — `pass` stays
/// true and `details.beat_aligned` (median within tolerance) + the distance
/// histogram let the agent/user judge tightness. The caller appends it ONLY for
/// a MUSIC-BED edit (a beat-bearing asset on a non-base audio track) — librosa
/// finds incidental beats in speech too, so gating on "any beat grid" would tack
/// it onto every talking-head receipt as noise.
///
/// `beats_by_asset` = (asset_id, beat grid) for the placed music-bed assets.
pub fn cut_on_beat(
    edl: &Edl,
    beats_by_asset: &[(String, BeatGrid)],
    tolerance_ms: u64,
) -> CheckResult {
    // 1. Map every placed music clip's beats from source → timeline coords.
    let mut timeline_beats: Vec<u64> = Vec::new();
    let mut bpm: Option<f32> = None;
    for seg in &edl.segments {
        let Some(asset) = seg.asset.as_deref() else {
            continue;
        };
        let Some((_, grid)) = beats_by_asset.iter().find(|(a, _)| a == asset) else {
            continue;
        };
        let (Some(src_in), Some(src_out)) = (seg.src_in_ms, seg.src_out_ms) else {
            continue;
        };
        bpm.get_or_insert(grid.bpm);
        for &b in &grid.beats_ms {
            if b >= src_in && b <= src_out {
                // Beat is a source position in the music asset; map to timeline
                // through the clip's speed (identity at 1.0).
                timeline_beats
                    .push(seg.timeline_in_ms + cut_core::src_off_to_tl(b - src_in, seg.speed));
            }
        }
    }
    timeline_beats.sort_unstable();
    timeline_beats.dedup();

    // 2. Program cut points = base-video-track segment boundaries (timeline),
    // excluding the timeline start (0) and the very end — those are the program
    // edges, not editorial cuts between shots.
    let mut cuts: Vec<u64> = Vec::new();
    if let Some(bv) = edl.base_video_track() {
        for seg in edl.track_segments(bv).filter(|s| s.asset.is_some()) {
            cuts.push(seg.timeline_in_ms);
            cuts.push(seg.timeline_out_ms);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let program_end = cuts.last().copied().unwrap_or(0);
    cuts.retain(|&c| c != 0 && c != program_end);

    // 3. Nearest-beat distance per cut.
    let mut distances: Vec<u64> = Vec::new();
    for &c in &cuts {
        if let Some(d) = timeline_beats.iter().map(|&b| b.abs_diff(c)).min() {
            distances.push(d);
        }
    }
    let on_beat = distances.iter().filter(|&&d| d <= tolerance_ms).count();
    let median = {
        let mut s = distances.clone();
        s.sort_unstable();
        s.get(s.len() / 2).copied()
    };
    let max = distances.iter().max().copied();
    // "beat_aligned": the typical cut is within tolerance — a signal the edit was
    // cut TO the music (not a pass/fail; advisory).
    let beat_aligned = !distances.is_empty() && median.is_some_and(|m| m <= tolerance_ms);

    CheckResult {
        name: check_names::CUT_ON_BEAT.into(),
        pass: true, // measurement receipt — never fails a non-beat-aligned edit
        details: json!({
            "bpm": bpm,
            "timeline_beats": timeline_beats.len(),
            "cut_count": distances.len(),
            "on_beat_count": on_beat,
            "tolerance_ms": tolerance_ms,
            "median_distance_ms": median,
            "max_distance_ms": max,
            "beat_aligned": beat_aligned,
        }),
        evidence: json!({ "cut_to_beat_distances_ms": distances }),
    }
}

/// bed_duck_under_speech — the verifiable-edit measurement set, AUDIO side. For a music-bed
/// edit with recorded DUCK windows (a `gain_window` that REDUCES the bed, db < 0),
/// measure how much of the ducked time lands ON speech. A good music bed ducks
/// UNDER the talk: the dip should sit on spoken words, not on silence. We map the
/// transcript words to TIMELINE coords through the audio-bearing EDL segments
/// (the same source→timeline path the cut checks use), merge them into speech
/// spans, and intersect each duck window with that speech.
///
/// MEASUREMENT receipt (never fails — like cut_on_beat): `pass` stays true and the
/// agent/user reads `details.ducked_over_speech_pct` (and `lands_on_speech`) to
/// judge whether the ducking is doing its job. A LOW percentage flags a duck that
/// missed the talk (wrong track, or a window placed off the speech). The caller
/// appends it ONLY when
/// the edit actually contains a duck window.
///
/// `duck_windows` = `(track_id, [start,end], db)` for windows with db < 0.
pub fn bed_duck_under_speech(
    edl: &Edl,
    transcripts: &[&Transcript],
    duck_windows: &[(String, [u64; 2], f64)],
) -> CheckResult {
    // 1. Speech spans in TIMELINE coords: map every transcript word through the
    //    audio-bearing EDL segments that carry its asset (clipped to the segment
    //    source window, scaled through the segment speed).
    let mut speech: Vec<(u64, u64)> = Vec::new();
    for seg in &edl.segments {
        if !edl.is_audio_bearing_track(&seg.track) {
            continue;
        }
        let (Some(asset), Some(src_in), Some(src_out)) =
            (seg.asset.as_deref(), seg.src_in_ms, seg.src_out_ms)
        else {
            continue;
        };
        let Some(t) = transcripts.iter().find(|t| t.asset == asset) else {
            continue;
        };
        for w in &t.words {
            let ws = w.start_ms.max(src_in);
            let we = w.end_ms.min(src_out);
            if we <= ws {
                continue; // word not inside this segment's source window
            }
            let tl_s = seg.timeline_in_ms + cut_core::src_off_to_tl(ws - src_in, seg.speed);
            let tl_e = seg.timeline_in_ms + cut_core::src_off_to_tl(we - src_in, seg.speed);
            speech.push((tl_s, tl_e));
        }
    }
    // Merge overlapping/adjacent speech spans so overlap is never double-counted.
    speech.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (s, e) in speech {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }

    // 2. Intersect each duck window with the merged speech spans.
    let total_duck: u64 = duck_windows
        .iter()
        .map(|(_, r, _)| r[1].saturating_sub(r[0]))
        .sum();
    let mut ducked_over_speech: u64 = 0;
    let mut per_window: Vec<serde_json::Value> = Vec::new();
    for (track, r, db) in duck_windows {
        let dur = r[1].saturating_sub(r[0]);
        let mut ov = 0u64;
        for (ss, se) in &merged {
            let lo = (*ss).max(r[0]);
            let hi = (*se).min(r[1]);
            if hi > lo {
                ov += hi - lo;
            }
        }
        ducked_over_speech += ov;
        let pct = if dur > 0 {
            (ov as f64 * 1000.0 / dur as f64).round() / 10.0
        } else {
            0.0
        };
        per_window.push(json!({
            "track": track, "range_ms": r, "db": db,
            "overlap_ms": ov, "overlap_pct": pct,
        }));
    }
    let pct = if total_duck > 0 {
        (ducked_over_speech as f64 * 1000.0 / total_duck as f64).round() / 10.0
    } else {
        0.0
    };
    CheckResult {
        name: check_names::BED_DUCK_UNDER_SPEECH.into(),
        pass: true, // measurement receipt — never fails an edit
        details: json!({
            "duck_window_count": duck_windows.len(),
            "ducked_ms": total_duck,
            "ducked_over_speech_ms": ducked_over_speech,
            "ducked_over_speech_pct": pct,
            "speech_span_count": merged.len(),
            // The typical music-bed duck should sit largely on speech; a low
            // value is the agent's cue the duck missed the talk.
            "lands_on_speech": total_duck > 0 && pct >= 60.0,
        }),
        evidence: json!({ "windows": per_window }),
    }
}

/// crossfade_smoothness — the verifiable-edit measurement set, VIDEO side. For each recorded
/// CROSSFADE seam (an EDL segment whose `xfade_in_ms > 0` dissolves from the
/// preceding segment on the same track), confirm a real dissolve of a sane length
/// sits at the seam — i.e. the transition is a smooth blend, not a hard cut that
/// was mislabeled, and not a degenerate (0 ms) or runaway (> the shorter clip)
/// dissolve. Structural / intent (EDL-only, no render needed): it surfaces every
/// crossfade + its duration in the receipt and flags any that are too short to
/// read as a dissolve. (A pixel-level luma-continuity measurement on the rendered
/// output is a heavier follow-up; this proves the seams are well-formed.)
///
/// MEASUREMENT receipt (pass:true); the caller appends it ONLY when at least one
/// crossfade exists (a hard-cut edit has none → not in the receipt).
pub fn crossfade_smoothness(edl: &Edl) -> CheckResult {
    const MIN_READABLE_MS: u64 = 80; // shorter than this barely reads as a blend
    let mut seams: Vec<serde_json::Value> = Vec::new();
    let mut too_short = 0usize;
    for seg in &edl.segments {
        let xf = seg.xfade_in_ms;
        if xf == 0 {
            continue;
        }
        let seg_len = seg.timeline_out_ms.saturating_sub(seg.timeline_in_ms);
        let degenerate = xf < MIN_READABLE_MS;
        if degenerate {
            too_short += 1;
        }
        seams.push(json!({
            "track": seg.track,
            "clip_id": seg.clip_id,
            "at_ms": seg.timeline_in_ms,
            "xfade_ms": xf,
            "segment_ms": seg_len,
            // A dissolve longer than its own segment can't fully play — flag it.
            "exceeds_segment": xf > seg_len,
            "readable": !degenerate,
        }));
    }
    CheckResult {
        name: check_names::CROSSFADE_SMOOTHNESS.into(),
        pass: true, // measurement receipt — never fails an edit
        details: json!({
            "crossfade_count": seams.len(),
            "too_short_count": too_short,
            "min_readable_ms": MIN_READABLE_MS,
            "all_readable": too_short == 0,
        }),
        evidence: json!({ "seams": seams }),
    }
}

/// Pacing report for the current timeline (`verify.pacing`): in-editor
/// retention/pacing critique. Pure
/// structural analysis of the base video track's shots (no model, no render):
/// shot count + mean/median/shortest/longest shot length, internal cut count,
/// and cuts-per-minute — the metrics a retention pass reasons about. Returned
/// as JSON by the verify.pacing verb (advisory; never a pass/fail).
pub fn pacing(edl: &Edl) -> serde_json::Value {
    let base_v = edl.base_video_track();
    let mut shots: Vec<u64> = base_v
        .map(|t| {
            edl.track_segments(t)
                .filter(|s| s.asset.is_some())
                .map(|s| s.timeline_out_ms.saturating_sub(s.timeline_in_ms))
                .collect()
        })
        .unwrap_or_default();
    let shot_count = shots.len();
    let total: u64 = shots.iter().sum();
    let mean = if shot_count > 0 {
        total / shot_count as u64
    } else {
        0
    };
    shots.sort_unstable();
    let median = shots.get(shot_count / 2).copied().unwrap_or(0);
    let shortest = shots.first().copied().unwrap_or(0);
    let longest = shots.last().copied().unwrap_or(0);
    let cuts = shot_count.saturating_sub(1); // internal cuts between shots
    let cuts_per_min = if edl.duration_ms > 0 {
        (cuts as f64 * 60_000.0 / edl.duration_ms as f64 * 100.0).round() / 100.0
    } else {
        0.0
    };
    json!({
        "duration_ms": edl.duration_ms,
        "shot_count": shot_count,
        "internal_cuts": cuts,
        "cuts_per_min": cuts_per_min,
        "mean_shot_ms": mean,
        "median_shot_ms": median,
        "shortest_shot_ms": shortest,
        "longest_shot_ms": longest,
    })
}

// =============================== verify.pregate ==============================
//
// PRE-render predictive quality gate (verify.pregate). pre-render ships a
// "pre-render gate + refuse-to-present" idea — don't pay for a render, and
// don't hand a render back as done, when the timeline is structurally likely to
// be broken. This makes that rigorous on OUR op-log: it reasons over the EDL
// geometry + the CACHED perception facts (no render, no model) and PREDICTS the
// render problems that have a known structural signature, BEFORE the render is
// spent.
//
// HONESTY — these are PREDICTIONS, not guarantees. The gate reasons over
// structure + cached facts, so:
//   - a clean pregate means "no KNOWN structural problem was found", NOT a proof
//     of a good render (it can't see anything the facts don't carry);
//   - a flagged risk is a strong predictor, not a certainty: an INTENTIONAL
//     audio-outro-over-black legitimately trips empty_tail, and a single long
//     talking-head take trips slideshow_risk. That is why severity matters:
//     HIGH means almost-certainly-wrong and blocks (pass=false); MED/LOW = worth a
//     human/agent glance, non-blocking.
// The point is to refuse to "present" a render as finished while a HIGH risk
// stands, and to do it for free before the render cost is paid.

/// Overlap length (ms) of the half-open ranges `[a0,a1)` and `[b0,b1)`.
fn span_overlap_ms(a0: u64, a1: u64, b0: u64, b1: u64) -> u64 {
    a1.min(b1).saturating_sub(a0.max(b0))
}

/// Tunable thresholds for [`pregate`]. Every value is defaulted + documented and
/// overridable per call (mirrors [`CaptionQcOpts`]) so a deliberately-unusual
/// timeline can relax/tighten the gate instead of fighting a hard-coded rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PregateOpts {
    /// empty_tail: a black VIDEO tail LONGER than this many ms past the last
    /// video clip is flagged. Sub-frame rounding below this is ignored (≈3
    /// frames @30fps); a real "plays into black" tail is seconds, not frames.
    pub empty_tail_tolerance_ms: u64,
    /// black_or_frozen: total black+frozen SOURCE ms a single clip may show
    /// before it is flagged. The sidecar only emits spans over its own
    /// blackdetect/freezedetect minimums, so every reported span is already
    /// real; this only suppresses an incidental sub-second dip.
    pub black_frozen_min_ms: u64,
    /// slideshow_risk: only evaluated for timelines at least this long — a 3 s
    /// bumper that is one static shot is not a "slideshow".
    pub slideshow_min_duration_ms: u64,
    /// slideshow_risk: cuts-per-minute AT OR BELOW this on a long timeline reads
    /// as a slideshow (the failure). 4 cpm = a cut every 15 s.
    pub slideshow_min_cpm: f64,
    /// silent_output: audio-bearing clips that are silent for AT LEAST this
    /// fraction of their measured timeline coverage flag a likely-silent render.
    pub silent_fraction: f64,
}

impl Default for PregateOpts {
    fn default() -> Self {
        Self {
            empty_tail_tolerance_ms: 100,
            black_frozen_min_ms: 500,
            slideshow_min_duration_ms: 20_000,
            slideshow_min_cpm: 4.0,
            silent_fraction: 0.90,
        }
    }
}

/// One predicted render risk (verify.pregate). `range_ms` localizes it on the
/// TIMELINE when one span applies (the black tail, the offending clip's slot),
/// so the agent can jump straight to the spot; `None` for whole-timeline risks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PregateRisk {
    /// Stable kind tag: `empty_tail` | `black_or_frozen` | `slideshow_risk` |
    /// `silent_output` | `tiny_or_zero_clips` | `uniform_border`.
    pub kind: String,
    /// `"high"` | `"med"` | `"low"` — HIGH blocks the gate, MED/LOW advise.
    pub severity: String,
    /// Human-readable explanation carrying the NUMBERS behind the verdict
    /// (the verify.* "read facts, not vibes" rule).
    pub detail: String,
    /// Timeline span [start,end) the risk localizes to, when one applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_ms: Option<[u64; 2]>,
}

/// verify.pregate result — a PRE-render predictive gate verdict.
#[derive(Debug, Clone, Serialize)]
pub struct PregateReport {
    /// `false` IFF any high-severity risk is present (the documented gate rule:
    /// only a HIGH risk — a class that almost always ships a broken render —
    /// blocks; MED/LOW are advisory).
    pub pass: bool,
    /// Every predicted risk, in evaluation order (HIGH kinds first).
    pub risks: Vec<PregateRisk>,
    /// One-line verdict for a human / agent log.
    pub summary: String,
    /// The thresholds actually applied (echoed for audit, like the other
    /// verify.* receipts).
    pub thresholds: serde_json::Value,
    /// Provenance: how many EDL-referenced assets had a CACHED perception
    /// report. The fact-dependent risks (black_or_frozen / silent_output /
    /// uniform_border) only see these.
    pub perception_assets: usize,
    /// EDL-referenced assets with NO cached perception report — the
    /// fact-dependent checks could not see these. Honesty: a clean pass over
    /// uninstrumented footage is WEAKER than one over analyzed footage, and a
    /// reader must be able to tell the two apart.
    pub uninstrumented_assets: Vec<String>,
}

/// pregate — the PRE-render predictive quality gate (verify.pregate). PURE +
/// deterministic: structural heuristics over the EDL + the cached
/// per-asset perception facts (`reports`: asset id → its
/// [`PerceptionReport`]; only assets that have one are present). No render, no
/// model, no I/O. See the module-level note above on why these are PREDICTIONS.
///
/// Risk kinds (severity):
///   - empty_tail (HIGH) — the timeline runs past the last VIDEO clip ⇒ the
///     program plays into black frames (a user-visible failure class). Only
///     evaluated when there IS a picture; an audio-only render is legitimately
///     black throughout and is a different concern, not a TAIL.
///   - black_or_frozen (HIGH) — a video clip's SOURCE range overlaps a detected
///     black/frozen span for its asset ⇒ dead footage baked into the cut.
///   - slideshow_risk (MED) — too few cuts over a long timeline (pre-render
///     "slideshow"): the base video track holds each shot too long.
///   - silent_output (MED) — the render is effectively silent: no audio-bearing
///     clip at all, OR audio-bearing clips that are mostly silence per facts.
///   - tiny_or_zero_clips (MED) — a media clip shorter than ONE frame (never
///     renders a visible frame) or zero-length (a no-op slot).
///   - uniform_border (LOW) — a clip's asset carries a baked-in
///     letterbox/pillarbox (the render ships black bands unless cropped).
///
/// `pass = false` iff any HIGH risk stands.
pub fn pregate(
    edl: &Edl,
    fps: f64,
    reports: &std::collections::BTreeMap<String, PerceptionReport>,
    opts: PregateOpts,
) -> PregateReport {
    let mut risks: Vec<PregateRisk> = Vec::new();
    let duration_ms = edl.duration_ms;

    // The picture = VIDEO-track media segments (asset.is_some() drops gaps).
    let video_segs: Vec<&cut_core::EdlSegment> = edl
        .segments
        .iter()
        .filter(|s| s.track_kind == TrackKind::Video && s.asset.is_some())
        .collect();

    // --- empty_tail (HIGH) -------------------------------------------------
    // The timeline (longest realized track end, incl. a longer audio bed or a
    // trailing gap) extends past the last VIDEO clip ⇒ black frames at the end.
    // Skip entirely when there is no picture at all (audio-only render).
    if let Some(last_video_end) = video_segs.iter().map(|s| s.timeline_out_ms).max() {
        let tail = duration_ms.saturating_sub(last_video_end);
        if tail > opts.empty_tail_tolerance_ms {
            risks.push(PregateRisk {
                kind: "empty_tail".into(),
                severity: "high".into(),
                detail: format!(
                    "the timeline is {duration_ms}ms but the last video clip ends at \
                     {last_video_end}ms — the final {tail}ms render as BLACK frames \
                     (a music/audio bed or a trailing gap outlasting the picture)"
                ),
                range_ms: Some([last_video_end, duration_ms]),
            });
        }
    }

    // --- black_or_frozen (HIGH) -------------------------------------------
    // A clip's SOURCE window overlaps a detected black/frozen span for its
    // asset ⇒ the composed frame is dead there. Spans are SOURCE-time; the
    // overlap maps onto the clip's timeline length by dividing by its speed
    // (forward playback assumed — see the doc note; reverse/ramps approximate).
    for s in &video_segs {
        let (Some(asset), Some(src_in), Some(src_out)) =
            (s.asset.as_deref(), s.src_in_ms, s.src_out_ms)
        else {
            continue;
        };
        let Some(rep) = reports.get(asset) else {
            continue;
        };
        let black_src: u64 = rep
            .black_spans
            .iter()
            .map(|sp| span_overlap_ms(sp.start_ms, sp.end_ms, src_in, src_out))
            .sum();
        let frozen_src: u64 = rep
            .frozen_spans
            .iter()
            .map(|sp| span_overlap_ms(sp.start_ms, sp.end_ms, src_in, src_out))
            .sum();
        let total_src = black_src + frozen_src;
        if total_src > opts.black_frozen_min_ms {
            let speed = if s.speed > 0.0 { s.speed } else { 1.0 };
            let tl_ms = (total_src as f64 / speed).round() as u64;
            let kind_word = match (black_src > 0, frozen_src > 0) {
                (true, true) => "black/frozen",
                (false, true) => "frozen",
                _ => "black",
            };
            risks.push(PregateRisk {
                kind: "black_or_frozen".into(),
                severity: "high".into(),
                detail: format!(
                    "clip {} (asset {asset}) shows ~{tl_ms}ms of {kind_word} frames \
                     ({black_src}ms black + {frozen_src}ms frozen of source) — dead \
                     footage in the cut",
                    s.clip_id.as_deref().unwrap_or("?")
                ),
                range_ms: Some([s.timeline_in_ms, s.timeline_out_ms]),
            });
        }
    }

    // --- slideshow_risk (MED) ---------------------------------------------
    // Too few cuts over a long timeline. Same base-video shot extraction as
    // `pacing`; only meaningful past the min-duration floor.
    if duration_ms >= opts.slideshow_min_duration_ms {
        let shots: Vec<u64> = edl
            .base_video_track()
            .map(|t| {
                edl.track_segments(t)
                    .filter(|s| s.asset.is_some())
                    .map(|s| s.timeline_out_ms.saturating_sub(s.timeline_in_ms))
                    .collect()
            })
            .unwrap_or_default();
        let shot_count = shots.len();
        if shot_count > 0 {
            let cuts = shot_count.saturating_sub(1);
            let cpm = cuts as f64 * 60_000.0 / duration_ms as f64;
            let mean = shots.iter().sum::<u64>() / shot_count as u64;
            if cpm <= opts.slideshow_min_cpm {
                risks.push(PregateRisk {
                    kind: "slideshow_risk".into(),
                    severity: "med".into(),
                    detail: format!(
                        "{shot_count} shot(s) over {duration_ms}ms = {cpm:.2} cuts/min \
                         (mean shot {mean}ms) — at or below {:.1} cuts/min the base video \
                         track reads as a slideshow; add cuts/motion or shorten the holds",
                        opts.slideshow_min_cpm
                    ),
                    range_ms: None,
                });
            }
        }
    }

    // --- silent_output (MED) ----------------------------------------------
    // An audio-bearing segment = a clip on an AUDIO track, or a VIDEO clip whose
    // asset report shows audio facts (loudness/words/silences present).
    let has_audio_facts = |asset: &str| -> bool {
        reports
            .get(asset)
            .map(|r| r.loudness.is_some() || r.words.is_some() || !r.silences.is_empty())
            .unwrap_or(false)
    };
    let audio_segs: Vec<&cut_core::EdlSegment> = edl
        .segments
        .iter()
        .filter(|s| match (s.asset.as_deref(), s.track_kind) {
            (Some(_), TrackKind::Audio) => true,
            (Some(a), TrackKind::Video) => has_audio_facts(a),
            _ => false,
        })
        .collect();
    if duration_ms > 0 {
        if audio_segs.is_empty() {
            // No audio-bearing clip anywhere ⇒ the render is silent. (Often a
            // real slip — forgot the VO/music; sometimes intentional silent
            // b-roll → MED, not HIGH.)
            risks.push(PregateRisk {
                kind: "silent_output".into(),
                severity: "med".into(),
                detail: "the timeline has no audio-bearing clip — the render will be \
                         entirely silent (add a VO/music track, or this is intentional \
                         silent footage)"
                    .into(),
                range_ms: None,
            });
        } else {
            // Audio exists: measure the silent fraction where we actually have
            // silence facts (segments without a report are assumed audible — we
            // never flag on missing facts).
            let mut covered_tl: u64 = 0; // audio-bearing timeline ms we could measure
            let mut silent_tl: u64 = 0; // of that, silent ms (mapped to timeline)
            for s in &audio_segs {
                let asset = s.asset.as_deref().unwrap();
                let Some(rep) = reports.get(asset) else {
                    continue;
                };
                if rep.silences.is_empty() && rep.loudness.is_none() && rep.words.is_none() {
                    continue; // no audio facts → unmeasured, not "silent"
                }
                let (Some(src_in), Some(src_out)) = (s.src_in_ms, s.src_out_ms) else {
                    continue;
                };
                let seg_src = src_out.saturating_sub(src_in);
                if seg_src == 0 {
                    continue;
                }
                let speed = if s.speed > 0.0 { s.speed } else { 1.0 };
                covered_tl += (seg_src as f64 / speed).round() as u64;
                let sil_src: u64 = rep
                    .silences
                    .iter()
                    .map(|sp| span_overlap_ms(sp.start_ms, sp.end_ms, src_in, src_out))
                    .sum();
                silent_tl += (sil_src as f64 / speed).round() as u64;
            }
            if covered_tl > 0 {
                let frac = silent_tl as f64 / covered_tl as f64;
                if frac >= opts.silent_fraction {
                    risks.push(PregateRisk {
                        kind: "silent_output".into(),
                        severity: "med".into(),
                        detail: format!(
                            "audio-bearing clips are ~{:.0}% silence ({silent_tl}ms of \
                             {covered_tl}ms measured) — the render is effectively silent",
                            frac * 100.0
                        ),
                        range_ms: None,
                    });
                }
            }
        }
    }

    // --- tiny_or_zero_clips (MED) -----------------------------------------
    // A media clip shorter than one frame never renders a visible frame; a
    // zero-length clip is a no-op slot. frame_ms from the project fps.
    let frame_ms = if fps > 0.0 {
        ((1000.0 / fps).round() as u64).max(1)
    } else {
        1
    };
    let tiny: Vec<&cut_core::EdlSegment> = edl
        .segments
        .iter()
        .filter(|s| s.asset.is_some())
        .filter(|s| s.timeline_out_ms.saturating_sub(s.timeline_in_ms) < frame_ms)
        .collect();
    if !tiny.is_empty() {
        let ids: Vec<String> = tiny
            .iter()
            .map(|s| s.clip_id.clone().unwrap_or_else(|| "?".into()))
            .collect();
        risks.push(PregateRisk {
            kind: "tiny_or_zero_clips".into(),
            severity: "med".into(),
            detail: format!(
                "{} clip(s) are shorter than one frame ({frame_ms}ms @ {:.0}fps) and \
                 won't render a visible frame: {}",
                tiny.len(),
                fps,
                ids.join(", ")
            ),
            range_ms: Some([tiny[0].timeline_in_ms, tiny[0].timeline_out_ms]),
        });
    }

    // --- uniform_border (LOW) ---------------------------------------------
    // A clip's asset carries a baked-in letterbox/pillarbox (cropdetect's
    // uniform_border flag) ⇒ the render ships black bands unless cropped. One
    // risk per distinct offending asset (the render's own uniform_border check
    // is the hard gate; this is the cheap PRE-render heads-up).
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for s in &video_segs {
        let asset = s.asset.as_deref().unwrap();
        let bordered = reports
            .get(asset)
            .and_then(|r| r.content_bbox.as_ref())
            .map(|b| b.uniform_border)
            .unwrap_or(false);
        if bordered && seen.insert(asset) {
            risks.push(PregateRisk {
                kind: "uniform_border".into(),
                severity: "low".into(),
                detail: format!(
                    "asset {asset} has a baked-in uniform border (letterbox/pillarbox) — \
                     edit.crop to its content bbox before render, or the output ships \
                     black bands"
                ),
                range_ms: Some([s.timeline_in_ms, s.timeline_out_ms]),
            });
        }
    }

    // --- verdict -----------------------------------------------------------
    let high = risks.iter().filter(|r| r.severity == "high").count();
    let advisory = risks.len() - high;
    let pass = high == 0;
    let summary = if risks.is_empty() {
        "pregate clean — no structural render risk predicted from the EDL + cached \
         perception facts (a PREDICTION, not a render-quality guarantee)"
            .to_string()
    } else if pass {
        format!(
            "pregate pass with {advisory} advisory risk(s) — none blocking, but worth a \
             look before render"
        )
    } else {
        let extra = if advisory > 0 {
            format!(" (+{advisory} advisory)")
        } else {
            String::new()
        };
        format!(
            "pregate FAIL — {high} high-severity risk(s){extra} likely to ship a broken \
             render; fix before spending the render"
        )
    };

    // --- provenance --------------------------------------------------------
    let referenced: std::collections::BTreeSet<&str> = edl
        .segments
        .iter()
        .filter_map(|s| s.asset.as_deref())
        .collect();
    let perception_assets = referenced
        .iter()
        .filter(|a| reports.contains_key(**a))
        .count();
    let uninstrumented_assets: Vec<String> = referenced
        .iter()
        .filter(|a| !reports.contains_key(**a))
        .map(|a| a.to_string())
        .collect();

    PregateReport {
        pass,
        risks,
        summary,
        thresholds: json!({
            "empty_tail_tolerance_ms": opts.empty_tail_tolerance_ms,
            "black_frozen_min_ms": opts.black_frozen_min_ms,
            "slideshow_min_duration_ms": opts.slideshow_min_duration_ms,
            "slideshow_min_cpm": opts.slideshow_min_cpm,
            "silent_fraction": opts.silent_fraction,
            "frame_ms": frame_ms,
        }),
        perception_assets,
        uninstrumented_assets,
    }
}

/// Caption-QC thresholds (verify.captions). Defaults are grounded in the
/// published timed-text standards, not invented:
///   - reading speed: BBC ≈15 CPS / Netflix English Timed-Text 17 CPS — we
///     default to the more permissive 17 so only genuinely-too-fast cues flag;
///   - min display: Netflix 5/6 s (833 ms) minimum on screen;
///   - max display: Netflix 7 s maximum;
///   - min gap between cues: 2 frames ≈ 80 ms (sub-frame gaps read as flicker);
///   - max length: 2 lines × 42 chars (BBC/Netflix line cap) = 84 chars.
/// All overridable per call so non-broadcast contexts can relax/tighten them.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CaptionQcOpts {
    pub max_cps: f64,
    pub min_duration_ms: u64,
    pub max_duration_ms: u64,
    pub min_gap_ms: u64,
    pub max_chars: usize,
}

impl Default for CaptionQcOpts {
    fn default() -> Self {
        Self {
            max_cps: 17.0,
            min_duration_ms: 833,
            max_duration_ms: 7000,
            min_gap_ms: 80,
            max_chars: 84,
        }
    }
}

/// First `n` chars of a cue for evidence, ellipsised — never dump the whole
/// caption into the receipt.
fn excerpt(text: &str, n: usize) -> String {
    let t = text.trim().replace('\n', " ");
    if t.chars().count() <= n {
        t
    } else {
        format!("{}…", t.chars().take(n).collect::<String>())
    }
}

/// caption_qc. Pure analysis over the
/// caption track's cues (text + ABSOLUTE timeline `range_ms`), measuring each
/// against the timed-text standards in [`CaptionQcOpts`]:
///   - too_fast: reading speed (chars incl. spaces / duration) exceeds max_cps;
///   - too_short / too_long: on-screen duration outside [min,max];
///   - too_long_text: character count exceeds the 2×42 line cap;
///   - short_gap: 0 < gap to the next cue < min_gap_ms (flicker);
///   - overlap: a cue starts before the previous one ends (also caught by the
///     caption-overlap render check; reported here for the authoring loop).
/// Returns a MEASUREMENT report (not a pass/fail render check): `pass` is true
/// only when every violation list is empty, and the thresholds are echoed so
/// the agent sees exactly what was applied. Empty caption track → an honest
/// `cue_count:0, pass:false` with a note (nothing to QC ≠ a clean pass).
pub fn caption_qc(cues: &[cut_core::CaptionClip], opts: CaptionQcOpts) -> serde_json::Value {
    if cues.is_empty() {
        return json!({
            "cue_count": 0,
            "pass": false,
            "note": "no caption cues to QC — run captions.generate first",
            "thresholds": opts,
        });
    }
    // Sort a working copy by start so gap/overlap detection is order-independent.
    let mut ordered: Vec<&cut_core::CaptionClip> = cues.iter().collect();
    ordered.sort_by_key(|c| c.range_ms[0]);

    let mut too_fast = Vec::new();
    let mut too_short = Vec::new();
    let mut too_long = Vec::new();
    let mut too_long_text = Vec::new();
    let mut short_gap = Vec::new();
    let mut overlap = Vec::new();
    let mut cps_values: Vec<f64> = Vec::new();

    for (i, c) in ordered.iter().enumerate() {
        let [start, end] = c.range_ms;
        let dur = end.saturating_sub(start);
        let chars = c.text.trim().chars().count();
        // CPS only meaningful for a positive-duration, non-empty cue.
        if dur > 0 && chars > 0 {
            let cps = (chars as f64) / (dur as f64 / 1000.0);
            let cps_r = (cps * 100.0).round() / 100.0;
            cps_values.push(cps_r);
            if cps > opts.max_cps {
                too_fast.push(json!({"id": c.id, "cps": cps_r, "chars": chars, "duration_ms": dur, "text": excerpt(&c.text, 32)}));
            }
        }
        if dur > 0 && dur < opts.min_duration_ms {
            too_short.push(json!({"id": c.id, "duration_ms": dur, "text": excerpt(&c.text, 32)}));
        }
        if dur > opts.max_duration_ms {
            too_long.push(json!({"id": c.id, "duration_ms": dur, "text": excerpt(&c.text, 32)}));
        }
        if chars > opts.max_chars {
            too_long_text.push(json!({"id": c.id, "chars": chars, "text": excerpt(&c.text, 32)}));
        }
        if i + 1 < ordered.len() {
            let next_start = ordered[i + 1].range_ms[0];
            if next_start < end {
                overlap.push(json!({"id": c.id, "next_id": ordered[i + 1].id, "overlap_ms": end - next_start}));
            } else {
                let gap = next_start - end;
                if gap > 0 && gap < opts.min_gap_ms {
                    short_gap
                        .push(json!({"id": c.id, "next_id": ordered[i + 1].id, "gap_ms": gap}));
                }
            }
        }
    }

    let max_cps = cps_values.iter().cloned().fold(0.0_f64, f64::max);
    let mean_cps = if cps_values.is_empty() {
        0.0
    } else {
        (cps_values.iter().sum::<f64>() / cps_values.len() as f64 * 100.0).round() / 100.0
    };
    let pass = too_fast.is_empty()
        && too_short.is_empty()
        && too_long.is_empty()
        && too_long_text.is_empty()
        && short_gap.is_empty()
        && overlap.is_empty();

    json!({
        "cue_count": cues.len(),
        "pass": pass,
        "max_cps": (max_cps * 100.0).round() / 100.0,
        "mean_cps": mean_cps,
        "violations": {
            "too_fast": too_fast,
            "too_short": too_short,
            "too_long": too_long,
            "too_long_text": too_long_text,
            "short_gap": short_gap,
            "overlap": overlap,
        },
        "thresholds": opts,
    })
}

/// Delivery thresholds (verify.delivery). Grounded in published speaking-rate
/// guidance for presentation/explainer video (not invented):
///   - ideal speech rate ~120–160 WPM; much above 160 risks losing the
///     audience, below ~100 reads as slow — we default the flag band to
///     [100, 170] (a small buffer past 160);
///   - filler density: a polished delivery keeps audible fillers low; >3 per
///     minute is noticeably distracting — default flag at 3.0/min.
/// `pause_gap_ms` is the inter-word gap above which speech is "broken" (used to
/// measure the longest unbroken stretch). All overridable per call.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DeliveryOpts {
    pub min_wpm: f64,
    pub max_wpm: f64,
    pub max_fillers_per_min: f64,
    pub pause_gap_ms: u64,
}

impl Default for DeliveryOpts {
    fn default() -> Self {
        Self {
            min_wpm: 100.0,
            max_wpm: 170.0,
            max_fillers_per_min: 3.0,
            pause_gap_ms: 600,
        }
    }
}

/// True if `word` (lowercased, non-alphanumerics stripped) is in `lexicon` —
/// the SAME normalization transcript.remove_fillers uses, so the counts agree.
fn is_filler_word(word: &str, lexicon: &[String]) -> bool {
    let clean: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    !clean.is_empty() && lexicon.contains(&clean)
}

/// delivery (verbal-pacing receipt — complements pacing()'s visual pacing).
/// Pure analysis over the SOURCE transcripts of `per_asset` (each inner slice
/// is one asset's word spans, so each contributes its own speaking window —
/// summing windows across assets is correct, concatenating words is not).
/// Measures, in aggregate:
///   - wpm: speech rate = words / speaking-window minutes (INCLUDES pauses —
///     this is the rate the published guidance targets);
///   - articulation_wpm: words / voiced minutes (EXCLUDES pauses);
///   - filler_count / fillers_per_min / filler_pct against `lexicon`;
///   - longest_unbroken_ms: longest run with no inter-word gap > pause_gap_ms.
/// Flags too_fast/too_slow/high_fillers against [`DeliveryOpts`]; pass=true only
/// when none fire. Measures the RAW recording's delivery (pre-cut) — labelled
/// `scope:"source"` so the agent knows it is not the post-edit timeline rate.
/// Empty/insufficient transcript → honest pass:false with a note.
pub fn delivery(
    per_asset: &[&[crate::types::WordSpan]],
    lexicon: &[String],
    opts: DeliveryOpts,
) -> serde_json::Value {
    let mut total_words = 0usize;
    let mut total_window_ms = 0u64;
    let mut total_voiced_ms = 0u64;
    let mut total_fillers = 0usize;
    let mut longest_unbroken_ms = 0u64;

    for words in per_asset {
        if words.is_empty() {
            continue;
        }
        total_words += words.len();
        let first = words.first().unwrap().start_ms;
        let last = words.last().unwrap().end_ms;
        total_window_ms += last.saturating_sub(first);
        total_voiced_ms += words
            .iter()
            .map(|w| w.end_ms.saturating_sub(w.start_ms))
            .sum::<u64>();
        total_fillers += words
            .iter()
            .filter(|w| is_filler_word(&w.word, lexicon))
            .count();
        // Longest unbroken stretch: extend while the gap to the next word is
        // within pause_gap_ms; reset on a longer pause.
        let mut run_start = words[0].start_ms;
        let mut prev_end = words[0].end_ms;
        for w in &words[1..] {
            if w.start_ms.saturating_sub(prev_end) > opts.pause_gap_ms {
                longest_unbroken_ms = longest_unbroken_ms.max(prev_end.saturating_sub(run_start));
                run_start = w.start_ms;
            }
            prev_end = w.end_ms;
        }
        longest_unbroken_ms = longest_unbroken_ms.max(prev_end.saturating_sub(run_start));
    }

    if total_words < 2 || total_window_ms == 0 {
        return json!({
            "scope": "source",
            "word_count": total_words,
            "pass": false,
            "note": "transcript too short to measure delivery (need spoken words with timestamps)",
            "thresholds": opts,
        });
    }

    let round2 = |x: f64| (x * 100.0).round() / 100.0;
    let wpm = round2(total_words as f64 / (total_window_ms as f64 / 60_000.0));
    let articulation_wpm = if total_voiced_ms > 0 {
        round2(total_words as f64 / (total_voiced_ms as f64 / 60_000.0))
    } else {
        0.0
    };
    let fillers_per_min = round2(total_fillers as f64 / (total_window_ms as f64 / 60_000.0));
    let filler_pct = round2(total_fillers as f64 / total_words as f64 * 100.0);

    let too_fast = wpm > opts.max_wpm;
    let too_slow = wpm < opts.min_wpm;
    let high_fillers = fillers_per_min > opts.max_fillers_per_min;
    let pass = !too_fast && !too_slow && !high_fillers;

    json!({
        "scope": "source",
        "word_count": total_words,
        "speaking_ms": total_window_ms,
        "wpm": wpm,
        "articulation_wpm": articulation_wpm,
        "filler_count": total_fillers,
        "fillers_per_min": fillers_per_min,
        "filler_pct": filler_pct,
        "longest_unbroken_ms": longest_unbroken_ms,
        "pass": pass,
        "flags": {"too_fast": too_fast, "too_slow": too_slow, "high_fillers": high_fillers},
        "thresholds": opts,
    })
}

/// Brand-constraint spec (verify.brand). Every field is OPTIONAL — only the
/// constraints the brand actually pins are checked; the rest pass vacuously.
/// The agent RESOLVES AGAINST the brand
/// instead of overwriting it, and the receipt PROVES conformance before render.
#[derive(Debug, Clone, Default)]
pub struct BrandSpec {
    /// Allowed caption fonts (case-insensitive exact match).
    pub fonts: Option<Vec<String>>,
    /// Allowed colours (text + bg), normalised hex (#rgb/#rgba/#rrggbb/#rrggbbaa).
    pub colors: Option<Vec<String>>,
    /// Required caption position keyword ("bottom"|"top"|"center").
    pub position: Option<String>,
    /// Caption font-size bounds (px).
    pub min_size: Option<u32>,
    pub max_size: Option<u32>,
    /// Required output aspect as a raw ratio (e.g. (16,9)); compared reduced
    /// against the project settings geometry.
    pub aspect: Option<(u32, u32)>,
}

/// Normalise a CSS-ish hex colour for comparison: lowercase, strip '#', and
/// expand 3/4-digit shorthand to 6/8 (so "#FFF" == "#ffffff", "#000a" ==
/// "#000000aa"). Non-hex strings pass through lowercased (best-effort match).
fn normalize_color(c: &str) -> String {
    let h = c.trim().trim_start_matches('#').to_lowercase();
    let expand = |s: &str| s.chars().flat_map(|ch| [ch, ch]).collect::<String>();
    match h.len() {
        3 | 4 => expand(&h),
        _ => h,
    }
}

/// Reduce a ratio by its gcd so 1920:1080 == 16:9 for comparison.
fn reduce_ratio(w: u32, h: u32) -> (u32, u32) {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a.max(1)
        } else {
            gcd(b, a % b)
        }
    }
    let g = gcd(w, h);
    (w / g, h / g)
}

/// brand_check: brand as an enforced constraint, not a default the AI
/// overwrites. Pure check of the project's
/// stored caption styles + output geometry against a [`BrandSpec`]. Per style,
/// flags: font not in the allowed list; text/bg colour off-palette; caption
/// position wrong; font size out of bounds. Project-level: output aspect wrong.
/// Returns a MEASUREMENT receipt — `pass` true only when every violation list is
/// empty; the spec is echoed so the agent sees exactly what was enforced. No
/// caption styles AND no aspect constraint → pass:true with a note (nothing the
/// brand pins is present to violate).
pub fn brand_check(
    styles: &std::collections::BTreeMap<String, cut_core::CaptionStyle>,
    settings: &cut_core::ProjectSettings,
    spec: &BrandSpec,
) -> serde_json::Value {
    let allowed_fonts: Option<Vec<String>> = spec
        .fonts
        .as_ref()
        .map(|v| v.iter().map(|f| f.to_lowercase()).collect());
    let allowed_colors: Option<Vec<String>> = spec
        .colors
        .as_ref()
        .map(|v| v.iter().map(|c| normalize_color(c)).collect());

    let mut font = Vec::new();
    let mut color = Vec::new();
    let mut position = Vec::new();
    let mut size = Vec::new();

    for (name, st) in styles {
        if let Some(fonts) = &allowed_fonts {
            if !fonts.contains(&st.font.to_lowercase()) {
                font.push(json!({"style": name, "font": st.font}));
            }
        }
        if let Some(palette) = &allowed_colors {
            for (slot, val) in [("color", Some(&st.color)), ("bg", st.bg.as_ref())] {
                if let Some(v) = val {
                    if !palette.contains(&normalize_color(v)) {
                        color.push(json!({"style": name, "slot": slot, "color": v}));
                    }
                }
            }
        }
        if let Some(want) = &spec.position {
            // Unstyled position defaults to "bottom" (the renderer default).
            let have = st.pos.as_deref().unwrap_or("bottom");
            if !have.eq_ignore_ascii_case(want) {
                position.push(json!({"style": name, "position": have, "expected": want}));
            }
        }
        if let Some(min) = spec.min_size {
            if st.size < min {
                size.push(json!({"style": name, "size": st.size, "min": min}));
            }
        }
        if let Some(max) = spec.max_size {
            if st.size > max {
                size.push(json!({"style": name, "size": st.size, "max": max}));
            }
        }
    }

    // Project-level: output aspect.
    let mut aspect_violation: Option<serde_json::Value> = None;
    if let Some((rw, rh)) = spec.aspect {
        let want = reduce_ratio(rw, rh);
        let have = reduce_ratio(settings.width, settings.height);
        if want != have {
            aspect_violation = Some(json!({
                "expected": format!("{}:{}", want.0, want.1),
                "actual": format!("{}:{}", have.0, have.1),
                "geometry": format!("{}x{}", settings.width, settings.height),
            }));
        }
    }

    let pass = font.is_empty()
        && color.is_empty()
        && position.is_empty()
        && size.is_empty()
        && aspect_violation.is_none();
    let nothing_to_check = styles.is_empty() && spec.aspect.is_none();

    json!({
        "styles_checked": styles.len(),
        "pass": pass && !nothing_to_check,
        "note": nothing_to_check.then_some("no caption styles and no aspect constraint to check"),
        "violations": {
            "font": font,
            "color": color,
            "position": position,
            "size": size,
            "aspect": aspect_violation,
        },
        "brand": {
            "fonts": spec.fonts,
            "colors": spec.colors,
            "position": spec.position,
            "min_size": spec.min_size,
            "max_size": spec.max_size,
            "aspect": spec.aspect.map(|(w, h)| format!("{w}:{h}")),
        },
    })
}

/// Below this the video and audio cuts count as ALIGNED (a straight cut).
pub const J_L_CUT_TOLERANCE_MS: u64 = 40;
/// A paired audio cut must be within this of the video cut to be the SAME
/// transition — a larger gap means the audio is just continuous (straight).
const J_L_PAIR_WINDOW_MS: u64 = 1000;

/// j_l_cut.
/// Classifies each transition where the base video and base audio cuts are
/// OFFSET: a J-cut (audio of the next shot leads the picture) vs an L-cut
/// (picture cuts first, audio of the previous shot lags), and measures the
/// lead/lag. STRUCTURAL — derived from the EDL alone, no diarization. For each
/// base-video cut, the nearest base-audio cut within a 1 s window is its pair:
/// |offset| ≤ tolerance → straight; audio earlier → J (audio_leads_ms); audio
/// later → L (video_leads_ms). A MEASUREMENT receipt (pass:true — an offset cut
/// is a deliberate technique, never a failure); the caller appends it only when
/// a J or L cut exists (an aligned edit has none → not in the receipt).
/// Approximation: nearest-cut pairing is a heuristic for offset transitions.
pub fn j_l_cut(edl: &Edl, tolerance_ms: u64) -> CheckResult {
    let cut_points = |track: Option<&str>| -> Vec<u64> {
        let Some(tid) = track else { return vec![] };
        let mut v: Vec<u64> = edl
            .track_segments(tid)
            .filter(|s| s.asset.is_some())
            .flat_map(|s| [s.timeline_in_ms, s.timeline_out_ms])
            .collect();
        v.sort_unstable();
        v.dedup();
        let end = v.last().copied().unwrap_or(0);
        v.retain(|&c| c != 0 && c != end); // program edges aren't editorial cuts
        v
    };
    let base_v = edl.base_video_track();
    let base_a = edl
        .segments
        .iter()
        .find(|s| s.track_kind == TrackKind::Audio)
        .map(|s| s.track.as_str());
    let v_cuts = cut_points(base_v);
    let a_cuts = cut_points(base_a);

    let (mut straight, mut j, mut l) = (0u64, 0u64, 0u64);
    let mut cuts: Vec<serde_json::Value> = Vec::new();
    for &vc in &v_cuts {
        match a_cuts.iter().min_by_key(|&&ac| ac.abs_diff(vc)) {
            Some(&ac) if ac.abs_diff(vc) <= J_L_PAIR_WINDOW_MS => {
                let offset = ac as i64 - vc as i64;
                if offset.unsigned_abs() <= tolerance_ms {
                    straight += 1;
                } else if offset < 0 {
                    j += 1;
                    cuts.push(json!({"at_ms": vc, "type": "J", "audio_leads_ms": -offset}));
                } else {
                    l += 1;
                    cuts.push(json!({"at_ms": vc, "type": "L", "video_leads_ms": offset}));
                }
            }
            // No paired audio cut within the window → audio runs through → straight.
            _ => straight += 1,
        }
    }
    CheckResult {
        name: check_names::J_L_CUT.into(),
        pass: true, // measurement receipt — an offset cut is a technique, not a fault
        details: json!({
            "transitions": v_cuts.len(),
            "straight": straight,
            "j_cuts": j,
            "l_cuts": l,
            "tolerance_ms": tolerance_ms,
        }),
        evidence: json!({ "cuts": cuts }),
    }
}

/// lufs: integrated LUFS within `target_lufs ± tolerance_lu` AND true peak
/// ≤ −1 dBTP (podcast/social norm, public verb contract). Fails honestly when no loudness
/// was measured.
pub fn lufs(facts: &RenderFacts, target_lufs: f64, tolerance_lu: f64) -> CheckResult {
    const MAX_TRUE_PEAK_DBTP: f64 = -1.0;
    let Some(l) = &facts.loudness else {
        return CheckResult {
            name: check_names::LUFS.into(),
            pass: false,
            details: json!({"error": "no loudness measured for the output — run the loudness instrument on the render"}),
            evidence: json!({"loudness": null}),
        };
    };
    let deviation = (l.integrated_lufs - target_lufs).abs();
    let lufs_ok = deviation <= tolerance_lu;
    let peak_ok = l.true_peak_dbtp <= MAX_TRUE_PEAK_DBTP;
    CheckResult {
        name: check_names::LUFS.into(),
        pass: lufs_ok && peak_ok,
        details: json!({
            "target_lufs": target_lufs, "tolerance_lu": tolerance_lu,
            "max_true_peak_dbtp": MAX_TRUE_PEAK_DBTP,
            "integrated_within_target": lufs_ok, "true_peak_ok": peak_ok,
        }),
        evidence: json!({
            "integrated_lufs": l.integrated_lufs,
            "true_peak_dbtp": l.true_peak_dbtp,
            "deviation_lu": deviation,
        }),
    }
}

/// caption_presence: the project carries a caption track with at least one
/// caption clip, and every caption range overlaps audio-bearing media on the
/// timeline (a caption floating over a gap/black is a defect).
///
/// Approximation note (recorded in details): "overlaps spoken words" is
/// approximated by overlap with AUDIO media segments — exact word overlap
/// belongs to cut_on_word's transcript domain; captions generated from the
/// transcript satisfy both definitions.
pub fn caption_presence(project: &Project, edl: &Edl) -> CheckResult {
    let caption_segs: Vec<_> = edl
        .segments
        .iter()
        .filter(|s| s.track_kind == TrackKind::Caption && s.caption_text.is_some())
        .collect();
    // Audio-bearing media ranges on the timeline (what a caption must overlap).
    // Uses the EDITORIAL audio-bearing notion (edl.is_audio_bearing_track):
    // audio tracks AND the base video track — its embedded audio is program
    // audio (the same track cut_on_word reasons about, and the track transcript
    // verbs ripple). Filtering on TrackKind::Audio ONLY false-FAILED captions
    // over a talking-head clip placed on the base video track with no separate
    // audio-track mirror. Overlay video + caption tracks stay
    // excluded (is_audio_bearing_track returns false for them).
    let audio_ranges: Vec<(u64, u64)> = edl
        .segments
        .iter()
        .filter(|s| s.asset.is_some() && edl.is_audio_bearing_track(&s.track))
        .map(|s| (s.timeline_in_ms, s.timeline_out_ms))
        .collect();

    let has_caption_track = project.tracks.iter().any(|t| t.kind == TrackKind::Caption);
    let mut orphans: Vec<serde_json::Value> = Vec::new();
    for seg in &caption_segs {
        let overlaps = audio_ranges
            .iter()
            .any(|(a_in, a_out)| seg.timeline_in_ms < *a_out && seg.timeline_out_ms > *a_in);
        let in_bounds =
            seg.timeline_in_ms < edl.duration_ms && seg.timeline_out_ms > seg.timeline_in_ms;
        if !overlaps || !in_bounds {
            orphans.push(json!({
                "clip_id": seg.clip_id, "range_ms": [seg.timeline_in_ms, seg.timeline_out_ms],
                "overlaps_audio": overlaps, "in_bounds": in_bounds,
                "text": seg.caption_text,
            }));
        }
    }
    // Caption overlap: two cues on the SAME caption track whose
    // timeline ranges overlap render stacked on top of each other. The model
    // allows it (caption ranges are absolute, not cumulative), so detect + fail.
    let mut overlaps: Vec<serde_json::Value> = Vec::new();
    for (i, a) in caption_segs.iter().enumerate() {
        for b in &caption_segs[i + 1..] {
            if a.track == b.track
                && a.timeline_in_ms < b.timeline_out_ms
                && b.timeline_in_ms < a.timeline_out_ms
            {
                overlaps.push(json!({
                    "track": a.track,
                    "a": {"clip_id": a.clip_id, "range_ms": [a.timeline_in_ms, a.timeline_out_ms]},
                    "b": {"clip_id": b.clip_id, "range_ms": [b.timeline_in_ms, b.timeline_out_ms]},
                }));
            }
        }
    }
    // Text sanity (caption-deduplication guard): doubled-word generation ("hello hello and
    // and welcome welcome") is invisible to coverage but trivially measurable
    // in the cue text. Aggregate ratio of adjacent IDENTICAL words (lowercase,
    // punctuation-stripped) across all cues; the doubling artifact sits at
    // ~0.5+, genuine speech well under 0.1 — threshold 0.25 with the measured
    // ratio always in details so a borderline case can be reasoned about.
    let norm = |w: &str| {
        w.to_lowercase()
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string()
    };
    let (mut pairs, mut repeats) = (0u64, 0u64);
    for seg in &caption_segs {
        let words: Vec<String> = seg
            .caption_text
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .map(&norm)
            .collect();
        for pair in words.windows(2) {
            if pair[0].is_empty() {
                continue; // pure-punctuation token — no honest comparison
            }
            pairs += 1;
            if pair[0] == pair[1] {
                repeats += 1;
            }
        }
    }
    const REPEATED_WORD_RATIO_MAX: f64 = 0.25;
    let repeated_word_ratio = if pairs > 0 {
        repeats as f64 / pairs as f64
    } else {
        0.0
    };
    let text_sane = repeated_word_ratio <= REPEATED_WORD_RATIO_MAX;
    let pass = has_caption_track
        && !caption_segs.is_empty()
        && orphans.is_empty()
        && text_sane
        && overlaps.is_empty();
    CheckResult {
        name: check_names::CAPTION_PRESENCE.into(),
        pass,
        details: json!({
            "has_caption_track": has_caption_track,
            "caption_clip_count": caption_segs.len(),
            "orphan_count": orphans.len(),
            "overlap_count": overlaps.len(),
            "speech_overlap_approximation": "overlap with audio media segments",
            "repeated_word_ratio": repeated_word_ratio,
            "repeated_word_ratio_max": REPEATED_WORD_RATIO_MAX,
        }),
        evidence: json!({
            "orphans": orphans,
            "overlaps": overlaps,
            "audio_ranges_ms": audio_ranges,
            "adjacent_word_pairs": pairs,
            "repeated_adjacent_pairs": repeats,
        }),
    }
}

/// black_or_frozen_frames: the OUTPUT report carries no detected black or
/// frozen video spans. Detection thresholds live in the sidecar invocation
/// (blackdetect d=0.3s, freezedetect d=2.0s — instruments.py "scenes"
/// instrument); anything the sidecar reported is by construction over
/// threshold, so the check asserts the lists are empty.
pub fn black_or_frozen_frames(facts: &RenderFacts) -> CheckResult {
    let Some(report) = &facts.output_report else {
        return CheckResult {
            name: check_names::BLACK_OR_FROZEN_FRAMES.into(),
            pass: false,
            details: json!({"error": "no perception report for the output — run instruments (scenes) on the render"}),
            evidence: json!({"black_spans": null, "frozen_spans": null}),
        };
    };
    let pass = report.black_spans.is_empty() && report.frozen_spans.is_empty();
    CheckResult {
        name: check_names::BLACK_OR_FROZEN_FRAMES.into(),
        pass,
        details: json!({
            "black_span_count": report.black_spans.len(),
            "frozen_span_count": report.frozen_spans.len(),
            "detector_thresholds": {"blackdetect_min_s": 0.3, "freezedetect_min_s": 2.0},
            // Detection-floor honesty: freezedetect
            // only flags holds ≥ 2.0s, so a sub-2s stuck region (e.g. a bad
            // concat seam) is below the detection floor and passes. A green
            // result means "no freeze ≥ 2.0s / no black ≥ 0.3s", not "every
            // frame moves."
            "detection_floor_note": "freezes shorter than freezedetect_min_s are below the detection floor",
        }),
        evidence: json!({
            "black_spans": report.black_spans,
            "frozen_spans": report.frozen_spans,
        }),
    }
}

/// uniform_border: the RENDERED output carries no
/// baked-in uniform-colour border (letterbox/pillarbox). cropdetect on the
/// render (the output perception report's content_bbox) reports the content
/// rectangle; the check FAILS when the content is inset by more than
/// UNIFORM_BORDER_MAX_INSET_PX on any edge.
///
/// WHY THIS GUARDS EVERY PROFILE (NOT waived by silent_screen_demo): the
/// real-world driver IS a silent screen demo — an OBS capture whose window
/// sat inside a larger canvas baked 56px black bands into the source, which
/// the editor rendered and receipt-passed undetected. Margins are MOST common
/// on screen recordings, not least; waiving this check on screen demos would
/// blind the receipt to the exact defect it exists to catch. So unlike the
/// loudness/caption/edge-silence checks (legitimately by-design on silent
/// footage), uniform_border stays gating on both profiles — see
/// run_all_with_profile: it is NOT in the waive() list for SilentScreenDemo.
///
/// HONEST SCOPE: cropdetect measures UNIFORM-colour bands. A border that is a
/// near-uniform-but-not-flat colour (e.g. a faint gradient) below the detector
/// limit is not caught here; the judge layer (verify.judge) covers perceptual
/// framing. A non-black solid border IS caught (cropdetect keys on luma
/// uniformity, not on black specifically).
///
/// Fails honestly when no content_bbox was measured (the output report had no
/// video, or the scenes/cropdetect instrument did not run on the render).
pub fn uniform_border(facts: &RenderFacts) -> CheckResult {
    let bbox: Option<&ContentBbox> = facts
        .output_report
        .as_ref()
        .and_then(|r| r.content_bbox.as_ref());
    let Some(b) = bbox else {
        return CheckResult {
            name: check_names::UNIFORM_BORDER.into(),
            pass: false,
            details: json!({
                "error": "no content_bbox measured for the output — run the scenes/cropdetect instrument on the render",
                "max_inset_px": UNIFORM_BORDER_MAX_INSET_PX,
            }),
            evidence: json!({"content_bbox": null}),
        };
    };
    // Inset on each edge, in source px (the render's own geometry).
    let left = b.x;
    let top = b.y;
    let right = b.frame_width.saturating_sub(b.x + b.width);
    let bottom = b.frame_height.saturating_sub(b.y + b.height);
    let max_inset = left.max(top).max(right).max(bottom);
    // Pass = no edge inset beyond the jitter tolerance. b.uniform_border is
    // the detector's own verdict; we re-derive from the geometry so the check
    // is robust to a stale flag and the evidence is fully auditable.
    let pass = max_inset <= UNIFORM_BORDER_MAX_INSET_PX;
    CheckResult {
        name: check_names::UNIFORM_BORDER.into(),
        pass,
        details: json!({
            "max_inset_px": UNIFORM_BORDER_MAX_INSET_PX,
            "detector_uniform_border": b.uniform_border,
            "measured_max_inset_px": max_inset,
            "policy": "the rendered output must fill the frame — a uniform-colour band on any edge (baked-in letterbox/pillarbox, e.g. an OBS canvas/window mismatch) fails; fix the SOURCE with edit.crop to its content_bbox, or render with fit:cover to crop-to-fill",
        }),
        evidence: json!({
            "frame": [b.frame_width, b.frame_height],
            "content_rect": [b.x, b.y, b.width, b.height],
            "inset_px": {"left": left, "top": top, "right": right, "bottom": bottom},
        }),
    }
}

/// silence_at_edges: output does not start or end with silence longer than
/// the padding budget (default 500ms). Uses the output report's silence
/// spans; a span counts as edge-touching when it begins within EDGE_SNAP_MS
/// of 0 (head) or ends within EDGE_SNAP_MS of the measured duration (tail).
pub fn silence_at_edges(facts: &RenderFacts, max_edge_silence_ms: u64) -> CheckResult {
    let Some(report) = &facts.output_report else {
        return CheckResult {
            name: check_names::SILENCE_AT_EDGES.into(),
            pass: false,
            details: json!({"error": "no perception report for the output — run instruments (silence) on the render"}),
            evidence: json!({"head_silence_ms": null, "tail_silence_ms": null}),
        };
    };
    // Span ends are clamped to the measured duration: instruments measure the
    // extracted wav, which can overrun the container by ~16-50 ms (regression
    // cosmetic: head_silence 46 016 ms on a 46 000 ms render reads as broken).
    // F-7b: take the LONGEST edge-touching span, not the first one found — if
    // the silero/silencedetect cross-check leaves two adjacent spans near an
    // edge, `.find()` could report the shorter half and under-state the edge
    // silence. `.max()` over all edge-touching spans is robust to that.
    let head = report
        .silences
        .iter()
        .filter(|s| s.start_ms <= EDGE_SNAP_MS)
        .map(|s| s.end_ms.min(facts.duration_ms).saturating_sub(s.start_ms))
        .max()
        .unwrap_or(0);
    let tail = report
        .silences
        .iter()
        .filter(|s| s.end_ms + EDGE_SNAP_MS >= facts.duration_ms)
        .map(|s| s.end_ms.min(facts.duration_ms).saturating_sub(s.start_ms))
        .max()
        .unwrap_or(0);
    let pass = head <= max_edge_silence_ms && tail <= max_edge_silence_ms;
    CheckResult {
        name: check_names::SILENCE_AT_EDGES.into(),
        pass,
        details: json!({
            "max_edge_silence_ms": max_edge_silence_ms,
            "edge_snap_ms": EDGE_SNAP_MS,
            // detector-visibility contract: the detector gate was invisible — a clearly audible
            // -28 LUFS music bed below the -35 dB silencedetect floor counted
            // as "silence" and the user couldn't reason about why (or how
            // much bed level would clear it). Numbers mirror instruments.py
            // (SILENCE_NOISE_DB / SILENCE_MIN_S — change BOTH places or
            // neither); silero contributes spans by ABSENCE OF SPEECH, so a
            // music-only edge can register as silence regardless of level.
            "detector": {
                "engines": "silero-vad (speech absence) + ffmpeg silencedetect",
                "silencedetect_noise_db": -35,
                "min_span_s": 0.3,
                "note": "spans below the -35 dB gate OR without detected speech count as silence — a quiet-but-audible music bed at the edge can therefore fail this check; raise the bed above -35 dB at the edge, trim the edge, or run the silent_screen_demo profile if silence is by design",
            },
        }),
        evidence: json!({
            "head_silence_ms": head,
            "tail_silence_ms": tail,
            "duration_ms": facts.duration_ms,
        }),
    }
}

/// duration_matches_edl: measured output duration == EDL duration, within the
/// larger of one rendered video frame or one output-audio sample.
pub fn duration_matches_edl(
    edl: &Edl,
    facts: &RenderFacts,
    fps: f64,
    audio_rate: u32,
) -> CheckResult {
    // One frame in ms, ceil — at 30fps that is 34ms. Guard fps<=0 → 1000ms.
    let frame_ms = if fps > 0.0 {
        (1000.0 / fps).ceil() as u64
    } else {
        1000
    };
    let sample_ms = if audio_rate > 0 {
        (1000.0 / audio_rate as f64).ceil() as u64
    } else {
        0
    };
    let tolerance_ms = frame_ms.max(sample_ms);
    let delta = facts.duration_ms.abs_diff(edl.duration_ms);
    CheckResult {
        name: check_names::DURATION_MATCHES_EDL.into(),
        pass: delta <= tolerance_ms,
        details: json!({
            "tolerance_ms": tolerance_ms,
            "fps": fps,
            "video_frame_tolerance_ms": frame_ms,
            "audio_rate": audio_rate,
            "audio_sample_tolerance_ms": sample_ms,
        }),
        evidence: json!({
            "edl_duration_ms": edl.duration_ms,
            "measured_duration_ms": facts.duration_ms,
            "delta_ms": delta,
            "expected_output_frames": cut_core::timeline_frame_count(edl.duration_ms, fps),
        }),
    }
}

/// Run the full verification battery in canonical order (public verb contract verify.checks).
/// `transcripts` are the source transcripts for assets used in the EDL.
/// Default profile: `talking_head` (original behavior); auto-detect may
/// PROPOSE a different profile in the appended `footage_profile` entry.
pub fn run_all(
    project: &Project,
    edl: &Edl,
    transcripts: &[&Transcript],
    facts: &RenderFacts,
    beats: &[(String, BeatGrid)],
) -> Vec<CheckResult> {
    run_all_with_profile(project, edl, transcripts, facts, beats, None)
}

/// Profile-aware battery (the footage-profile contract). `profile: None` = no explicit choice
/// → talking_head defaults; `Some(p)` = explicit operator/agent selection.
///
/// The six canonical checks keep their order; a 7th `footage_profile`
/// metadata entry (pass=true, never gates) is appended recording the active
/// profile, how it was selected, and any auto-detected proposal. Under
/// `silent_screen_demo`: lufs / caption_presence / silence_at_edges are
/// waived (details.waived_by_profile, measured outcome preserved) and
/// black_or_frozen_frames runs the UI-tuned variant.
pub fn run_all_with_profile(
    project: &Project,
    edl: &Edl,
    transcripts: &[&Transcript],
    facts: &RenderFacts,
    beats: &[(String, BeatGrid)],
    profile: Option<FootageProfile>,
) -> Vec<CheckResult> {
    let active = profile.unwrap_or(FootageProfile::TalkingHead);
    let proposal = propose_profile(transcripts, facts);
    let output = crate::output_checks_with_profile(facts, active);
    let captions = match active {
        FootageProfile::TalkingHead => caption_presence(project, edl),
        FootageProfile::SilentScreenDemo => waive(
            caption_presence(project, edl),
            active,
            "no speech — caption absence is correct for a silent screen demo",
        ),
    };
    // Keep the long-standing canonical receipt order. Only the four
    // rendered-output checks are delegated, so timeline-dependent checks keep
    // their original snapshot inputs here.
    let mut checks = vec![
        cut_on_word(edl, transcripts),
        output.lufs,
        captions,
        output.black_or_frozen_frames,
        output.uniform_border,
        output.silence_at_edges,
        duration_matches_edl(
            edl,
            facts,
            project.settings.fps,
            project.settings.audio_rate,
        ),
    ];
    // cut_on_beat: a measurement receipt appended ONLY when a placed asset
    // carries a beat grid (a music-driven edit) — talking-head footage has no
    // music, so it never clutters the common receipt. Inserted before the
    // footage_profile metadata so that record stays last.
    if !beats.is_empty() {
        checks.push(cut_on_beat(edl, beats, CUT_ON_BEAT_TOLERANCE_MS));
    }
    // j_l_cut: appended ONLY when the edit actually contains an offset
    // (J or L) cut — an aligned edit has none, so it never clutters the common
    // receipt. EDL-only, no extra inputs.
    let jl = j_l_cut(edl, J_L_CUT_TOLERANCE_MS);
    let has_jl = jl
        .details
        .get("j_cuts")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + jl.details
            .get("l_cuts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
        > 0;
    if has_jl {
        checks.push(jl);
    }
    // bed_duck_under_speech (measurement, audio side): appended ONLY when the edit
    // carries a DUCK window (a gain_window reducing the bed, db < 0) — a
    // talking-head cut with no music bed has none, so it never clutters the
    // common receipt. Gathers the duck windows from every track's gain_windows.
    let duck_windows: Vec<(String, [u64; 2], f64)> = project
        .tracks
        .iter()
        .flat_map(|t| {
            t.gain_windows
                .iter()
                .filter(|gw| gw.db < 0.0)
                .map(move |gw| (t.id.clone(), gw.range_ms, gw.db))
        })
        .collect();
    if !duck_windows.is_empty() {
        checks.push(bed_duck_under_speech(edl, transcripts, &duck_windows));
    }
    // crossfade_smoothness (measurement, video side): appended ONLY when at least one
    // crossfade seam exists (a hard-cut edit has none). EDL-only.
    let xfade = crossfade_smoothness(edl);
    if xfade
        .details
        .get("crossfade_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        > 0
    {
        checks.push(xfade);
    }
    checks.push(footage_profile_record(
        active,
        profile.is_some(),
        proposal.as_ref(),
    ));
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SilenceSpan, VideoSpan, WordSpan, PERCEPTION_SCHEMA};
    use cut_core::{edl_from_project, Clip, MediaClip, Project, ProjectSettings};

    /// Transcript with words at known source positions: "hello"(100-500),
    /// "world"(600-1200), "again"(1500-2000).
    fn transcript() -> Transcript {
        let words = [
            ("hello", 100u64, 500u64),
            ("world", 600, 1200),
            ("again", 1500, 2000),
        ];
        Transcript {
            asset: "a1".into(),
            model: "test".into(),
            language: Some("en".into()),
            words: words
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

    fn media_clip(id: &str, src_in: u64, src_out: u64) -> Clip {
        Clip::Media(MediaClip {
            id: id.into(),
            asset: "a1".into(),
            src_in_ms: src_in,
            src_out_ms: src_out,
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
        })
    }

    fn empty_report() -> PerceptionReport {
        PerceptionReport {
            schema: PERCEPTION_SCHEMA.into(),
            asset_hash: "sha256:test".into(),
            source_path: "/out.mp4".into(),
            instruments_run: vec!["silence".into(), "scenes".into(), "loudness".into()],
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

    #[test]
    fn cut_on_word_passes_on_word_boundaries() {
        // Cuts at 500 (end of "hello") and 1500 (start of "again") — both edges.
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 500));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c2", 1500, 2000));
        let edl = edl_from_project(&p);
        let t = transcript();
        let r = cut_on_word(&edl, &[&t]);
        assert!(r.pass, "boundary cuts must pass: {:?}", r.evidence);
    }

    #[test]
    fn bed_duck_under_speech_measures_overlap_with_talk() {
        // Put the talk on the base AUDIO track [0,2000] (source == timeline @1×).
        // transcript(): hello 100-500, world 600-1200, again 1500-2000 (source ms).
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 2000));
        let edl = edl_from_project(&p);
        let t = transcript();
        // Duck ON the talk (covers hello + world): high speech overlap.
        let on = bed_duck_under_speech(&edl, &[&t], &[("a1t".into(), [100, 1200], -12.0)]);
        assert!(on.pass);
        let pct = on.details["ducked_over_speech_pct"].as_f64().unwrap();
        assert!(
            pct > 80.0,
            "a duck over the talk should mostly overlap speech, got {pct}"
        );
        assert_eq!(on.details["lands_on_speech"], json!(true));
        // Duck in the SILENT gap (1200-1500) between "world" and "again": misses talk.
        let off = bed_duck_under_speech(&edl, &[&t], &[("a1t".into(), [1200, 1500], -12.0)]);
        assert_eq!(off.details["ducked_over_speech_ms"].as_u64().unwrap(), 0);
        assert_eq!(off.details["lands_on_speech"], json!(false));
    }

    #[test]
    fn crossfade_smoothness_surfaces_and_flags_seams() {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 1000));
        let mut c2 = media_clip("c2", 1000, 2000);
        if let Clip::Media(ref mut mc) = c2 {
            mc.xfade_in_ms = 500; // a readable dissolve
        }
        p.track_mut("v1").unwrap().clips.push(c2);
        let edl = edl_from_project(&p);
        let r = crossfade_smoothness(&edl);
        assert!(r.pass);
        assert_eq!(r.details["crossfade_count"].as_u64().unwrap(), 1);
        assert_eq!(r.details["all_readable"], json!(true));
        // A no-crossfade edit yields a zero-count receipt (caller drops it).
        let mut p2 = Project::new("t", ProjectSettings::default());
        p2.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 1000));
        assert_eq!(
            crossfade_smoothness(&edl_from_project(&p2)).details["crossfade_count"]
                .as_u64()
                .unwrap(),
            0
        );
    }

    #[test]
    fn cut_on_word_fails_inside_word() {
        // Cut at 900 — strictly inside "world" (600-1200), > 40ms from edges.
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 900));
        let edl = edl_from_project(&p);
        let t = transcript();
        let r = cut_on_word(&edl, &[&t]);
        assert!(!r.pass);
        assert_eq!(r.evidence["violations"][0]["word"], "world");
    }

    #[test]
    fn cut_on_word_tolerates_near_edge_and_seamless_splits() {
        // 630 is inside "world" but within 40ms of its 600 start → pass.
        // c2/c3 boundary at source 1700 is inside "again" BUT source-contiguous
        // (seamless split) → exempt.
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 630));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c2", 1500, 1700));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c3", 1700, 2100));
        let edl = edl_from_project(&p);
        let t = transcript();
        let r = cut_on_word(&edl, &[&t]);
        assert!(
            r.pass,
            "near-edge + seamless split must pass: {:?}",
            r.evidence
        );
    }

    #[test]
    fn cut_on_word_ignores_overlay_track_boundaries() {
        // Regression (compositing flag: a PiP overlay inserted
        // MID-WORD must NOT trip cut_on_word — overlay video tracks (2nd)
        // composite above the base and never cut program audio. The skipped
        // track is recorded in details (visible exemption, not a silent drop).
        use cut_core::{Track, TrackKind};
        let mut p = Project::new("t", ProjectSettings::default());
        // Base track cuts on word boundaries — clean.
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 500));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c2", 1500, 2000));
        // Overlay track: BOTH boundaries strictly inside "world" (600-1200),
        // > 40ms from the edges — would violate if the track were checked.
        p.tracks.push(Track {
            id: "v2".into(),
            kind: TrackKind::Video,
            clips: vec![media_clip("o1", 700, 1100)],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        let edl = edl_from_project(&p);
        let t = transcript();
        let r = cut_on_word(&edl, &[&t]);
        assert!(
            r.pass,
            "overlay mid-word boundaries must not trip: {:?}",
            r.evidence
        );
        assert_eq!(r.details["skipped_overlay_tracks"][0], "v2");
        // Only the base track's 4 boundaries were checked.
        assert_eq!(r.details["checked_boundaries"], 4);
        // Sanity: the SAME mid-word boundary on the BASE track still trips —
        // the exemption must not over-reach.
        let mut p2 = Project::new("t", ProjectSettings::default());
        p2.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 900));
        p2.tracks.push(Track {
            id: "v2".into(),
            kind: TrackKind::Video,
            clips: vec![media_clip("o1", 700, 1100)],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        let r2 = cut_on_word(&edl_from_project(&p2), &[&transcript()]);
        assert!(!r2.pass, "base-track mid-word cut must still fail");
        assert_eq!(r2.evidence["violations"][0]["word"], "world");
        assert_eq!(
            r2.details["violation_count"], 1,
            "overlay must contribute no violations"
        );
    }

    #[test]
    fn cut_on_word_reports_unchecked_assets() {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 900));
        let edl = edl_from_project(&p);
        let r = cut_on_word(&edl, &[]); // no transcripts at all
        assert!(r.pass); // vacuous, but explicitly:
        assert_eq!(r.details["unchecked_assets"][0], "a1");
        assert_eq!(r.details["checked_boundaries"], 0);
    }

    /// cut_on_beat: measures program-cut → nearest-beat distance, mapping
    /// the music asset's source-time beats to timeline through its placement.
    #[test]
    fn cut_on_beat_measures_and_maps_through_placement() {
        let music = |src_in: u64, src_out: u64| {
            Clip::Media(MediaClip {
                id: "mc1".into(),
                asset: "m1".into(),
                src_in_ms: src_in,
                src_out_ms: src_out,
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
            })
        };
        let beats = vec![(
            "m1".to_string(),
            BeatGrid {
                bpm: 120.0,
                beats_ms: vec![0, 1000, 2000, 3000, 4000],
            },
        )];

        // Music at timeline 0, src_in 0 → beat ms == timeline ms. The single
        // internal video cut at 2000 lands exactly on the 2000ms beat.
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 2000));
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c2", 0, 2000));
        p.track_mut("a1t").unwrap().clips.push(music(0, 4000));
        let r = cut_on_beat(&edl_from_project(&p), &beats, CUT_ON_BEAT_TOLERANCE_MS);
        assert!(r.pass, "measurement receipt never fails");
        assert_eq!(
            r.details["cut_count"], 1,
            "one internal cut (program edges 0/4000 excluded)"
        );
        assert_eq!(r.details["on_beat_count"], 1);
        assert_eq!(r.details["median_distance_ms"], 0);
        assert_eq!(r.details["beat_aligned"], true);

        // Same cut, but the music clip starts at src_in 300 → every beat maps
        // 300ms earlier on the timeline, so the 2000ms cut is now 300ms off the
        // nearest beat (1700) — proves the placement mapping, not raw source ms.
        let mut p2 = Project::new("t2", ProjectSettings::default());
        p2.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 2000));
        p2.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c2", 0, 2000));
        p2.track_mut("a1t").unwrap().clips.push(music(300, 4000));
        let r2 = cut_on_beat(&edl_from_project(&p2), &beats, CUT_ON_BEAT_TOLERANCE_MS);
        assert_eq!(
            r2.details["on_beat_count"], 0,
            "cut is off-beat after the placement shift"
        );
        assert_eq!(r2.details["median_distance_ms"], 300);
        assert_eq!(r2.details["beat_aligned"], false);
    }

    /// j_l_cut: an audio cut earlier than the video cut → J-cut (audio
    /// leads); aligned cuts → straight.
    #[test]
    fn j_l_cut_classifies_offset_vs_aligned() {
        let clip = |id: &str, dur: u64| {
            Clip::Media(MediaClip {
                id: id.into(),
                asset: "a1".into(),
                src_in_ms: 0,
                src_out_ms: dur,
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
            })
        };
        // v1 cuts at 5000 (5000+3000); a1t cuts at 4500 (4500+3500) — both end
        // at 8000, so audio leads the picture by 500 ms → a J-cut.
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .extend([clip("v1", 5000), clip("v2", 3000)]);
        p.track_mut("a1t")
            .unwrap()
            .clips
            .extend([clip("a1", 4500), clip("a2", 3500)]);
        let r = j_l_cut(&edl_from_project(&p), J_L_CUT_TOLERANCE_MS);
        assert!(r.pass);
        assert_eq!(r.details["j_cuts"], 1, "audio leads → J-cut");
        assert_eq!(r.details["l_cuts"], 0);
        assert_eq!(r.evidence["cuts"][0]["type"], "J");
        assert_eq!(r.evidence["cuts"][0]["audio_leads_ms"], 500);

        // Aligned cuts (both at 5000) → straight, no J/L.
        let mut p2 = Project::new("t2", ProjectSettings::default());
        p2.track_mut("v1")
            .unwrap()
            .clips
            .extend([clip("v1", 5000), clip("v2", 3000)]);
        p2.track_mut("a1t")
            .unwrap()
            .clips
            .extend([clip("a1", 5000), clip("a2", 3000)]);
        let r2 = j_l_cut(&edl_from_project(&p2), J_L_CUT_TOLERANCE_MS);
        assert_eq!(r2.details["straight"], 1);
        assert_eq!(r2.details["j_cuts"], 0);
        assert_eq!(r2.details["l_cuts"], 0);
    }

    /// verify.pacing: shot metrics + cuts-per-minute from the base video track.
    #[test]
    fn pacing_reports_shot_metrics() {
        let clip = |id: &str, dur: u64| {
            Clip::Media(MediaClip {
                id: id.into(),
                asset: "a1".into(),
                src_in_ms: 0,
                src_out_ms: dur,
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
            })
        };
        let mut p = Project::new("t", ProjectSettings::default());
        // 3 shots: 2000 / 4000 / 6000 → total 12000ms, 2 internal cuts.
        p.track_mut("v1").unwrap().clips.extend([
            clip("c1", 2000),
            clip("c2", 4000),
            clip("c3", 6000),
        ]);
        let r = pacing(&edl_from_project(&p));
        assert_eq!(r["shot_count"], 3);
        assert_eq!(r["internal_cuts"], 2);
        assert_eq!(r["mean_shot_ms"], 4000);
        assert_eq!(r["median_shot_ms"], 4000);
        assert_eq!(r["shortest_shot_ms"], 2000);
        assert_eq!(r["longest_shot_ms"], 6000);
        assert_eq!(r["duration_ms"], 12000);
        assert_eq!(r["cuts_per_min"], 10.0); // 2 cuts × 60000 / 12000ms
    }

    #[test]
    fn caption_qc_flags_each_violation_class() {
        use cut_core::CaptionClip;
        let cue = |id: &str, text: &str, a: u64, b: u64| CaptionClip {
            id: id.into(),
            text: text.into(),
            style_ref: None,
            range_ms: [a, b],
        };
        let opts = CaptionQcOpts::default();

        // Clean cue: 11 chars over 2000ms = 5.5 CPS, in-duration, no neighbour.
        let clean = vec![cue("c1", "Hello there", 0, 2000)];
        let r = caption_qc(&clean, opts);
        assert_eq!(r["cue_count"], 1);
        assert_eq!(r["pass"], true, "clean cue should pass: {r}");

        // Too fast: 40 chars over 1000ms = 40 CPS (> 17).
        let fast = vec![cue("c1", &"x".repeat(40), 0, 1000)];
        let r = caption_qc(&fast, opts);
        assert_eq!(r["pass"], false);
        assert!(
            !r["violations"]["too_fast"].as_array().unwrap().is_empty(),
            "too_fast: {r}"
        );

        // Too short: 300ms display (< 833ms floor).
        let short = vec![cue("c1", "Hi", 0, 300)];
        let r = caption_qc(&short, opts);
        assert!(
            !r["violations"]["too_short"].as_array().unwrap().is_empty(),
            "too_short: {r}"
        );

        // Overlap: second cue starts before the first ends.
        let ov = vec![
            cue("c1", "Hello there", 0, 2000),
            cue("c2", "world over", 1500, 3500),
        ];
        let r = caption_qc(&ov, opts);
        assert!(
            !r["violations"]["overlap"].as_array().unwrap().is_empty(),
            "overlap: {r}"
        );

        // Empty track: an honest non-pass, not a vacuous clean.
        let r = caption_qc(&[], opts);
        assert_eq!(r["cue_count"], 0);
        assert_eq!(r["pass"], false);
    }

    #[test]
    fn delivery_measures_rate_and_fillers() {
        use crate::types::WordSpan;
        // Build `n` words evenly across `window_ms`, each voiced for half its
        // slot; every `filler_every`-th word is an "um".
        let build = |n: usize, window_ms: u64, filler_every: usize| -> Vec<WordSpan> {
            let slot = window_ms / n as u64;
            (0..n)
                .map(|i| {
                    let start = i as u64 * slot;
                    WordSpan {
                        idx: i,
                        word: if filler_every > 0 && i % filler_every == 0 {
                            "um".into()
                        } else {
                            "word".into()
                        },
                        start_ms: start,
                        end_ms: start + slot / 2,
                        confidence: None,
                        speaker: None,
                    }
                })
                .collect()
        };
        let lex: Vec<String> = vec!["um".into()];
        let opts = DeliveryOpts::default();

        // 150 words over ~60s ≈ 150 WPM, no fillers → pass. (The window is
        // first-word-start..last-word-end, a hair under 60s, so allow a band.)
        let w = build(150, 60_000, 0);
        let r = delivery(&[w.as_slice()], &lex, opts);
        assert_eq!(r["word_count"], 150);
        assert!(
            (r["wpm"].as_f64().unwrap() - 150.5).abs() < 1.5,
            "≈150 WPM: {r}"
        );
        assert_eq!(r["pass"], true, "in-band rate, no fillers: {r}");

        // 200 words over 60s = 200 WPM (> 170) → too_fast.
        let w = build(200, 60_000, 0);
        let r = delivery(&[w.as_slice()], &lex, opts);
        assert_eq!(r["flags"]["too_fast"], true, "200 WPM: {r}");
        assert_eq!(r["pass"], false);

        // 120 words over 60s with every 4th an "um" = 30 fillers/min (> 3) → high_fillers.
        let w = build(120, 60_000, 4);
        let r = delivery(&[w.as_slice()], &lex, opts);
        assert_eq!(r["flags"]["high_fillers"], true, "filler density: {r}");
        assert!(r["fillers_per_min"].as_f64().unwrap() > 3.0);

        // Aggregation across two assets sums windows, not concatenates words.
        let a = build(75, 30_000, 0);
        let b = build(75, 30_000, 0);
        let r = delivery(&[a.as_slice(), b.as_slice()], &lex, opts);
        assert_eq!(r["word_count"], 150);
        assert!(
            (r["wpm"].as_f64().unwrap() - 151.0).abs() < 1.5,
            "150 words over 30s+30s windows ≈150 WPM: {r}"
        );

        // Too short to measure → honest non-pass.
        let one = build(1, 1000, 0);
        let r = delivery(&[one.as_slice()], &lex, opts);
        assert_eq!(r["pass"], false);
        assert!(r["note"].is_string());
    }

    #[test]
    fn brand_check_enforces_each_constraint() {
        use cut_core::{CaptionStyle, ProjectSettings};
        use std::collections::BTreeMap;
        let style = |font: &str, size: u32, color: &str, pos: &str| CaptionStyle {
            font: font.into(),
            size,
            color: color.into(),
            bg: None,
            pos: Some(pos.into()),
            extra: Default::default(),
        };
        let mut styles = BTreeMap::new();
        styles.insert(
            "brand1".to_string(),
            style("Inter", 42, "#ffffff", "bottom"),
        );
        // 1920x1080 default settings.
        let settings = ProjectSettings {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48000,
            color: cut_core::ColorConfig::default(),
        };

        // Conformant spec → pass. Colour uses #fff shorthand to prove normalization.
        let ok = BrandSpec {
            fonts: Some(vec!["inter".into()]),
            colors: Some(vec!["#fff".into()]),
            position: Some("bottom".into()),
            min_size: Some(20),
            max_size: Some(60),
            aspect: Some((16, 9)),
        };
        let r = brand_check(&styles, &settings, &ok);
        assert_eq!(r["pass"], true, "conformant style+geometry: {r}");
        assert_eq!(r["styles_checked"], 1);

        // Wrong font.
        let r = brand_check(
            &styles,
            &settings,
            &BrandSpec {
                fonts: Some(vec!["Helvetica".into()]),
                ..Default::default()
            },
        );
        assert_eq!(r["pass"], false);
        assert!(
            !r["violations"]["font"].as_array().unwrap().is_empty(),
            "font: {r}"
        );

        // Off-palette colour.
        let r = brand_check(
            &styles,
            &settings,
            &BrandSpec {
                colors: Some(vec!["#000000".into()]),
                ..Default::default()
            },
        );
        assert!(
            !r["violations"]["color"].as_array().unwrap().is_empty(),
            "color: {r}"
        );

        // Wrong position.
        let r = brand_check(
            &styles,
            &settings,
            &BrandSpec {
                position: Some("top".into()),
                ..Default::default()
            },
        );
        assert!(
            !r["violations"]["position"].as_array().unwrap().is_empty(),
            "position: {r}"
        );

        // Size out of bounds (42 > max 30).
        let r = brand_check(
            &styles,
            &settings,
            &BrandSpec {
                max_size: Some(30),
                ..Default::default()
            },
        );
        assert!(
            !r["violations"]["size"].as_array().unwrap().is_empty(),
            "size: {r}"
        );

        // Wrong aspect (project is 16:9, brand wants 9:16).
        let r = brand_check(
            &styles,
            &settings,
            &BrandSpec {
                aspect: Some((9, 16)),
                ..Default::default()
            },
        );
        assert_eq!(r["pass"], false);
        assert!(!r["violations"]["aspect"].is_null(), "aspect: {r}");

        // Nothing pinned + no styles → honest non-pass (nothing to prove).
        let empty: BTreeMap<String, CaptionStyle> = BTreeMap::new();
        let r = brand_check(&empty, &settings, &BrandSpec::default());
        assert_eq!(r["pass"], false);
        assert!(r["note"].is_string());
    }

    #[test]
    fn lufs_pass_and_fail() {
        let mk = |i: f64, p: f64| RenderFacts {
            duration_ms: 1000,
            loudness: Some(Loudness {
                integrated_lufs: i,
                true_peak_dbtp: p,
                windows: vec![],
            }),
            output_report: None,
        };
        assert!(lufs(&mk(-16.5, -2.0), -16.0, 2.0).pass);
        assert!(!lufs(&mk(-25.0, -2.0), -16.0, 2.0).pass, "too quiet");
        assert!(
            !lufs(&mk(-16.0, -0.2), -16.0, 2.0).pass,
            "true peak too hot"
        );
        // Missing loudness fails honestly.
        let none = RenderFacts {
            duration_ms: 0,
            loudness: None,
            output_report: None,
        };
        assert!(!lufs(&none, -16.0, 2.0).pass);
    }

    #[test]
    fn caption_presence_pass_and_orphan() {
        use cut_core::{CaptionClip, Track, TrackKind};
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media_clip("ac1", 0, 5000));
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "s1".into(),
                text: "hello".into(),
                style_ref: None,
                range_ms: [0, 1200],
            })],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        let edl = edl_from_project(&p);
        assert!(caption_presence(&p, &edl).pass);

        // Orphan caption: floats past all audio (5000.. has no audio under it).
        p.tracks
            .last_mut()
            .unwrap()
            .clips
            .push(Clip::Caption(CaptionClip {
                id: "s2".into(),
                text: "floating".into(),
                style_ref: None,
                range_ms: [6000, 7000],
            }));
        let edl2 = edl_from_project(&p);
        let r = caption_presence(&p, &edl2);
        assert!(!r.pass);
        assert_eq!(r.evidence["orphans"][0]["clip_id"], "s2");
    }

    /// Caption-layering guard: a caption over a talking-head clip placed on the BASE VIDEO
    /// track (its embedded audio is program audio) — with NO separate audio
    /// track — must PASS. Before the fix, audio_ranges only saw TrackKind::Audio
    /// segments, so this false-FAILED a perfectly captioned render.
    #[test]
    fn caption_presence_passes_on_base_video_track_audio() {
        use cut_core::{CaptionClip, Track, TrackKind};
        let mut p = Project::new("t", ProjectSettings::default());
        // Media on the base video track v1 only; a1t left empty.
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("vc1", 0, 5000));
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "s1".into(),
                text: "hello".into(),
                style_ref: None,
                range_ms: [0, 1200],
            })],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        let edl = edl_from_project(&p);
        assert!(
            caption_presence(&p, &edl).pass,
            "caption over base-video-track audio must pass (F-4)"
        );
    }

    /// Two caption cues on the same track whose ranges overlap render
    /// stacked (visual clutter) → caption_presence fails with overlap_count.
    #[test]
    fn caption_presence_catches_overlapping_cues() {
        use cut_core::{CaptionClip, Track, TrackKind};
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media_clip("ac1", 0, 5000));
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![
                Clip::Caption(CaptionClip {
                    id: "s1".into(),
                    text: "hello".into(),
                    style_ref: None,
                    range_ms: [0, 1500],
                }),
                Clip::Caption(CaptionClip {
                    id: "s2".into(),
                    text: "world".into(),
                    style_ref: None,
                    range_ms: [1000, 2500], // overlaps s1 on [1000,1500]
                }),
            ],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        let r = caption_presence(&p, &edl_from_project(&p));
        assert!(!r.pass, "overlapping caption cues must fail");
        assert_eq!(r.details["overlap_count"], 1);
    }

    #[test]
    fn caption_presence_fails_without_captions() {
        let p = Project::new("t", ProjectSettings::default());
        let edl = edl_from_project(&p);
        assert!(!caption_presence(&p, &edl).pass);
    }

    /// caption-deduplication guard sanity sub-check: cue text where (nearly) every word is
    /// doubled fails caption_presence even though coverage is perfect;
    /// genuine repetition ("very very good") in otherwise normal text passes.
    #[test]
    fn caption_presence_catches_doubled_words() {
        use cut_core::{CaptionClip, Track, TrackKind};
        let mk = |text: &str| {
            let mut p = Project::new("t", ProjectSettings::default());
            p.track_mut("a1t")
                .unwrap()
                .clips
                .push(media_clip("ac1", 0, 5000));
            p.tracks.push(Track {
                id: "cap1".into(),
                kind: TrackKind::Caption,
                clips: vec![Clip::Caption(CaptionClip {
                    id: "s1".into(),
                    text: text.into(),
                    style_ref: None,
                    range_ms: [0, 1200],
                })],
                gain_db: 0.0,
                gain_windows: vec![],
                blend_mode: None,
                visible: true,
                locked: false,
                muted: false,
                solo: false,
                pan: 0.0,
            });
            let edl = edl_from_project(&p);
            caption_presence(&p, &edl)
        };
        // The artifact shape (punctuation rides on both copies).
        let r = mk("Hello, Hello, and and welcome welcome to to June June");
        assert!(!r.pass, "doubled cue text must fail: {:?}", r.details);
        assert!(r.details["repeated_word_ratio"].as_f64().unwrap() > 0.25);
        // Honest repetition inside normal prose stays under the threshold.
        let r = mk("it was very very good and we shipped it on time");
        assert!(r.pass, "normal prose must pass: {:?}", r.details);
    }

    #[test]
    fn black_frozen_and_edges() {
        let mut report = empty_report();
        let facts_ok = RenderFacts {
            duration_ms: 10_000,
            loudness: None,
            output_report: Some(report.clone()),
        };
        assert!(black_or_frozen_frames(&facts_ok).pass);
        assert!(silence_at_edges(&facts_ok, 500).pass);

        // A black span fails the frame check.
        report.black_spans.push(VideoSpan {
            start_ms: 4000,
            end_ms: 4600,
        });
        // Head silence of 900ms > 500ms budget fails the edge check.
        report.silences.push(SilenceSpan {
            start_ms: 0,
            end_ms: 900,
            source: Some("both".into()),
        });
        let facts_bad = RenderFacts {
            duration_ms: 10_000,
            loudness: None,
            output_report: Some(report),
        };
        assert!(!black_or_frozen_frames(&facts_bad).pass);
        let r = silence_at_edges(&facts_bad, 500);
        assert!(!r.pass);
        assert_eq!(r.evidence["head_silence_ms"], 900);
        // Missing report fails honestly for both.
        let none = RenderFacts {
            duration_ms: 0,
            loudness: None,
            output_report: None,
        };
        assert!(!black_or_frozen_frames(&none).pass);
        assert!(!silence_at_edges(&none, 500).pass);
    }

    #[test]
    fn duration_check_one_frame_tolerance() {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, 5000));
        let edl = edl_from_project(&p); // 5000ms @ 30fps → 34ms tolerance
        let mk = |d: u64| RenderFacts {
            duration_ms: d,
            loudness: None,
            output_report: None,
        };
        assert!(duration_matches_edl(&edl, &mk(5020), 30.0, 48_000).pass);
        assert!(!duration_matches_edl(&edl, &mk(5100), 30.0, 48_000).pass);
        let ntsc = duration_matches_edl(&edl, &mk(5033), 30_000.0 / 1001.0, 48_000);
        assert_eq!(ntsc.details["video_frame_tolerance_ms"], 34);
        assert_eq!(ntsc.details["audio_sample_tolerance_ms"], 1);
    }

    // ---------------- footage profiles (the footage-profile contract) ----------------

    /// Facts mimicking the silent-screen regression render: fully silent (−70 LUFS,
    /// one silence span overrunning the container by 16 ms), no words,
    /// heavy frozen coverage (static UI), no black spans.
    fn silent_screen_facts(duration_ms: u64) -> RenderFacts {
        let mut report = empty_report();
        report.words = Some(Transcript {
            asset: "out".into(),
            model: "test".into(),
            language: None,
            words: vec![],
        });
        report.silences.push(SilenceSpan {
            start_ms: 0,
            end_ms: duration_ms + 16, // wav-extraction overrun, clamped by checks
            source: Some("both".into()),
        });
        // Frozen spans covering ~86% like the regression render measured.
        let mut at = 0;
        while at + 2000 < duration_ms {
            report.frozen_spans.push(VideoSpan {
                start_ms: at,
                end_ms: at + 1800,
            });
            at += 2100;
        }
        report.loudness = Some(Loudness {
            integrated_lufs: -70.0,
            true_peak_dbtp: -99.0,
            windows: vec![],
        });
        // A correctly-framed render fills the frame — full-frame content_bbox,
        // no uniform border (the check gates on screen demos too, so the
        // silent-screen fixture must be clean here to keep passing).
        report.content_bbox = Some(crate::types::ContentBbox {
            frame_width: 1920,
            frame_height: 1080,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            uniform_border: false,
            samples_agreed: Some(4),
        });
        RenderFacts {
            duration_ms,
            loudness: report.loudness.clone(),
            output_report: Some(report),
        }
    }

    fn silent_project(duration_ms: u64) -> Project {
        let mut p = Project::new("t", ProjectSettings::default());
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media_clip("c1", 0, duration_ms));
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media_clip("ac1", 0, duration_ms));
        p
    }

    #[test]
    fn waive_records_and_preserves_measured_outcome() {
        let facts = silent_screen_facts(20_000);
        let measured = lufs(&facts, -16.0, 2.0);
        assert!(!measured.pass, "silent footage fails the lufs target");
        let waived = waive(
            measured.clone(),
            FootageProfile::SilentScreenDemo,
            "test reason",
        );
        assert!(waived.pass, "waived check must not gate the receipt");
        assert_eq!(waived.details["waived_by_profile"], "silent_screen_demo");
        assert_eq!(waived.details["waiver_reason"], "test reason");
        assert_eq!(waived.details["measured_pass"], false);
        assert_eq!(
            waived.evidence, measured.evidence,
            "evidence preserved verbatim"
        );
    }

    #[test]
    fn talking_head_fails_silent_footage_screen_demo_passes_with_waivers() {
        let dur = 20_000;
        let p = silent_project(dur);
        let edl = edl_from_project(&p);
        let facts = silent_screen_facts(dur);

        // Default battery (talking_head): the 4 by-design failures fire.
        let th = run_all(&p, &edl, &[], &facts, &[]);
        let failing: Vec<&str> = th
            .iter()
            .filter(|c| !c.pass)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            failing,
            vec![
                "lufs",
                "caption_presence",
                "black_or_frozen_frames",
                "silence_at_edges"
            ],
            "talking_head must fail exactly the 4 silent-footage checks"
        );
        // ... and the appended metadata entry PROPOSES the right profile.
        let meta = th
            .iter()
            .find(|c| c.name == FOOTAGE_PROFILE_CHECK)
            .expect("metadata entry");
        assert!(meta.pass);
        assert_eq!(meta.details["active_profile"], "talking_head");
        assert_eq!(meta.details["selection"], "default");
        assert_eq!(meta.details["proposed_profile"], "silent_screen_demo");

        // Explicit silent_screen_demo: everything passes, waivers recorded.
        let sd = run_all_with_profile(
            &p,
            &edl,
            &[],
            &facts,
            &[],
            Some(FootageProfile::SilentScreenDemo),
        );
        assert!(
            sd.iter().all(|c| c.pass),
            "screen-demo profile must pass: {:?}",
            sd.iter()
                .filter(|c| !c.pass)
                .map(|c| &c.name)
                .collect::<Vec<_>>()
        );
        for name in ["lufs", "caption_presence", "silence_at_edges"] {
            let c = sd.iter().find(|c| c.name == name).unwrap();
            assert_eq!(
                c.details["waived_by_profile"], "silent_screen_demo",
                "{name} must carry the waiver record"
            );
        }
        // Frozen spans are waived-with-evidence in the UI-tuned variant.
        let bf = sd
            .iter()
            .find(|c| c.name == "black_or_frozen_frames")
            .unwrap();
        assert!(bf.pass);
        assert_eq!(bf.details["variant"], "silent_screen_demo (UI-tuned)");
        assert!(bf.details["waived_frozen_span_count"].as_u64().unwrap() > 0);
        assert!(!bf.evidence["waived_frozen_spans"]
            .as_array()
            .unwrap()
            .is_empty());
        let meta = sd.iter().find(|c| c.name == FOOTAGE_PROFILE_CHECK).unwrap();
        assert_eq!(meta.details["selection"], "explicit");
    }

    #[test]
    fn screen_demo_still_fails_stuck_render_and_black() {
        // One frozen span covering 96% of the render = stuck output.
        let dur = 20_000;
        let mut report = empty_report();
        report.frozen_spans.push(VideoSpan {
            start_ms: 100,
            end_ms: 19_400,
        });
        let facts = RenderFacts {
            duration_ms: dur,
            loudness: None,
            output_report: Some(report.clone()),
        };
        let r = black_or_frozen_frames_screen_demo(&facts);
        assert!(
            !r.pass,
            "a wall-to-wall frozen span must fail even under the UI profile"
        );
        assert_eq!(r.details["stuck_span_count"], 1);

        // Black spans fail in any profile.
        let mut report2 = empty_report();
        report2.black_spans.push(VideoSpan {
            start_ms: 4000,
            end_ms: 4600,
        });
        let facts2 = RenderFacts {
            duration_ms: dur,
            loudness: None,
            output_report: Some(report2),
        };
        assert!(!black_or_frozen_frames_screen_demo(&facts2).pass);

        // Missing report fails honestly (same as the base check).
        let none = RenderFacts {
            duration_ms: 0,
            loudness: None,
            output_report: None,
        };
        assert!(!black_or_frozen_frames_screen_demo(&none).pass);
    }

    #[test]
    fn proposal_requires_all_three_signals() {
        let dur = 20_000;
        // Baseline: fires.
        assert!(propose_profile(&[], &silent_screen_facts(dur)).is_some());

        // Speech present (source transcript) → no proposal.
        let t = transcript();
        assert!(propose_profile(&[&t], &silent_screen_facts(dur)).is_none());

        // Not silent (loud + low silence coverage) → no proposal.
        let mut facts = silent_screen_facts(dur);
        let rep = facts.output_report.as_mut().unwrap();
        rep.silences.clear();
        rep.loudness = Some(Loudness {
            integrated_lufs: -16.0,
            true_peak_dbtp: -2.0,
            windows: vec![],
        });
        facts.loudness = rep.loudness.clone();
        assert!(propose_profile(&[], &facts).is_none());

        // Moving picture (no frozen coverage) → no proposal.
        let mut facts2 = silent_screen_facts(dur);
        facts2.output_report.as_mut().unwrap().frozen_spans.clear();
        assert!(propose_profile(&[], &facts2).is_none());
    }

    #[test]
    fn silence_edge_evidence_clamped_to_duration() {
        // Instrument overrun (46016 on a 46000 render) must not leak into
        // the receipt as head_silence > duration.
        let facts = silent_screen_facts(20_000);
        let r = silence_at_edges(&facts, 500);
        assert!(!r.pass);
        assert_eq!(r.evidence["head_silence_ms"], 20_000);
        assert_eq!(r.evidence["tail_silence_ms"], 20_000);
    }

    /// uniform_border: passes on a full-frame render, fails on a render
    /// with a baked-in border beyond the jitter tolerance, fails honestly when
    /// no content_bbox was measured.
    #[test]
    fn uniform_border_pass_fail_and_missing() {
        let mk = |bbox: Option<crate::types::ContentBbox>| {
            let mut r = empty_report();
            r.content_bbox = bbox;
            RenderFacts {
                duration_ms: 10_000,
                loudness: None,
                output_report: Some(r),
            }
        };
        let full = crate::types::ContentBbox {
            frame_width: 1920,
            frame_height: 1080,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            uniform_border: false,
            samples_agreed: Some(4),
        };
        assert!(
            uniform_border(&mk(Some(full))).pass,
            "a full-frame render must pass"
        );

        // The real driver shape: 3840x2160 with content 3840x2052 at y=54 —
        // baked-in 54px top/bottom bands → inset 54px > 8px tolerance → FAIL.
        let bands = crate::types::ContentBbox {
            frame_width: 3840,
            frame_height: 2160,
            x: 0,
            y: 54,
            width: 3840,
            height: 2052,
            uniform_border: true,
            samples_agreed: Some(4),
        };
        let r = uniform_border(&mk(Some(bands)));
        assert!(!r.pass, "a letterboxed render must fail");
        assert_eq!(r.evidence["inset_px"]["top"], 54);
        assert_eq!(r.evidence["inset_px"]["bottom"], 54);

        // A sub-tolerance inset (cropdetect jitter) still passes.
        let jitter = crate::types::ContentBbox {
            frame_width: 1920,
            frame_height: 1080,
            x: 4,
            y: 4,
            width: 1912,
            height: 1072,
            uniform_border: false,
            samples_agreed: Some(4),
        };
        assert!(
            uniform_border(&mk(Some(jitter))).pass,
            "sub-tolerance inset is jitter, not a band"
        );

        // No content_bbox measured → fail honestly (never a vacuous pass).
        assert!(!uniform_border(&mk(None)).pass);
        let none = RenderFacts {
            duration_ms: 0,
            loudness: None,
            output_report: None,
        };
        assert!(!uniform_border(&none).pass);
    }

    /// uniform_border is in the battery for BOTH profiles and is NEVER waived
    /// — a margin is a defect on a silent screen demo too (the original case was
    /// exactly a silent screen demo with baked-in bands). A bordered render
    /// fails the receipt under silent_screen_demo just as under talking_head.
    #[test]
    fn uniform_border_gates_both_profiles_never_waived() {
        let dur = 20_000;
        let p = silent_project(dur);
        let edl = edl_from_project(&p);
        // Start from the (clean) silent-screen fixture, then bake in a border.
        let mut facts = silent_screen_facts(dur);
        facts.output_report.as_mut().unwrap().content_bbox = Some(crate::types::ContentBbox {
            frame_width: 1920,
            frame_height: 1080,
            x: 0,
            y: 60,
            width: 1920,
            height: 960,
            uniform_border: true,
            samples_agreed: Some(4),
        });
        for prof in [None, Some(FootageProfile::SilentScreenDemo)] {
            let checks = run_all_with_profile(&p, &edl, &[], &facts, &[], prof);
            let ub = checks
                .iter()
                .find(|c| c.name == "uniform_border")
                .expect("uniform_border present");
            assert!(
                !ub.pass,
                "uniform_border must FAIL on a bordered render (profile {prof:?})"
            );
            // It carries no waiver record under silent_screen_demo (not waived).
            assert!(
                ub.details.get("waived_by_profile").is_none(),
                "uniform_border must NOT be waived by the screen-demo profile"
            );
            assert!(
                !checks.iter().all(|c| c.pass),
                "the receipt must not pass with a bordered render (profile {prof:?})"
            );
        }
    }

    #[test]
    fn profile_wire_names_round_trip() {
        use std::str::FromStr;
        for p in [
            FootageProfile::TalkingHead,
            FootageProfile::SilentScreenDemo,
        ] {
            assert_eq!(FootageProfile::from_str(p.as_str()).unwrap(), p);
        }
        assert!(FootageProfile::from_str("camera").is_err());
        // serde wire form matches as_str (server passes the arg through serde).
        assert_eq!(
            serde_json::to_value(FootageProfile::SilentScreenDemo).unwrap(),
            json!("silent_screen_demo")
        );
    }

    // ----------------------------- verify.pregate ------------------------------
    use crate::types::{ContentBbox, Loudness};
    use cut_core::{GapClip, TrackKind};
    use std::collections::BTreeMap;

    /// An empty reports map (no perception facts cached).
    fn no_reports() -> BTreeMap<String, PerceptionReport> {
        BTreeMap::new()
    }

    /// Find a risk by kind in a report.
    fn risk<'a>(r: &'a PregateReport, kind: &str) -> Option<&'a PregateRisk> {
        r.risks.iter().find(|x| x.kind == kind)
    }

    /// Push a media clip onto a named track of the project.
    fn push_media(p: &mut Project, track: &str, id: &str, src_in: u64, src_out: u64) {
        p.track_mut(track)
            .unwrap()
            .clips
            .push(media_clip(id, src_in, src_out));
    }

    /// A clean short project — video + matching audio, under the slideshow floor,
    /// no perception needed — must pregate-pass with zero risks.
    #[test]
    fn pregate_clean_passes() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        push_media(&mut p, "a1t", "a1c", 0, 5000);
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        assert!(r.pass, "clean project must pass: {:?}", r.risks);
        assert!(r.risks.is_empty(), "no risks expected: {:?}", r.risks);
    }

    /// A longer audio bed than the video ⇒ the program plays into BLACK at the
    /// tail. empty_tail is HIGH and blocks the gate.
    #[test]
    fn pregate_empty_tail_audio_bed_high_fails() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 4000);
        push_media(&mut p, "a1t", "a1c", 0, 8000);
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        assert!(!r.pass, "an empty tail must FAIL the gate");
        let t = risk(&r, "empty_tail").expect("empty_tail risk present");
        assert_eq!(t.severity, "high");
        assert_eq!(t.range_ms, Some([4000, 8000]), "tail span localized");
    }

    /// A trailing GAP on the video track also extends the timeline past the last
    /// clip ⇒ empty_tail (the canonical "extend project duration past the last
    /// clip" shape).
    #[test]
    fn pregate_empty_tail_trailing_gap_high_fails() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 4000);
        p.track_mut("v1").unwrap().clips.push(Clip::Gap(GapClip {
            kind: "gap".into(),
            duration_ms: 3000,
        }));
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        assert!(!r.pass);
        let t = risk(&r, "empty_tail").expect("empty_tail present");
        assert_eq!(t.range_ms, Some([4000, 7000]));
    }

    /// A clip whose source overlaps a black span ⇒ dead footage baked into the
    /// cut. black_or_frozen is HIGH.
    #[test]
    fn pregate_black_span_high_fails() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        let edl = edl_from_project(&p);
        // Asset a1 has a 2000ms black span inside the clip's source window, and
        // loudness (so it counts as audio-bearing → no silent_output noise).
        let mut rep = empty_report();
        rep.black_spans = vec![VideoSpan {
            start_ms: 1000,
            end_ms: 3000,
        }];
        rep.loudness = Some(Loudness {
            integrated_lufs: -16.0,
            true_peak_dbtp: -2.0,
            windows: vec![],
        });
        let mut reports = no_reports();
        reports.insert("a1".into(), rep);
        let r = pregate(&edl, 30.0, &reports, PregateOpts::default());
        assert!(!r.pass, "black footage must FAIL");
        let b = risk(&r, "black_or_frozen").expect("black_or_frozen present");
        assert_eq!(b.severity, "high");
        assert_eq!(b.range_ms, Some([0, 5000]));
        assert!(b.detail.contains("2000ms black"), "detail: {}", b.detail);
        // The asset WAS analyzed → provenance reflects it.
        assert_eq!(r.perception_assets, 1);
        assert!(r.uninstrumented_assets.is_empty());
    }

    /// A single long static shot ⇒ pre-render "slideshow". MED, non-blocking.
    #[test]
    fn pregate_slideshow_med_does_not_block() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 30_000);
        push_media(&mut p, "a1t", "a1c", 0, 30_000);
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        let s = risk(&r, "slideshow_risk").expect("slideshow risk present");
        assert_eq!(s.severity, "med");
        assert!(r.pass, "a MED slideshow must NOT block the gate");
    }

    /// No audio-bearing clip anywhere ⇒ silent render (MED).
    #[test]
    fn pregate_no_audio_is_silent_med() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        let s = risk(&r, "silent_output").expect("silent_output present");
        assert_eq!(s.severity, "med");
        assert!(r.pass);
    }

    /// Audio present but ≈all silence (per facts) ⇒ silent_output (MED).
    #[test]
    fn pregate_mostly_silent_audio_med() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        push_media(&mut p, "a1t", "a1c", 0, 5000);
        let edl = edl_from_project(&p);
        // a1's source is silent for 4800 of its 5000ms window.
        let mut rep = empty_report();
        rep.silences = vec![SilenceSpan {
            start_ms: 100,
            end_ms: 4900,
            source: Some("both".into()),
        }];
        let mut reports = no_reports();
        reports.insert("a1".into(), rep);
        let r = pregate(&edl, 30.0, &reports, PregateOpts::default());
        let s = risk(&r, "silent_output").expect("silent_output present");
        assert_eq!(s.severity, "med");
        assert!(r.pass);
    }

    /// A sub-frame clip never renders a visible frame (MED).
    #[test]
    fn pregate_tiny_clip_med() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        push_media(&mut p, "v1", "c2", 0, 20); // 20ms < 33ms frame @30fps
        push_media(&mut p, "a1t", "a1c", 0, 5020); // keep audio so no silent noise
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        let t = risk(&r, "tiny_or_zero_clips").expect("tiny risk present");
        assert_eq!(t.severity, "med");
        assert!(t.detail.contains("c2"), "names the offender: {}", t.detail);
        assert!(r.pass, "tiny clip is MED, not blocking");
    }

    /// A baked-in letterbox ⇒ uniform_border (LOW, non-blocking).
    #[test]
    fn pregate_uniform_border_low() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        push_media(&mut p, "a1t", "a1c", 0, 5000);
        let edl = edl_from_project(&p);
        let mut rep = empty_report();
        rep.content_bbox = Some(ContentBbox {
            frame_width: 1920,
            frame_height: 1080,
            x: 0,
            y: 56,
            width: 1920,
            height: 968,
            uniform_border: true,
            samples_agreed: Some(3),
        });
        let mut reports = no_reports();
        reports.insert("a1".into(), rep);
        let r = pregate(&edl, 30.0, &reports, PregateOpts::default());
        let u = risk(&r, "uniform_border").expect("uniform_border present");
        assert_eq!(u.severity, "low");
        assert!(r.pass, "LOW does not block the gate");
    }

    /// Provenance: an EDL-referenced asset with NO cached report is reported as
    /// uninstrumented, so a clean pass over un-analyzed footage is honestly
    /// distinguishable from one over analyzed footage.
    #[test]
    fn pregate_reports_uninstrumented_assets() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "v1", "c1", 0, 5000);
        push_media(&mut p, "a1t", "a1c", 0, 5000);
        let edl = edl_from_project(&p);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        assert_eq!(r.perception_assets, 0);
        assert_eq!(r.uninstrumented_assets, vec!["a1".to_string()]);
    }

    /// The TrackKind import is exercised so an audio-only project skips
    /// empty_tail (no picture ⇒ no "tail into black").
    #[test]
    fn pregate_audio_only_skips_empty_tail() {
        let mut p = Project::new("t", ProjectSettings::default());
        push_media(&mut p, "a1t", "a1c", 0, 8000);
        let edl = edl_from_project(&p);
        assert_eq!(edl.duration_ms, 8000);
        let r = pregate(&edl, 30.0, &no_reports(), PregateOpts::default());
        assert!(
            risk(&r, "empty_tail").is_none(),
            "audio-only render has no picture → no empty_tail"
        );
        // It IS flagged silent only if no audio facts; here a1t has a clip but no
        // report → unmeasured → not silent. Track kind import used:
        assert_eq!(p.track("a1t").unwrap().kind, TrackKind::Audio);
    }
}
