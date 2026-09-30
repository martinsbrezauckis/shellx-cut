//! Imported jobs cannot grant recovery ownership of another directory.

use super::{quarantine, recover, JobRecord};
use crate::events::EventBus;
use crate::jobs::{JobManager, JobOutcome, JobState};
use cut_core::error_codes;
use std::path::Path;

fn queued_record() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "job_id": "job_002", "kind": "render", "state": "queued",
        "progress": 0.0, "created_ts": "old", "updated_ts": "old"
    }))
    .unwrap()
}

fn assert_outside_untouched(outside: &Path) {
    assert_eq!(
        std::fs::read(outside.join("preferences.json")).unwrap(),
        b"{\"keep\":true}"
    );
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn linked_jobs_recovery_and_direct_attach_preserve_outside_and_previous_attachment() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("imported");
    let outside = root.path().join("outside");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("preferences.json"), b"{\"keep\":true}").unwrap();
    std::os::unix::fs::symlink(&outside, project.join("jobs")).unwrap();
    assert_eq!(
        recover(&project).err().unwrap().code,
        error_codes::INVALID_ARGS
    );
    assert_outside_untouched(&outside);

    let prior = root.path().join("prior");
    std::fs::create_dir(&prior).unwrap();
    let manager = JobManager::new(EventBus::new());
    manager.attach_project(&prior).unwrap();
    let job = manager.create("render");
    assert_eq!(
        manager.attach_project(&project).unwrap_err().code,
        error_codes::INVALID_ARGS
    );
    assert_eq!(manager.get(&job.job_id).unwrap().state, JobState::Queued);
    manager.finish(&job.job_id, serde_json::json!({"ok":true}));
    let persisted: JobRecord = serde_json::from_slice(
        &std::fs::read(prior.join("jobs").join(format!("{}.json", job.job_id))).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted.state, JobState::Done);
    assert_outside_untouched(&outside);
}

#[cfg(unix)]
#[test]
fn linked_quarantine_preflight_preserves_valid_and_corrupt_records() {
    let root = tempfile::tempdir().unwrap();
    let jobs = root.path().join("jobs");
    let outside = root.path().join("outside");
    std::fs::create_dir(&jobs).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("preferences.json"), b"{\"keep\":true}").unwrap();
    std::os::unix::fs::symlink(&outside, jobs.join("quarantine")).unwrap();
    let queued = queued_record();
    std::fs::write(jobs.join("job_002.json"), &queued).unwrap();
    // Also prove refusal when no record needs quarantine: preflight cannot be
    // deferred until an invalid record is encountered.
    assert_eq!(
        recover(root.path()).err().unwrap().code,
        error_codes::INVALID_ARGS
    );
    assert_eq!(std::fs::read(jobs.join("job_002.json")).unwrap(), queued);
    std::fs::write(jobs.join("job_007.json"), b"corrupt").unwrap();
    let manager = JobManager::new(EventBus::new());
    assert_eq!(
        manager.attach_project(root.path()).unwrap_err().code,
        error_codes::INVALID_ARGS
    );
    assert!(manager.list().is_empty());
    assert_eq!(
        std::fs::read(jobs.join("job_007.json")).unwrap(),
        b"corrupt"
    );
    assert_eq!(std::fs::read(jobs.join("job_002.json")).unwrap(), queued);
    assert_outside_untouched(&outside);
}

#[cfg(unix)]
#[test]
fn quarantine_rechecks_both_directories_before_creating_or_moving() {
    for relative in ["jobs", "jobs/quarantine"] {
        let root = tempfile::tempdir().unwrap();
        let jobs = root.path().join("jobs");
        std::fs::create_dir(&jobs).unwrap();
        recover(root.path()).unwrap();
        let outside = root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("preferences.json"), b"{\"keep\":true}").unwrap();
        let link = root.path().join(relative);
        if relative == "jobs" {
            std::fs::remove_dir(&jobs).unwrap();
        }
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let source = jobs.join("job_007.json");
        std::fs::write(&source, b"corrupt").unwrap();
        assert_eq!(
            quarantine(&jobs, &source, "job_007.json", "invalid")
                .unwrap_err()
                .code,
            error_codes::INVALID_ARGS
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"corrupt");
        assert_eq!(
            std::fs::read(outside.join("preferences.json")).unwrap(),
            b"{\"keep\":true}"
        );
        assert_eq!(
            std::fs::read_dir(&outside).unwrap().count(),
            if relative == "jobs" { 2 } else { 1 }
        );
    }
}

#[cfg(unix)]
#[test]
fn recovery_rejects_dangling_job_directory_links_without_creating_targets() {
    for relative in ["jobs", "jobs/quarantine"] {
        let root = tempfile::tempdir().unwrap();
        let link = root.path().join(relative);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        let target = root.path().join("absent");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            recover(root.path()).err().unwrap().code,
            error_codes::INVALID_ARGS
        );
        assert!(!target.exists());
    }
}

#[test]
fn recovery_rejects_non_directory_job_roots_without_mutation() {
    for relative in ["jobs", "jobs/quarantine"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"keep").unwrap();
        assert_eq!(
            recover(root.path()).err().unwrap().code,
            error_codes::INVALID_ARGS
        );
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
    }
}

#[test]
fn plain_job_roots_preserve_quarantine_notices_sequence_and_interrupted_recovery() {
    let root = tempfile::tempdir().unwrap();
    assert!(recover(root.path()).unwrap().records.is_empty());
    let jobs = root.path().join("jobs");
    std::fs::write(jobs.join("job_002.json"), queued_record()).unwrap();
    std::fs::write(jobs.join("job_021.json"), b"corrupt").unwrap();
    let recovered = recover(root.path()).unwrap();
    assert_eq!(recovered.next_seq, 21);
    assert_eq!(recovered.notices.len(), 1);
    let notice = &recovered.notices[0];
    assert_eq!(notice.code, "job_record_quarantined");
    assert_eq!(notice.record, "job_021.json");
    assert_eq!(
        std::fs::read(jobs.join(&notice.quarantine)).unwrap(),
        b"corrupt"
    );
    assert!(!jobs.join("job_021.json").exists());
    assert_eq!(recovered.records[0].state, JobState::Failed);
    assert_eq!(recovered.records[0].outcome, Some(JobOutcome::Interrupted));
    let persisted: JobRecord =
        serde_json::from_slice(&std::fs::read(jobs.join("job_002.json")).unwrap()).unwrap();
    assert_eq!(persisted.state, JobState::Failed);
    assert_eq!(persisted.outcome, Some(JobOutcome::Interrupted));
    assert!(recover(root.path()).unwrap().notices.is_empty());
}
