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

#[test]
fn cancelled_worker_can_persist_a_path_free_reconciliation_result() {
    let dir = tempfile::tempdir().unwrap();
    let manager = JobManager::new(EventBus::new());
    manager.attach_project(dir.path()).unwrap();
    let job = manager.create("cache_purge");
    manager.cancel_from_worker_with_result(
        &job.job_id,
        JobCancellationReason::CancelledByUser,
        serde_json::json!({
            "schema": "shellx-cut/cache-purge-reconciliation/1",
            "reconciliation": {"removed": {"files": 1, "bytes": 42}}
        }),
    );

    let record = manager.get(&job.job_id).unwrap();
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(record.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(record.outcome_reason, Some(JobOutcomeReason::UserCancelled));
    assert_eq!(
        record
            .result
            .as_ref()
            .and_then(|result| result["reconciliation"]["removed"]["bytes"].as_u64()),
        Some(42)
    );
    let disk: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("jobs").join(format!("{}.json", job.job_id))).unwrap(),
    )
    .unwrap();
    assert_eq!(disk["outcome"], "cancelled");
    assert_eq!(disk["result"]["reconciliation"]["removed"]["files"], 1);
}

#[test]
fn project_switch_terminal_record_accepts_the_workers_late_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let manager = JobManager::new(EventBus::new());
    manager.attach_project(dir.path()).unwrap();
    let job = manager.create("cache_purge");
    let job_id = job.job_id.clone();

    // detach_project records this terminal state before its cooperative worker
    // has drained. The worker must be able to attach its exact outcome without
    // changing the cancellation's state or reason.
    manager.cancel_for_project_switch(&job_id);
    manager.cancel_from_worker_with_result(
        &job_id,
        JobCancellationReason::ProjectSwitch,
        serde_json::json!({"reconciliation": {"removed": {"files": 1, "bytes": 64}}}),
    );

    let record = manager.get(&job_id).unwrap();
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(record.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(
        record.outcome_reason,
        Some(JobOutcomeReason::ProjectSwitchCancelled)
    );
    assert_eq!(
        record
            .result
            .as_ref()
            .and_then(|result| result["reconciliation"]["removed"]["bytes"].as_u64()),
        Some(64)
    );
    let disk: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("jobs").join(format!("{job_id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(disk["outcome_reason"], "project_switch_cancelled");
    assert_eq!(disk["result"]["reconciliation"]["removed"]["files"], 1);
    drop(manager);
    let recovered = JobManager::new(EventBus::new());
    recovered.attach_project(dir.path()).unwrap();
    assert_eq!(
        recovered
            .get(&job_id)
            .and_then(|record| record.result)
            .and_then(|result| result["reconciliation"]["removed"]["bytes"].as_u64()),
        Some(64),
        "the closed project's durable terminal accounting reopens with the same job record"
    );
}

#[test]
fn failed_worker_can_persist_known_partial_progress_without_becoming_done() {
    let manager = JobManager::new(EventBus::new());
    let job = manager.create("cache_purge");
    manager.fail_with_result(
        &job.job_id,
        CutError::new(
            "cache_ledger_write_failed",
            "ledger write failed",
            "fixture",
        ),
        serde_json::json!({"reconciliation": {"removed": {"files": 1, "bytes": 64}}}),
    );
    let record = manager.get(&job.job_id).unwrap();
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(record.outcome, Some(JobOutcome::Failed));
    assert_eq!(record.outcome_reason, Some(JobOutcomeReason::TrueFailure));
    assert_eq!(
        record
            .result
            .as_ref()
            .and_then(|result| result["reconciliation"]["removed"]["bytes"].as_u64()),
        Some(64)
    );
}
