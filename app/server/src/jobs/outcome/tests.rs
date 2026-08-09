use super::*;
use crate::events::EventBus;

#[test]
fn terminal_outcomes_are_persisted_without_changing_job_state() {
    let dir = tempfile::tempdir().unwrap();
    let manager = JobManager::new(EventBus::new());
    manager.attach_project(dir.path()).unwrap();

    let completed = manager.create("render");
    manager.finish(&completed.job_id, serde_json::json!({"ok": true}));
    let completed = manager.get(&completed.job_id).unwrap();
    assert_eq!(completed.state, JobState::Done);
    assert_eq!(completed.outcome, Some(JobOutcome::Succeeded));
    assert_eq!(completed.outcome_reason, Some(JobOutcomeReason::Completed));

    let failed = manager.create("render");
    manager.fail(
        &failed.job_id,
        CutError::new("job_failed", "render failed", "fixture"),
    );
    let failed = manager.get(&failed.job_id).unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(failed.outcome, Some(JobOutcome::Failed));
    assert_eq!(failed.outcome_reason, Some(JobOutcomeReason::TrueFailure));

    let cancelled = manager.create("motion_render");
    manager.fail(
        &cancelled.job_id,
        CutError::new(
            cut_core::error::codes::RENDER_CANCELLED,
            "render stopped",
            "fixture",
        ),
    );
    let cancelled = manager.get(&cancelled.job_id).unwrap();
    assert_eq!(cancelled.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(
        cancelled.outcome_reason,
        Some(JobOutcomeReason::UserCancelled)
    );

    let superseded = manager.create("render");
    manager.supersede(&superseded.job_id);
    let superseded = manager.get(&superseded.job_id).unwrap();
    assert_eq!(superseded.state, JobState::Failed);
    assert_eq!(superseded.outcome, Some(JobOutcome::Superseded));
    assert_eq!(
        superseded.outcome_reason,
        Some(JobOutcomeReason::Superseded)
    );
    assert_eq!(
        superseded.error.as_ref().map(|error| error.code.as_str()),
        Some("job_superseded")
    );

    let disk: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            dir.path()
                .join("jobs")
                .join(format!("{}.json", superseded.job_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(disk["outcome"], "superseded");
    assert_eq!(disk["outcome_reason"], "superseded");

    // jobs.status serializes this same JobRecord directly.
    let api = serde_json::to_value(&superseded).unwrap();
    assert_eq!(api["state"], "failed");
    assert_eq!(api["outcome"], "superseded");
    assert_eq!(api["outcome_reason"], "superseded");
}

#[test]
fn cooperative_worker_preserves_its_cancellation_reason() {
    let manager = JobManager::new(EventBus::new());
    for (reason, outcome, outcome_reason) in [
        (
            JobCancellationReason::CancelledByUser,
            JobOutcome::Cancelled,
            JobOutcomeReason::UserCancelled,
        ),
        (
            JobCancellationReason::ProjectSwitch,
            JobOutcome::Cancelled,
            JobOutcomeReason::ProjectSwitchCancelled,
        ),
        (
            JobCancellationReason::Restart,
            JobOutcome::Interrupted,
            JobOutcomeReason::RestartInterrupted,
        ),
        (
            JobCancellationReason::Superseded,
            JobOutcome::Superseded,
            JobOutcomeReason::Superseded,
        ),
    ] {
        let job = manager.create("render");
        manager.cancel_from_worker(&job.job_id, reason);
        let record = manager.get(&job.job_id).unwrap();
        assert_eq!(record.outcome, Some(outcome));
        assert_eq!(record.outcome_reason, Some(outcome_reason));
    }
}
