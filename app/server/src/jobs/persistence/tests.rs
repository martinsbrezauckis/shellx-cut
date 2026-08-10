use super::*;
use crate::jobs::{JobOutcome, JobOutcomeReason, JobState};

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
        created_ts: "2026-08-08T00:00:00.000Z".into(),
        updated_ts: "2026-08-08T00:00:00.000Z".into(),
        result: None,
        error: None,
        persistence_error: None,
    }
}

#[test]
fn atomic_write_fault_keeps_the_previous_record_and_cleans_the_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("job_001.json");
    std::fs::write(&path, b"old record").unwrap();

    let error = write_atomically_with(&path, b"new record", |_| {
        Err(io::Error::other("injected replace fault"))
    })
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(std::fs::read(&path).unwrap(), b"old record");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn corrupt_json_is_quarantined_and_disclosed() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(jobs.join("job_007.json"), b"{not json").unwrap();

    let recovered = recover(project.path()).unwrap();

    assert!(recovered.records.is_empty());
    assert_eq!(recovered.next_seq, 7);
    assert_eq!(recovered.notices.len(), 1);
    assert_eq!(recovered.notices[0].code, "job_record_quarantined");
    assert_eq!(recovered.notices[0].record, "job_007.json");
    assert!(recovered.notices[0].message.contains("invalid JSON"));
    assert!(Path::new(&recovered.notices[0].quarantine).starts_with("quarantine"));
    assert!(!jobs.join("job_007.json").exists());
    assert_eq!(
        std::fs::read_dir(jobs.join("quarantine")).unwrap().count(),
        1
    );
}

#[test]
fn recovery_uses_filename_and_record_history_to_avoid_id_collisions() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(jobs.join("job_021.json"), b"corrupt").unwrap();
    std::fs::write(
        jobs.join("job_004.json"),
        serde_json::to_vec(&record("job_020", JobState::Done)).unwrap(),
    )
    .unwrap();

    let recovered = recover(project.path()).unwrap();

    assert_eq!(recovered.next_seq, 21);
    assert!(recovered.records.is_empty());
    assert_eq!(recovered.notices.len(), 2);
}

#[test]
fn interrupted_records_are_recovered_with_an_atomic_terminal_write() {
    let project = tempfile::tempdir().unwrap();
    let jobs = project.path().join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(
        jobs.join("job_002.json"),
        serde_json::to_vec(&record("job_002", JobState::Running)).unwrap(),
    )
    .unwrap();

    let recovered = recover(project.path()).unwrap();
    let restored = recovered.records.first().unwrap();
    let persisted: JobRecord =
        serde_json::from_slice(&std::fs::read(jobs.join("job_002.json")).unwrap()).unwrap();

    assert_eq!(restored.state, JobState::Failed);
    assert_eq!(restored.message.as_deref(), Some("interrupted by restart"));
    assert_eq!(restored.outcome, Some(JobOutcome::Interrupted));
    assert_eq!(
        restored.outcome_reason,
        Some(JobOutcomeReason::RestartInterrupted)
    );
    assert_eq!(persisted.state, JobState::Failed);
    assert_eq!(persisted.outcome, Some(JobOutcome::Interrupted));
    assert_eq!(persisted.error.as_ref().unwrap().code, "job_failed");
}
