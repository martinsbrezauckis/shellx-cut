//! How a render queue job ends: its own outcome follows its deliveries.
//!
//! Large batch exporters (Adobe Media Encoder's queue, Resolve's render queue)
//! finish the queue and mark each failed item; the queue never reads as a plain
//! success when an item failed, and the summary names the count. Here:
//! - every delivery written and clean -> plain "done";
//! - written, with problems or warnings -> as `check_finish` decides;
//! - SOME failed -> "done, but N of M failed" (`completion: done_with_warnings`,
//!   `outcome_reason: items_failed`); the delivered files are real, so the job
//!   is done, and the status bar lists it like a failed job until dismissed;
//! - ALL failed -> the job FAILS (state failed, true_failure) with the first
//!   delivery's reason, and the per-delivery rows stay in the result.

use super::check_finish::{self, Finish};
use crate::state::AppState;
use cut_core::{error::codes as error_codes, CutError};
use serde_json::{json, Value};

/// What the deliveries of one queue came to.
#[derive(Default)]
pub(super) struct QueueTally {
    pub(super) succeeded: usize,
    /// "delivery N: <reason>" for each failed delivery, in order.
    pub(super) failures: Vec<String>,
    /// The first failed delivery's own error, for an all-failed queue.
    pub(super) first_error: Option<CutError>,
    pub(super) with_problems: Vec<String>,
    pub(super) with_warnings: Vec<String>,
}

impl QueueTally {
    /// Record a delivery that failed (refused to start or its render failed).
    pub(super) fn fail(&mut self, idx: usize, error: Option<CutError>) {
        let reason = error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "the render did not start".into());
        self.failures
            .push(format!("delivery {}: {reason}", idx + 1));
        if self.first_error.is_none() {
            self.first_error = error;
        }
    }
}

/// One line for the queue: "render queue: 2/3 delivered, 1 failed, ...".
fn summary(tally: &QueueTally, count: usize) -> String {
    let failed = tally.failures.len();
    let mut line = format!("render queue: {}/{count} delivered", tally.succeeded);
    if failed > 0 {
        line.push_str(&format!(", {failed} failed"));
    }
    if !tally.with_problems.is_empty() {
        line.push_str(&format!(
            ", {} written but did not pass their checks",
            tally.with_problems.len()
        ));
    }
    if !tally.with_warnings.is_empty() {
        line.push_str(&format!(", {} with warnings", tally.with_warnings.len()));
    }
    line
}

/// End the queue job by its deliveries (see the module note).
pub(super) fn finish_queue(
    st: &AppState,
    job_id: &str,
    count: usize,
    tally: QueueTally,
    jobs: Vec<Value>,
    rationale: Option<String>,
) {
    let failed = tally.failures.len();
    let summary_line = summary(&tally, count);
    let finish = if !tally.with_problems.is_empty() {
        Finish::Problems
    } else if !tally.with_warnings.is_empty() {
        Finish::Warnings
    } else {
        Finish::Plain
    };
    let warnings: Vec<String> = tally
        .failures
        .iter()
        .chain(&tally.with_problems)
        .chain(&tally.with_warnings)
        .cloned()
        .collect();
    let result = json!({
        "summary_line": summary_line,
        "count": count,
        "succeeded": tally.succeeded,
        "failed": failed,
        "with_problems": tally.with_problems.len(),
        "with_warnings": tally.with_warnings.len(),
        // "delivery N: <reason>" for each failed delivery.
        "failures": tally.failures,
        "jobs": jobs,
        "rationale": rationale,
        "warnings": warnings,
    });
    if failed > 0 && failed == count {
        let first = tally.first_error.unwrap_or_else(|| {
            CutError::new(error_codes::JOB_FAILED, "the render did not start", "")
        });
        let error = CutError::new(
            error_codes::JOB_FAILED,
            format!(
                "None of the {count} deliveries was written: {}",
                first.message
            ),
            summary_line,
        )
        .with_suggested_action(first.suggested_action.unwrap_or_else(|| {
            "Read each delivery's error in the queue result (jobs[].error), fix it, then run the queue again.".into()
        }));
        st.jobs.fail_with_result(job_id, error, result);
    } else if failed > 0 {
        st.jobs
            .finish_with_failed_items(job_id, result, failed, count);
    } else {
        check_finish::finish(st, job_id, finish, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_every_kind_of_delivery() {
        let mut tally = QueueTally {
            succeeded: 2,
            with_warnings: vec!["delivery 3: loud".into()],
            ..Default::default()
        };
        tally.fail(1, None);
        assert_eq!(
            summary(&tally, 3),
            "render queue: 2/3 delivered, 1 failed, 1 with warnings"
        );
        assert_eq!(tally.failures, ["delivery 2: the render did not start"]);
    }
}
