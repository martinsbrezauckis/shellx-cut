//! Focused retry-lineage recovery tests.

use super::recover;
use crate::events::EventBus;
use crate::jobs::{
    JobManager, JobOutcome, JobOutcomeReason, JobRecord, JobRetry, JobState,
    ScreenRecordExportRetryDescriptor, ScreenRecordExportRetryFormat,
};

fn record(job_id: &str, state: JobState) -> JobRecord {
    JobRecord {
        job_id: job_id.into(),
        kind: "render".into(),
        state,
        completion: None,
        outcome: None,
        outcome_reason: None,
        progress: if matches!(state, JobState::Done | JobState::Failed) {
            1.0
        } else {
            0.0
        },
        message: None,
        queue: None,
        waiting_on: None,
        retry: None,
        created_ts: "2026-08-08T00:00:00.000Z".into(),
        updated_ts: "2026-08-08T00:00:00.000Z".into(),
        result: None,
        error: None,
        persistence_error: None,
    }
}

fn screen_record_retry_descriptor(plan: &str) -> ScreenRecordExportRetryDescriptor {
    ScreenRecordExportRetryDescriptor {
        project_revision: "op_000001".into(),
        source: "cache/screen_record/capture/source.mp4".into(),
        plan: plan.into(),
        format: ScreenRecordExportRetryFormat::Mp4,
        inputs: Vec::new(),
        system_audio_offset_ms: 0,
    }
}

fn retry_record(job_id: &str, state: JobState, retry: JobRetry) -> JobRecord {
    let mut record = record(job_id, state);
    record.kind = "screen_record_export".into();
    record.retry = Some(retry);
    record
}

#[test]
fn retry_recovery_reconciles_a_partial_admission_and_durably_continues_from_child() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = true;
    parent_retry.reason = None;
    let child_retry = parent_retry.admitted_child("job_001");
    let parent = retry_record("job_001", JobState::Failed, parent_retry);
    let child = retry_record("job_002", JobState::Queued, child_retry);
    std::fs::write(
        jobs.join("job_001.json"),
        serde_json::to_vec(&parent).unwrap(),
    )
    .unwrap();
    std::fs::write(
        jobs.join("job_002.json"),
        serde_json::to_vec(&child).unwrap(),
    )
    .unwrap();

    let first_reopen = JobManager::new(EventBus::new());
    first_reopen.attach_project(project.path()).unwrap();
    let repaired_parent = first_reopen.get("job_001").unwrap();
    let recovered_child = first_reopen.get("job_002").unwrap();
    let parent_retry = repaired_parent.retry.as_ref().unwrap();
    assert!(!parent_retry.eligible);
    assert_eq!(
        parent_retry.reason.as_deref(),
        Some("a retry attempt has already been admitted")
    );
    assert_eq!(parent_retry.retried_by.as_deref(), Some("job_002"));
    assert_eq!(recovered_child.state, JobState::Failed);
    assert_eq!(recovered_child.outcome, Some(JobOutcome::Interrupted));
    assert_eq!(
        recovered_child.outcome_reason,
        Some(JobOutcomeReason::RestartInterrupted)
    );
    assert!(recovered_child.retry.as_ref().unwrap().eligible);

    let persisted_parent: JobRecord =
        serde_json::from_slice(&std::fs::read(jobs.join("job_001.json")).unwrap()).unwrap();
    assert_eq!(
        persisted_parent.retry.unwrap().retried_by.as_deref(),
        Some("job_002")
    );
    assert_eq!(
        first_reopen.admit_retry("job_001").unwrap_err().code,
        cut_core::error_codes::CONFLICT
    );
    let grandchild = first_reopen.admit_retry("job_002").unwrap();
    assert_eq!(grandchild.job_id, "job_003");
    assert_eq!(grandchild.retry.as_ref().unwrap().attempt, 3);
    drop(first_reopen);

    // The second reopen proves both the repaired first parent and the newly
    // consumed recovered child remain durable; only the newest child becomes
    // retryable after its own queued restart interruption.
    let second_reopen = JobManager::new(EventBus::new());
    second_reopen.attach_project(project.path()).unwrap();
    let parent_after_second_reopen = second_reopen.get("job_001").unwrap();
    let child_after_second_reopen = second_reopen.get("job_002").unwrap();
    let grandchild_after_second_reopen = second_reopen.get("job_003").unwrap();
    assert!(!parent_after_second_reopen.retry.as_ref().unwrap().eligible);
    assert_eq!(
        parent_after_second_reopen
            .retry
            .as_ref()
            .unwrap()
            .retried_by
            .as_deref(),
        Some("job_002")
    );
    assert!(!child_after_second_reopen.retry.as_ref().unwrap().eligible);
    assert_eq!(
        child_after_second_reopen
            .retry
            .as_ref()
            .unwrap()
            .retried_by
            .as_deref(),
        Some("job_003")
    );
    assert_eq!(grandchild_after_second_reopen.state, JobState::Failed);
    assert_eq!(
        grandchild_after_second_reopen.outcome_reason,
        Some(JobOutcomeReason::RestartInterrupted)
    );
    assert!(
        grandchild_after_second_reopen
            .retry
            .as_ref()
            .unwrap()
            .eligible
    );
    assert_eq!(
        second_reopen.admit_retry("job_002").unwrap_err().code,
        cut_core::error_codes::CONFLICT
    );
    assert_eq!(
        second_reopen
            .admit_retry("job_003")
            .unwrap()
            .retry
            .as_ref()
            .unwrap()
            .attempt,
        4
    );
}

