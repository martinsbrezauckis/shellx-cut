//! check_outcome.rs — what a render's own checks mean, in one verdict.
//!
//! A receipt lists every check with pass/fail. A person or an agent who looks
//! at the render JOB (or the status bar) must not have to open the receipt to
//! learn that the file did not pass. Large editors report an export that
//! completed with problems as "completed with warnings/errors" and list them;
//! they never report plain success. This module gives the receipt that one
//! verdict, in two levels:
//!
//! - `problems`: a check that says the FILE is not the timeline failed
//!   (`duration_matches_edl`: a stream is shorter than the timeline;
//!   `sources_decodable`: a source is cut off or damaged). The file was
//!   written, but it is not a correct export of the edit.
//! - `warnings`: only quality or editorial checks failed (loudness, captions,
//!   silence or black at the edges, frozen frames, borders, cuts inside words):
//!   the file is the edit, it may still need work.
//!
//! Unmeasured checks (they apply, but the analysis runtime could not measure
//! them) are neither: they are counted, not judged. Not-applicable checks (this
//! export has nothing for them to check: no captions, no sound, no transcript,
//! no picture) are a third, distinct state: never a failure, never a warning,
//! listed separately. `passed` means every measured, applicable check passed.

use crate::receipt::{check_names as cn, CheckResult};
use serde::{Deserialize, Serialize};

/// The checks whose failure means the written file is not the timeline.
pub const INTEGRITY_CHECKS: &[&str] = &[cn::DURATION_MATCHES_EDL, cn::SOURCES_DECODABLE];

/// One failing check, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckProblem {
    /// The check's name, e.g. "duration_matches_edl".
    pub name: String,
    /// "problem" (the file is not the timeline) or "warning" (quality).
    pub level: String,
    /// What a failure of this check means, for a person.
    pub meaning: String,
    /// The check's own reason, when it states one (e.g. which stream is short).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The receipt's verdict: `passed`, `problems`, `warnings` or `unmeasured`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub verdict: String,
    /// One sentence a person reads.
    pub summary: String,
    /// Failing measured checks, problems first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<CheckProblem>,
    /// Checks that could not be measured on this machine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmeasured: Vec<String>,
    /// Checks that do not apply to this export (see [`not_applicable_check`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_applicable: Vec<String>,
    /// The way forward, when something failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// `details.status` of a check that does not apply to this export.
pub const NOT_APPLICABLE: &str = "not_applicable";

/// A check that does not apply to this export: `pass` stays true so it never
/// fails the aggregate or produces a fix action, and `details.status` says
/// `not_applicable` with when it would apply and why it does not here, so no
/// surface shows it as a measured pass.
pub fn not_applicable_check(name: &str, applies_when: &str, reason: &str) -> CheckResult {
    CheckResult {
        name: name.into(),
        pass: true,
        details: serde_json::json!({
            "status": NOT_APPLICABLE,
            "applies_when": applies_when,
            "reason": reason,
        }),
        evidence: serde_json::json!({}),
    }
}

/// True for a check made by [`not_applicable_check`].
pub fn is_not_applicable(check: &CheckResult) -> bool {
    check.details.get("status").and_then(|v| v.as_str()) == Some(NOT_APPLICABLE)
}

fn is_unmeasured(check: &CheckResult) -> bool {
    check.details.get("status").and_then(|v| v.as_str()) == Some("unmeasured")
        || check.details.get("measured").and_then(|v| v.as_bool()) == Some(false)
}

/// What a failure of `name` means, in plain words.
pub fn check_meaning(name: &str) -> &'static str {
    match name {
        cn::DURATION_MATCHES_EDL => "The file is not as long as the timeline: part of the picture or sound is missing.",
        cn::SOURCES_DECODABLE => "A source on the timeline is cut off or damaged, so part of the export is missing or frozen.",
        cn::LUFS => "The loudness is not at the target level.",
        cn::CAPTION_PRESENCE => "The export has no captions, or captions sit over silence.",
        cn::SILENCE_AT_EDGES => "There is silence at the start or the end.",
        cn::BLACK_OR_FROZEN_FRAMES => "There are black or frozen stretches in the picture.",
        cn::UNIFORM_BORDER => "The picture has black bars around it.",
        cn::CUT_ON_WORD => "A cut falls inside a spoken word.",
        _ => "This check did not pass.",
    }
}

const PROBLEM_ACTION: &str = "The file was written but is not a correct export of the timeline. \
Review the failed checks in the receipt. If a source is reported as cut off or damaged, import a complete copy (or replace it on the timeline), then export again.";
const WARNING_ACTION: &str =
    "The file is the timeline as edited. Open the receipt (Receipts tab, or \
verify.checks) for each check's numbers and fix, or export as is if this is intended.";

