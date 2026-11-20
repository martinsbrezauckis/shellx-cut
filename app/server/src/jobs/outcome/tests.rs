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

#[tokio::test]
async fn a_cancelled_job_keeps_the_progress_it_reached() {
    let bus = EventBus::new();
    let mut events = bus.subscribe();
    let manager = JobManager::new(bus);
    let job = manager.create("analysis");
    manager.progress(&job.job_id, 0.4, Some("working".into()));
    manager.cancel_by_user(&job.job_id);
    let record = manager.get(&job.job_id).unwrap();
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(record.outcome, Some(JobOutcome::Cancelled));
    assert!(
        (record.progress - 0.4).abs() < 1e-6,
        "a cancelled job must not read as complete, got {}",
        record.progress
    );
    // Earlier events (create, running progress) come first; find the terminal one.
    loop {
        let event = events.recv().await.expect("terminal progress event");
        if let crate::events::Event::JobProgress {
            progress, message, ..
        } = event
        {
            if message.as_deref() == Some("cancelled") {
                assert!((progress - 0.4).abs() < 1e-6, "event progress {progress}");
                break;
            }
        }
    }
}

#[test]
fn a_superseded_job_and_a_queued_cancel_do_not_read_as_complete() {
    let manager = JobManager::new(EventBus::new());
    let superseded = manager.create("analysis");
    manager.progress(&superseded.job_id, 0.25, None);
    manager.supersede(&superseded.job_id);
    assert_eq!(manager.get(&superseded.job_id).unwrap().progress, 0.25);
    let queued = manager.create("analysis");
    manager.cancel_by_user(&queued.job_id);
    assert_eq!(manager.get(&queued.job_id).unwrap().progress, 0.0);
    // A failed job did not finish either: it keeps what it reached.
    let failed = manager.create("analysis");
    manager.progress(&failed.job_id, 0.6, None);
    manager.fail(&failed.job_id, CutError::new("failed", "boom", "fixture"));
    assert_eq!(manager.get(&failed.job_id).unwrap().progress, 0.6);
}

/// Row 210: `outcome` answers "did the job do what was asked". A job that ran
/// to its end but did not fully deliver is `partial`, never `succeeded`; a
/// clean or merely warned job stays `succeeded`. Read as an agent reads it.
#[test]
fn outcome_says_whether_the_job_delivered() {
    let manager = JobManager::new(EventBus::new());
    let read = |id: &str| serde_json::to_value(manager.get(id).unwrap()).unwrap();
    let cases: [(&str, fn(&JobManager, &str)); 4] = [
        ("succeeded", |m, id| m.finish(id, serde_json::json!({}))),
        ("succeeded", |m, id| {
            m.finish_with_warnings(id, serde_json::json!({}))
        }),
        ("partial", |m, id| {
            m.finish_with_problems(id, serde_json::json!({}))
        }),
        ("partial", |m, id| {
            m.finish_with_failed_items(id, serde_json::json!({}), 1, 3)
        }),
    ];
    for (expected, end) in cases {
        let job = manager.create("render_queue");
        end(&manager, &job.job_id);
        let record = read(&job.job_id);
        assert_eq!(record["state"], "done", "{record}");
        assert_eq!(record["outcome"], expected, "{record}");
    }
}
