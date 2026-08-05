//! receipt.rs — RenderReceipt + check results (render-receipt contract, "verify").
//!
//! Role: "done requires evidence" — render completion emits a RenderReceipt
//! with measured facts + check results; the agent never claims success
//! without it. Checks themselves run in cut-perception; the TYPES live here
//! so core/media/perception/server all speak the same receipt shape.
//! Dependencies: serde. Primary callers: cut-perception (produces
//! CheckResult), cut-media (RenderOutput facts), server (verify.checks,
//! render.final auto-check), UI review rail.

use serde::{Deserialize, Serialize};

/// Result of one verification check (public verb contract: "each → {name, pass, details,
/// evidence}"). `details` is human-readable structured data; `evidence` points
/// at/embeds the measured facts that justify the verdict (timestamps, LUFS
/// numbers, frame paths) — never a bare boolean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    /// Canonical check name: cut_on_word | lufs | caption_presence |
    /// black_or_frozen_frames | silence_at_edges | duration_matches_edl |
    /// uniform_border.
    pub name: String,
    pub pass: bool,
    pub details: serde_json::Value,
    pub evidence: serde_json::Value,
}

/// Canonical check names (`verify.checks` runs exactly this battery).
pub mod check_names {
    pub const CUT_ON_WORD: &str = "cut_on_word";
    /// For a music-driven edit, the distribution of each
    /// program cut's distance to the nearest beat of the placed music. A
    /// MEASUREMENT receipt (never fails a non-beat-aligned edit); emitted only
    /// when a placed asset carries a beat grid.
    pub const CUT_ON_BEAT: &str = "cut_on_beat";
    /// Completes the measurement trio with cut_on_word/cut_on_beat and
    /// classifies each transition where the base video and base audio cuts are
    /// OFFSET as a J-cut (audio leads) or L-cut (video leads) and measures the
    /// lead/lag. Structural (EDL-only, no diarization). A MEASUREMENT receipt
    /// (pass:true), emitted only when at least one J/L cut exists (an aligned
    /// edit has none → not in the receipt).
    pub const J_L_CUT: &str = "j_l_cut";
    /// The verifiable-edit measurement set, audio side — for a music-bed edit with recorded
    /// DUCK windows (a gain_window that REDUCES the bed), how much of the ducked
    /// time lands ON speech (transcript words mapped to timeline). A good bed
    /// ducks UNDER the talk. MEASUREMENT receipt (pass:true), emitted only when
    /// the edit actually contains a duck window (no music bed → not in receipt).
    pub const BED_DUCK_UNDER_SPEECH: &str = "bed_duck_under_speech";
    /// The verifiable-edit measurement set, video side — for each recorded CROSSFADE seam
    /// (a segment with xfade_in_ms > 0), confirms a real dissolve of the recorded
    /// length sits at the seam (not a hard cut). Structural/intent (EDL-only): it
    /// proves the crossfade is present + has a non-degenerate duration. A
    /// MEASUREMENT receipt (pass:true), emitted only when a crossfade exists.
    pub const CROSSFADE_SMOOTHNESS: &str = "crossfade_smoothness";
    pub const LUFS: &str = "lufs";
    pub const CAPTION_PRESENCE: &str = "caption_presence";
    pub const BLACK_OR_FROZEN_FRAMES: &str = "black_or_frozen_frames";
    pub const SILENCE_AT_EDGES: &str = "silence_at_edges";
    pub const DURATION_MATCHES_EDL: &str = "duration_matches_edl";
    /// Framing correctness: the rendered output carries no baked-in
    /// uniform-colour border (cropdetect on the render). NOT waived by any
    /// footage profile — a margin is a defect on talking-head AND screen-demo
    /// footage (cut-perception emits it in the battery).
    pub const UNIFORM_BORDER: &str = "uniform_border";
    /// Metadata entry (always pass:true — documents, never gates): which
    /// footage profile ran the battery + the auto-detect proposal emitted by
    /// cut-perception as the receipt's metadata entry.
    pub const FOOTAGE_PROFILE: &str = "footage_profile";
}

