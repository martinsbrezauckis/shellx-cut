//! How a render job ends when its receipt is in: by the checks' verdict.
//!
//! The file is always kept. A job whose receipt verdict is `problems` (the
//! file is not the timeline) ends "done, but it did not pass its checks"
//! (`outcome_reason: checks_failed`), so the status bar lists it like a failed
//! job; `warnings` (quality checks only) ends "done with warnings"; otherwise
//! the job ends as before. The result carries the verdict as `checks` and its
//! sentence in `warnings`, so an agent reading the job alone learns it.

use crate::state::AppState;
use cut_core::CheckOutcome;
use serde_json::{json, Value};

/// How to finish, given the verdict and whether the plain path would warn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Finish {
    Plain,
    Warnings,
    Problems,
}

pub(super) fn finish_kind(outcome: Option<&CheckOutcome>, otherwise: Finish) -> Finish {
    match outcome.map(|o| o.verdict.as_str()) {
        Some("problems") => Finish::Problems,
        Some("warnings") => Finish::Warnings,
        _ => otherwise,
    }
}

/// Put the verdict on the job result: `checks` (the whole verdict) and, for a
/// warning or problem, its sentence and the way forward in `warnings`.
pub(super) fn annotate(result: &mut Value, outcome: Option<&CheckOutcome>) {
    let Some(outcome) = outcome else {
        return;
    };
    result["checks"] = json!(outcome);
    if matches!(outcome.verdict.as_str(), "problems" | "warnings") {
        let mut lines = vec![outcome.summary.clone()];
        if let Some(action) = &outcome.action {
            lines.push(action.clone());
        }
        match result.get_mut("warnings").and_then(Value::as_array_mut) {
            Some(existing) => existing.extend(lines.into_iter().map(Value::from)),
            None => result["warnings"] = json!(lines),
        }
    }
}

/// End the job the chosen way.
pub(super) fn finish(st: &AppState, job_id: &str, kind: Finish, result: Value) {
    match kind {
        Finish::Plain => st.jobs.finish(job_id, result),
        Finish::Warnings => st.jobs.finish_with_warnings(job_id, result),
        Finish::Problems => st.jobs.finish_with_problems(job_id, result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(verdict: &str) -> CheckOutcome {
        CheckOutcome {
            verdict: verdict.into(),
            summary: format!("{verdict} summary"),
            failed: vec![],
            unmeasured: vec![],
            not_applicable: vec![],
            action: Some("do this".into()),
        }
    }

    #[test]
    fn the_verdict_decides_and_rides_on_the_result() {
        assert_eq!(
            finish_kind(Some(&outcome("problems")), Finish::Plain),
            Finish::Problems
        );
        assert_eq!(
            finish_kind(Some(&outcome("warnings")), Finish::Plain),
            Finish::Warnings
        );
        assert_eq!(
            finish_kind(Some(&outcome("passed")), Finish::Plain),
            Finish::Plain
        );
        assert_eq!(
            finish_kind(Some(&outcome("unmeasured")), Finish::Warnings),
            Finish::Warnings
        );
        let mut result = json!({"warnings": ["earlier"]});
        annotate(&mut result, Some(&outcome("problems")));
        assert_eq!(result["checks"]["verdict"], "problems");
        assert_eq!(
            result["warnings"],
            json!(["earlier", "problems summary", "do this"])
        );
        let mut plain = json!({});
        annotate(&mut plain, Some(&outcome("passed")));
        assert!(plain.get("warnings").is_none());
    }
}