#[test]
fn inconsistent_retry_descriptor_refuses_recovery_without_rewriting_the_pair() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = true;
    parent_retry.reason = None;
    let mut child_retry = parent_retry.admitted_child("job_001");
    child_retry.descriptor = Some(crate::jobs::JobRetryDescriptor::ScreenRecordExport(
        screen_record_retry_descriptor("plans/changed.json"),
    ));
    let parent = retry_record("job_001", JobState::Failed, parent_retry);
    let child = retry_record("job_002", JobState::Queued, child_retry);
    std::fs::write(
        jobs.join("job_001.json"),
        serde_json::to_vec(&parent).unwrap(),
    )
    .unwrap();
    std::fs::write(
        jobs.join("job_002.json"),
        serde_json::to_vec(&child).unwrap(),
    )
    .unwrap();

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("descriptor-mismatched retry lineage must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error.cause.contains("descriptor does not match"));

    let persisted_parent: JobRecord =
        serde_json::from_slice(&std::fs::read(jobs.join("job_001.json")).unwrap()).unwrap();
    let persisted_child: JobRecord =
        serde_json::from_slice(&std::fs::read(jobs.join("job_002.json")).unwrap()).unwrap();
    assert!(persisted_parent.retry.as_ref().unwrap().eligible);
    assert!(persisted_parent
        .retry
        .as_ref()
        .unwrap()
        .retried_by
        .is_none());
    assert_eq!(persisted_child.state, JobState::Queued);
}