/// The receipt for a completed render (render-receipt contract). Persisted under
/// `receipts/render_<id>.json` and pushed over WS as `receipt_ready`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderReceipt {
    /// "render_NNN" id, unique per project.
    pub render_id: String,
    /// RFC3339 completion time.
    pub ts: String,
    /// Output file path + content hash (determinism: same input+EDL ⇒ same hash).
    pub output_path: String,
    pub output_hash: String,
    /// Measured output duration, ms.
    pub duration_ms: u64,
    /// Encoder preset used (e.g. "h264_1080p30").
    pub preset: String,
    /// Op id of the log head when the render started — ties the receipt to an
    /// exact timeline state.
    pub at_op: String,
    /// The check battery results.
    pub checks: Vec<CheckResult>,
    /// Aggregate: true iff every check passed.
    pub pass: bool,
    /// Optional judge (VLM) review attachment — absent until a judge run when
    /// no API key; NEVER a fake pass (public verb contract verify.judge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<serde_json::Value>,
    /// Machine-actionable repairs derived from the FAILING checks (one per
    /// recognised failure) — the substrate the Receipted-Autopilot self-fix
    /// loop maps onto. Empty when every check passed. `#[serde(default)]`:
    /// receipts written before this field load fine (read back as empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix_actions: Vec<FixAction>,
}

impl RenderReceipt {
    /// Compute the derived fields from `checks`: aggregate `pass` (all green)
    /// AND the `fix_actions` list (one per recognised failing check). Call once
    /// after filling `checks`.
    pub fn compute_pass(&mut self) {
        self.pass = !self.checks.is_empty() && self.checks.iter().all(|c| c.pass);
        self.fix_actions = self.fix_actions();
    }

    /// Derive the fix-action list for THIS receipt: one [`FixAction`] per
    /// FAILING check that the [`fix_action`] mapper recognises. This is the
    /// substrate the Receipted-Autopilot self-fix loop consumes — instead of
    /// re-parsing heterogeneous check evidence, it reads a uniform, typed list
    /// of "this check failed → call this verb on this target". Passing checks
    /// and metadata receipts (footage_profile, cut_on_beat, j_l_cut) yield no
    /// action.
    pub fn fix_actions(&self) -> Vec<FixAction> {
        self.checks.iter().filter_map(fix_action).collect()
    }
}

/// Where a defect lives, lifted from a check's structured evidence — the
/// provenance an agent acts on (`clip_id` to target the edit, `at_ms` to seek,
/// `op_id` to trace the op that introduced it). All optional: a check carries
/// whichever coordinates it measured; never fabricated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FixTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
}

/// A machine-actionable repair derived from one FAILING [`CheckResult`]. The
/// contract that the autopilot self-fix loop maps a receipt onto: which verb to
/// call (`fix_verb`), a best-effort partial arg set (`fix_args` — the autopilot
/// merges in the render/target context it owns), WHERE (`targets`), the
/// measured-vs-target numbers that justify it (`measured`), a one-line `why`,
/// and whether the fix is mechanical + safe to auto-apply (`auto_fixable`) or
/// needs a human / the perceptual judge (`auto_fixable=false`, surfaced so a
/// failing check is NEVER silently un-actionable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixAction {
    /// The failing check name this addresses.
    pub check: String,
    /// The verb the autopilot should call to repair it. "(manual)" when no
    /// mechanical fix exists (auto_fixable=false) — the check stays visible.
    pub fix_verb: String,
    /// Best-effort partial args for `fix_verb`; the autopilot fills the rest.
    pub fix_args: serde_json::Value,
    /// Defect provenance from the check evidence (clip/time/op).
    pub targets: Vec<FixTarget>,
    /// The measured numbers + target that justify the fix (for the plan/log).
    pub measured: serde_json::Value,
    /// One-line rationale for the agent's plan + the user-facing "what changed".
    pub rationale: String,
    /// True = mechanical + safe for the self-fix loop to apply; false = needs
    /// a human or verify.judge (e.g. black/frozen frames, duration drift).
    pub auto_fixable: bool,
}

#[cfg(test)]
mod fix_action_tests {
    use super::*;
    use serde_json::json;

    fn check(
        name: &str,
        pass: bool,
        details: serde_json::Value,
        evidence: serde_json::Value,
    ) -> CheckResult {
        CheckResult {
            name: name.into(),
            pass,
            details,
            evidence,
        }
    }

    #[test]
    fn passing_and_metadata_checks_yield_no_action() {
        assert!(fix_action(&check(check_names::LUFS, true, json!({}), json!({}))).is_none());
        // metadata receipts are pass:true → no action even though "named".
        assert!(fix_action(&check(
            check_names::FOOTAGE_PROFILE,
            true,
            json!({}),
            json!({})
        ))
        .is_none());
    }