/// The verdict of a receipt's checks.
pub fn check_outcome(checks: &[CheckResult]) -> CheckOutcome {
    let mut failed: Vec<CheckProblem> = Vec::new();
    let mut unmeasured: Vec<String> = Vec::new();
    let mut not_applicable: Vec<String> = Vec::new();
    for check in checks {
        if is_not_applicable(check) {
            not_applicable.push(check.name.clone());
            continue;
        }
        if is_unmeasured(check) {
            unmeasured.push(check.name.clone());
            continue;
        }
        if check.pass {
            continue;
        }
        let integrity = INTEGRITY_CHECKS.contains(&check.name.as_str());
        failed.push(CheckProblem {
            name: check.name.clone(),
            level: if integrity { "problem" } else { "warning" }.into(),
            meaning: check_meaning(&check.name).into(),
            reason: check
                .details
                .get("reason")
                .and_then(|r| r.as_str())
                .map(String::from),
        });
    }
    failed.sort_by_key(|p| p.level != "problem");
    let problems = failed.iter().filter(|p| p.level == "problem").count();
    let names = |level: &str| {
        failed
            .iter()
            .filter(|p| p.level == level)
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (verdict, summary, action) = if problems > 0 {
        (
            "problems",
            format!(
                "The export was written but did not pass its checks ({}): {}",
                names("problem"),
                failed[0].meaning
            ),
            Some(PROBLEM_ACTION.to_string()),
        )
    } else if !failed.is_empty() {
        (
            "warnings",
            format!(
                "The export was written; {} quality check{} did not pass ({}).",
                failed.len(),
                if failed.len() == 1 { "" } else { "s" },
                names("warning")
            ),
            Some(WARNING_ACTION.to_string()),
        )
    } else if !unmeasured.is_empty() {
        (
            "unmeasured",
            format!(
                "The export was written; the measured checks passed and {} could not be measured on this computer.",
                unmeasured.len()
            ),
            None,
        )
    } else {
        (
            "passed",
            "The export was written and passed its checks.".to_string(),
            None,
        )
    };
    CheckOutcome {
        verdict: verdict.into(),
        summary,
        failed,
        unmeasured,
        not_applicable,
        action,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(name: &str, pass: bool, details: serde_json::Value) -> CheckResult {
        CheckResult {
            name: name.into(),
            pass,
            details,
            evidence: json!({}),
        }
    }

    #[test]
    fn a_cut_off_source_is_a_problem_named_with_its_way_forward() {
        // The HQ Mac run: two measured failures, quality checks unmeasured.
        let out = check_outcome(&[
            check(cn::LUFS, false, json!({"status": "unmeasured"})),
            check(cn::CAPTION_PRESENCE, true, json!({})),
            check(
                cn::DURATION_MATCHES_EDL,
                false,
                json!({"reason": "The video in the file is 1.72 s (43 frames) but the timeline is 7.62 s (190 frames)."}),
            ),
            check(
                cn::SOURCES_DECODABLE,
                false,
                json!({"reason": "cut.mp4: Only the first 1.7 s ..."}),
            ),
        ]);
        assert_eq!(out.verdict, "problems");
        assert_eq!(
            out.summary,
            "The export was written but did not pass its checks (duration_matches_edl, sources_decodable): \
             The file is not as long as the timeline: part of the picture or sound is missing."
        );
        assert_eq!(out.failed.len(), 2);
        assert!(out.failed.iter().all(|p| p.level == "problem"));
        assert_eq!(out.failed[0].reason.as_deref(), Some("The video in the file is 1.72 s (43 frames) but the timeline is 7.62 s (190 frames)."));
        assert_eq!(out.unmeasured, vec![cn::LUFS.to_string()]);
        assert!(out
            .action
            .unwrap()
            .starts_with("The file was written but is not a correct export"));
    }

    #[test]
    fn duration_mismatch_does_not_assume_a_damaged_source() {
        let out = check_outcome(&[
            check(
                cn::DURATION_MATCHES_EDL,
                false,
                json!({"reason": "The video has 442 frames but the timeline has 443."}),
            ),
            check(cn::SOURCES_DECODABLE, true, json!({})),
        ]);
        assert_eq!(out.verdict, "problems");
        assert_eq!(out.failed.len(), 1);
        let action = out.action.unwrap();
        assert!(action.contains("Review the failed checks"), "{action}");
        assert!(
            action.contains("If a source is reported as cut off or damaged"),
            "{action}"
        );
    }

    #[test]
    fn quality_failures_are_warnings_and_problems_come_first() {
        let out = check_outcome(&[
            check(cn::CAPTION_PRESENCE, false, json!({})),
            check(cn::LUFS, false, json!({})),
        ]);
        assert_eq!(out.verdict, "warnings");
        assert_eq!(
            out.summary,
            "The export was written; 2 quality checks did not pass (caption_presence, lufs)."
        );
        let mixed = check_outcome(&[
            check(cn::LUFS, false, json!({})),
            check(cn::DURATION_MATCHES_EDL, false, json!({})),
        ]);
        assert_eq!(mixed.verdict, "problems");
        assert_eq!(mixed.failed[0].name, cn::DURATION_MATCHES_EDL);
    }

    #[test]
    fn a_check_that_does_not_apply_is_listed_apart_and_never_a_warning() {
        let out = check_outcome(&[
            not_applicable_check(
                cn::CAPTION_PRESENCE,
                "the export burns in captions",
                "This export has no captions.",
            ),
            check(cn::DURATION_MATCHES_EDL, true, json!({})),
        ]);
        assert_eq!(out.verdict, "passed");
        assert!(out.failed.is_empty());
        assert_eq!(out.not_applicable, vec![cn::CAPTION_PRESENCE.to_string()]);
        assert!(out.unmeasured.is_empty());
        assert!(out.action.is_none());
        let na = not_applicable_check(cn::LUFS, "the timeline has sound", "No sound.");
        assert!(na.pass && is_not_applicable(&na));
        assert_eq!(na.details["status"], "not_applicable");
    }

    #[test]
    fn all_passed_or_only_unmeasured_is_not_a_problem() {
        assert_eq!(
            check_outcome(&[check(cn::LUFS, true, json!({}))]).verdict,
            "passed"
        );
        let out = check_outcome(&[
            check(cn::LUFS, false, json!({"measured": false})),
            check(cn::DURATION_MATCHES_EDL, true, json!({})),
        ]);
        assert_eq!(out.verdict, "unmeasured");
        assert!(out.action.is_none());
    }
}
