use super::*;
use crate::events::EventBus;

#[test]
fn poisoned_lock_recovers_status_without_changing_queued_state() {
    let manager = JobManager::new(EventBus::new());
    let job = manager.create("render");
    manager.poison_lock_for_tests();

    let status = manager.get(&job.job_id).expect("status remains available");
    assert_eq!(status.state, JobState::Queued);
    assert_eq!(status.progress, 0.0);
    assert!(!manager.inner.is_poisoned());
}

#[test]
fn poisoned_lock_recovers_update_without_losing_progress() {
    let manager = JobManager::new(EventBus::new());
    let job = manager.create("render");
    manager.poison_lock_for_tests();

    manager.progress(&job.job_id, 0.25, Some("rendering".into()));

    let record = manager
        .get(&job.job_id)
        .expect("updated record remains available");
    assert_eq!(record.state, JobState::Running);
    assert_eq!(record.progress, 0.25);
    assert_eq!(record.message.as_deref(), Some("rendering"));
    assert!(!manager.inner.is_poisoned());
}

#[tokio::test]
async fn poisoned_lock_recovers_cancel_and_keeps_its_terminal_outcome() {
    let manager = JobManager::new(EventBus::new());
    let job = manager.create("render");
    manager.spawn(&job.job_id, std::future::pending());
    manager.poison_lock_for_tests();

    assert!(manager.abort(&job.job_id).await.expect("cancel recovers"));

    let record = manager
        .get(&job.job_id)
        .expect("cancelled record remains available");
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(record.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(record.outcome_reason, Some(JobOutcomeReason::UserCancelled));
    assert!(!manager.inner.is_poisoned());
}

#[test]
fn poisoned_lock_recovers_terminal_state_without_regression() {
    let manager = JobManager::new(EventBus::new());
    let job = manager.create("render");
    manager.poison_lock_for_tests();

    manager.finish(&job.job_id, serde_json::json!({"path": "output.mp4"}));

    let record = manager
        .get(&job.job_id)
        .expect("completed record remains available");
    assert_eq!(record.state, JobState::Done);
    assert_eq!(record.completion, Some(JobCompletion::Success));
    assert_eq!(record.outcome, Some(JobOutcome::Succeeded));
    assert_eq!(record.progress, 1.0);
    assert!(!manager.inner.is_poisoned());
}