    #[test]
    fn unmeasured_check_yields_no_repair_action() {
        let c = check(
            check_names::UNIFORM_BORDER,
            false,
            json!({
                "status": "unmeasured",
                "measured": false,
                "runtime_error": {"code": "sidecar", "message": "instrumentation failed"},
            }),
            json!({"content_bbox": null}),
        );
        assert!(
            fix_action(&c).is_none(),
            "a runtime gap is not a content defect to auto-repair"
        );
    }

    #[test]
    fn lufs_failure_maps_to_renormalize() {
        let c = check(
            check_names::LUFS,
            false,
            json!({"target_lufs": -14.0}),
            json!({"integrated_lufs": -22.3, "true_peak_dbtp": -3.1}),
        );
        let a = fix_action(&c).expect("lufs failure is actionable");
        assert_eq!(a.fix_verb, "render.final");
        assert_eq!(a.fix_args["normalize_loudness"], json!(-14));
        assert!(a.auto_fixable);
        assert_eq!(a.measured["integrated_lufs"], json!(-22.3));
    }

    #[test]
    fn uniform_border_maps_to_fit_cover() {
        let c = check(
            check_names::UNIFORM_BORDER,
            false,
            json!({"measured_max_inset_px": 56}),
            json!({"inset_px": {"left": 0, "top": 56, "right": 0, "bottom": 56}}),
        );
        let a = fix_action(&c).unwrap();
        assert_eq!(a.fix_verb, "render.final");
        assert_eq!(a.fix_args["fit"], json!("cover"));
        assert!(a.auto_fixable);
    }

    #[test]
    fn cut_on_word_carries_clip_and_at_ms_and_snaps_to_nearest_edge() {
        // Cut at 900, inside "world" [600,1200]; nearer edge is 1200 (|900-1200|=300 vs |900-600|=300 → tie → start).
        // Use a clearer case: cut at 1100 → nearer 1200.
        let c = check(
            check_names::CUT_ON_WORD,
            false,
            json!({"violation_count": 1}),
            json!({"violations": [
                {"clip_id": "c3", "boundary": "src_out", "cut_src_ms": 1100,
                 "word": "world", "word_idx": 1, "word_span_ms": [600, 1200]}
            ]}),
        );
        let a = fix_action(&c).unwrap();
        assert_eq!(a.fix_verb, "edit.trim");
        assert!(a.auto_fixable);
        assert_eq!(a.targets.len(), 1);
        assert_eq!(a.targets[0].clip_id.as_deref(), Some("c3"));
        assert_eq!(a.targets[0].at_ms, Some(1100));
        assert_eq!(a.fix_args["snap_src_ms"], json!(1200)); // nearest word edge
    }

    #[test]
    fn caption_overlap_reflows_but_doubled_word_regenerates_manually() {
        let overlap = check(
            check_names::CAPTION_PRESENCE,
            false,
            json!({"overlap_count": 1, "orphan_count": 0, "repeated_word_ratio": 0.0, "repeated_word_ratio_max": 0.25}),
            json!({"overlaps": [{"a": {"clip_id": "cap1"}, "b": {"clip_id": "cap2"}}], "orphans": []}),
        );
        let a = fix_action(&overlap).unwrap();
        assert_eq!(a.fix_verb, "captions.reflow");
        assert!(a.auto_fixable);
        assert!(a
            .targets
            .iter()
            .any(|t| t.clip_id.as_deref() == Some("cap1")));

        let doubled = check(
            check_names::CAPTION_PRESENCE,
            false,
            json!({"overlap_count": 0, "orphan_count": 0, "repeated_word_ratio": 0.6, "repeated_word_ratio_max": 0.25}),
            json!({"overlaps": [], "orphans": []}),
        );
        let a2 = fix_action(&doubled).unwrap();
        assert_eq!(a2.fix_verb, "captions.generate");
        assert!(
            !a2.auto_fixable,
            "a generation artifact is not a mechanical fix"
        );
    }

    #[test]
    fn unrecognised_failure_is_surfaced_as_manual_not_dropped() {
        let c = check(
            check_names::BLACK_OR_FROZEN_FRAMES,
            false,
            json!({"black_span_count": 2}),
            json!({"black_spans": [{"start_ms": 0, "end_ms": 400}]}),
        );
        let a = fix_action(&c).unwrap();
        assert_eq!(a.fix_verb, "(manual)");
        assert!(
            !a.auto_fixable,
            "no silent drop — a red check stays visible with a reason"
        );
    }