#[test]
fn multiple_persisted_retry_children_refuse_recovery() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = true;
    parent_retry.reason = None;
    let child_one_retry = parent_retry.admitted_child("job_001");
    let child_two_retry = parent_retry.admitted_child("job_001");
    for record in [
        retry_record("job_001", JobState::Failed, parent_retry),
        retry_record("job_002", JobState::Queued, child_one_retry),
        retry_record("job_003", JobState::Queued, child_two_retry),
    ] {
        std::fs::write(
            jobs.join(format!("{}.json", record.job_id)),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("multiple retry children must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error.cause.contains("multiple persisted retry children"));
}

#[test]
fn one_sided_persisted_retried_by_refuses_recovery() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = false;
    parent_retry.reason = Some("a retry attempt has already been admitted".into());
    parent_retry.retried_by = Some("job_099".into());
    let parent = retry_record("job_001", JobState::Failed, parent_retry);
    std::fs::write(
        jobs.join("job_001.json"),
        serde_json::to_vec(&parent).unwrap(),
    )
    .unwrap();

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("one-sided retried_by must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error.cause.contains("missing retry child 'job_099'"));
}

#[test]
fn invalid_retry_root_refuses_recovery() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut retry = JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    retry.root_job_id = "job_099".into();
    retry.attempt = 2;
    let root = retry_record("job_001", JobState::Failed, retry);
    std::fs::write(
        jobs.join("job_001.json"),
        serde_json::to_vec(&root).unwrap(),
    )
    .unwrap();

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("invalid retry root must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error.cause.contains("must name itself and use attempt 1"));
}

#[test]
fn linked_retry_without_a_descriptor_refuses_recovery() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = true;
    parent_retry.reason = None;
    let mut child_retry = parent_retry.admitted_child("job_001");
    child_retry.descriptor = None;
    for record in [
        retry_record("job_001", JobState::Failed, parent_retry),
        retry_record("job_002", JobState::Queued, child_retry),
    ] {
        std::fs::write(
            jobs.join(format!("{}.json", record.job_id)),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("linked retry without a descriptor must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error
        .cause
        .contains("retry job 'job_002' has no retry descriptor"));
}

#[test]
fn linked_retry_with_a_wrong_descriptor_owner_refuses_recovery() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = true;
    parent_retry.reason = None;
    let child_retry = parent_retry.admitted_child("job_001");
    let mut parent = retry_record("job_001", JobState::Failed, parent_retry);
    let mut child = retry_record("job_002", JobState::Queued, child_retry);
    parent.kind = "render".into();
    child.kind = "render".into();
    for record in [parent, child] {
        std::fs::write(
            jobs.join(format!("{}.json", record.job_id)),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("wrong retry descriptor owner must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error.cause.contains("descriptor owner does not match"));
}

#[test]
fn stale_consumed_retry_reason_is_repaired_once_and_survives_a_second_reopen() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut parent_retry =
        JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    parent_retry.root_job_id = "job_001".into();
    parent_retry.eligible = false;
    parent_retry.reason = Some("stale consumed reason".into());
    parent_retry.retried_by = Some("job_002".into());
    let child_retry = parent_retry.admitted_child("job_001");
    for record in [
        retry_record("job_001", JobState::Failed, parent_retry),
        retry_record("job_002", JobState::Queued, child_retry),
    ] {
        std::fs::write(
            jobs.join(format!("{}.json", record.job_id)),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    let first_reopen = JobManager::new(EventBus::new());
    first_reopen.attach_project(project.path()).unwrap();
    let repaired_parent = first_reopen.get("job_001").unwrap();
    let repaired_retry = repaired_parent.retry.as_ref().unwrap();
    assert!(!repaired_retry.eligible);
    assert_eq!(
        repaired_retry.reason.as_deref(),
        Some("a retry attempt has already been admitted")
    );
    assert_eq!(repaired_retry.retried_by.as_deref(), Some("job_002"));
    let repaired_updated_ts = repaired_parent.updated_ts;
    drop(first_reopen);

    let second_reopen = JobManager::new(EventBus::new());
    second_reopen.attach_project(project.path()).unwrap();
    let parent_after_second_reopen = second_reopen.get("job_001").unwrap();
    let retry_after_second_reopen = parent_after_second_reopen.retry.as_ref().unwrap();
    assert!(!retry_after_second_reopen.eligible);
    assert_eq!(
        retry_after_second_reopen.reason.as_deref(),
        Some("a retry attempt has already been admitted")
    );
    assert_eq!(
        retry_after_second_reopen.retried_by.as_deref(),
        Some("job_002")
    );
    assert_eq!(parent_after_second_reopen.updated_ts, repaired_updated_ts);
}

#[test]
fn standalone_root_with_a_wrong_descriptor_owner_refuses_recovery() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut retry = JobRetry::screen_record_export(screen_record_retry_descriptor("plans/a.json"));
    retry.root_job_id = "job_001".into();
    let mut root = retry_record("job_001", JobState::Failed, retry);
    root.kind = "render".into();
    std::fs::write(
        jobs.join("job_001.json"),
        serde_json::to_vec(&root).unwrap(),
    )
    .unwrap();

    let error = match recover(project.path()) {
        Err(error) => error,
        Ok(_) => panic!("wrong standalone retry descriptor owner must refuse recovery"),
    };
    assert_eq!(error.code, cut_core::error_codes::CONFLICT);
    assert!(error.cause.contains("descriptor owner does not match"));
}

#[test]
fn standalone_ineligible_root_without_a_descriptor_remains_readable() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();

    let mut retry = JobRetry::ineligible("explicit Save As output");
    retry.root_job_id = "job_001".into();
    let root = retry_record("job_001", JobState::Failed, retry);
    std::fs::write(
        jobs.join("job_001.json"),
        serde_json::to_vec(&root).unwrap(),
    )
    .unwrap();

    let recovered = recover(project.path()).unwrap();
    assert_eq!(recovered.records.len(), 1);
    assert!(recovered.records[0]
        .retry
        .as_ref()
        .unwrap()
        .descriptor
        .is_none());
}