    #[test]
    fn receipt_fix_actions_collects_only_failures() {
        let mut r = RenderReceipt {
            render_id: "render_001".into(),
            ts: "2026-06-16T00:00:00.000Z".into(),
            output_path: "/o.mp4".into(),
            output_hash: "sha256:x".into(),
            duration_ms: 1000,
            preset: "standard".into(),
            at_op: "op_000001".into(),
            checks: vec![
                check(check_names::CUT_ON_WORD, true, json!({}), json!({})),
                check(
                    check_names::LUFS,
                    false,
                    json!({"target_lufs": -16.0}),
                    json!({"integrated_lufs": -9.0}),
                ),
            ],
            pass: false,
            judge: None,
            fix_actions: vec![],
        };
        r.compute_pass();
        assert!(!r.pass);
        assert_eq!(
            r.fix_actions.len(),
            1,
            "only the failing lufs check is actionable"
        );
        assert_eq!(r.fix_actions[0].check, check_names::LUFS);
    }
}

/// Map ONE check to a [`FixAction`] — `None` for a passing check or a metadata
/// receipt (footage_profile / cut_on_beat / j_l_cut never fail). Pure: reads
/// only the check's own `details`/`evidence` JSON, so it stays in lock-step
/// with whatever the perception battery emits and is fully unit-testable.
///
/// The mapping IS the autopilot's repair policy, kept here (next to the receipt
/// types) so the contract is one place:
///   - `lufs`              → render.final{normalize_loudness:<target>}   (auto)
///   - `silence_at_edges`  → edit.trim_edges                            (auto)
///   - `uniform_border`    → render.final{fit:"cover"}                  (auto)
///   - `cut_on_word`       → edit.trim (snap boundary to nearest word edge) (auto)
///   - `caption_presence`  → captions.reflow (overlap/orphan) OR
///                           captions.generate (doubled-word artifact, manual)
///   - `black_or_frozen_frames`, `duration_matches_edl` → (manual)      (not auto)
pub fn fix_action(check: &CheckResult) -> Option<FixAction> {
    let unmeasured = check.details.get("status").and_then(|v| v.as_str()) == Some("unmeasured")
        || check.details.get("measured").and_then(|v| v.as_bool()) == Some(false);
    if check.pass || unmeasured {
        return None;
    }
    let d = &check.details;
    let e = &check.evidence;
    Some(match check.name.as_str() {
        check_names::LUFS => {
            let target = d
                .get("target_lufs")
                .and_then(|v| v.as_f64())
                .unwrap_or(-16.0);
            FixAction {
                check: check.name.clone(),
                fix_verb: "render.final".into(),
                fix_args: serde_json::json!({ "normalize_loudness": target.round() as i64 }),
                targets: vec![],
                measured: serde_json::json!({
                    "integrated_lufs": e.get("integrated_lufs"),
                    "true_peak_dbtp": e.get("true_peak_dbtp"),
                    "target_lufs": target,
                }),
                rationale: format!(
                    "loudness off target — re-render with loudnorm to {target} LUFS"
                ),
                auto_fixable: true,
            }
        }
        check_names::SILENCE_AT_EDGES => FixAction {
            check: check.name.clone(),
            fix_verb: "edit.trim_edges".into(),
            fix_args: serde_json::json!({}),
            targets: vec![],
            measured: serde_json::json!({
                "head_silence_ms": e.get("head_silence_ms"),
                "tail_silence_ms": e.get("tail_silence_ms"),
            }),
            rationale: "silence at an edge — trim leading/trailing dead air".into(),
            auto_fixable: true,
        },
        check_names::UNIFORM_BORDER => FixAction {
            check: check.name.clone(),
            fix_verb: "render.final".into(),
            fix_args: serde_json::json!({ "fit": "cover" }),
            targets: vec![],
            measured: serde_json::json!({
                "measured_max_inset_px": d.get("measured_max_inset_px"),
                "inset_px": e.get("inset_px"),
            }),
            rationale: "baked-in border — re-render fit:cover to crop-to-fill".into(),
            auto_fixable: true,
        },
        check_names::CUT_ON_WORD => {
            // One target per violating boundary (clip + the source-time cut).
            let mut targets = Vec::new();
            let mut first_args = serde_json::json!({});
            if let Some(vios) = e.get("violations").and_then(|v| v.as_array()) {
                for (i, v) in vios.iter().enumerate() {
                    let clip_id = v.get("clip_id").and_then(|x| x.as_str()).map(String::from);
                    let cut = v.get("cut_src_ms").and_then(|x| x.as_u64());
                    targets.push(FixTarget {
                        clip_id: clip_id.clone(),
                        at_ms: cut,
                        op_id: None,
                    });
                    if i == 0 {
                        // Snap to the nearer word edge of the first violation.
                        if let (Some(span), Some(cut)) =
                            (v.get("word_span_ms").and_then(|s| s.as_array()), cut)
                        {
                            let start = span.first().and_then(|x| x.as_u64()).unwrap_or(cut);
                            let end = span.get(1).and_then(|x| x.as_u64()).unwrap_or(cut);
                            let edge = if cut.abs_diff(start) <= cut.abs_diff(end) {
                                start
                            } else {
                                end
                            };
                            first_args = serde_json::json!({
                                "clip": clip_id, "boundary": v.get("boundary"),
                                "snap_src_ms": edge,
                            });
                        }
                    }
                }
            }
            FixAction {
                check: check.name.clone(),
                fix_verb: "edit.trim".into(),
                fix_args: first_args,
                targets,
                measured: serde_json::json!({ "violation_count": d.get("violation_count") }),
                rationale: "a cut lands inside a word — snap the boundary to the nearest word edge"
                    .into(),
                auto_fixable: true,
            }
        }
        check_names::CAPTION_PRESENCE => {
            // Overlaps/orphans are positional → reflow can fix; a doubled-word
            // text artifact is a generation defect → regenerate (not mechanical).
            let overlap_count = d.get("overlap_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let orphan_count = d.get("orphan_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let repeated = d
                .get("repeated_word_ratio")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let repeated_max = d
                .get("repeated_word_ratio_max")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.25);
            let positional = overlap_count > 0 || orphan_count > 0;
            let doubled = repeated > repeated_max;
            let mut targets = Vec::new();
            for key in ["overlaps", "orphans"] {
                if let Some(arr) = e.get(key).and_then(|v| v.as_array()) {
                    for v in arr {
                        // overlaps nest clip ids under a/b; orphans carry clip_id.
                        for cid in [
                            v.get("clip_id").and_then(|x| x.as_str()),
                            v.get("a")
                                .and_then(|a| a.get("clip_id"))
                                .and_then(|x| x.as_str()),
                            v.get("b")
                                .and_then(|b| b.get("clip_id"))
                                .and_then(|x| x.as_str()),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            targets.push(FixTarget {
                                clip_id: Some(cid.to_string()),
                                at_ms: None,
                                op_id: None,
                            });
                        }
                    }
                }
            }
            if positional {
                FixAction {
                    check: check.name.clone(),
                    fix_verb: "captions.reflow".into(),
                    fix_args: serde_json::json!({}),
                    targets,
                    measured: serde_json::json!({
                        "overlap_count": overlap_count, "orphan_count": orphan_count,
                    }),
                    rationale: "caption overlap/orphan — reflow the caption track".into(),
                    auto_fixable: true,
                }
            } else if doubled {
                FixAction {
                    check: check.name.clone(),
                    fix_verb: "captions.generate".into(),
                    fix_args: serde_json::json!({}),
                    targets,
                    measured: serde_json::json!({
                        "repeated_word_ratio": repeated, "max": repeated_max,
                    }),
                    rationale:
                        "doubled-word caption artifact — regenerate captions from transcript".into(),
                    auto_fixable: false,
                }
            } else {
                // Missing track entirely → generate.
                FixAction {
                    check: check.name.clone(),
                    fix_verb: "captions.generate".into(),
                    fix_args: serde_json::json!({}),
                    targets,
                    measured: serde_json::json!({
                        "has_caption_track": d.get("has_caption_track"),
                        "caption_clip_count": d.get("caption_clip_count"),
                    }),
                    rationale: "no captions present — generate captions from the transcript".into(),
                    auto_fixable: false,
                }
            }
        }
        // Failing checks with no mechanical repair — surfaced (not hidden) so a
        // red check always carries an explanation of why it can't auto-fix.
        other => FixAction {
            check: other.to_string(),
            fix_verb: "(manual)".into(),
            fix_args: serde_json::json!({}),
            targets: vec![],
            measured: check.evidence.clone(),
            rationale: format!(
                "'{other}' has no mechanical auto-fix — needs a human edit or verify.judge"
            ),
            auto_fixable: false,
        },
    })
}
