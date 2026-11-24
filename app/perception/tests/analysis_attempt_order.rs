use cut_core::{error_codes, CutError};
use cut_perception::analysis_attempts::{load_attempts, AttemptRecorder, ExecutionState};
use serde_json::json;

fn event(state: &str) -> String {
    format!(
        "CUT_ATTEMPT {}",
        json!({"instrument":"scenes", "provider":"perception", "state":state,
        "at_unix_ms":1234, "tool_version":"Python 3.12.9", "error":null})
    )
}

#[test]
fn a_failed_provider_without_details_still_has_an_explanation() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], false);
    recorder.observe(&event("failed"));
    let record = recorder.finish(None);
    assert!(record.protocol_error.is_some());
    assert!(record.instruments[0].error.is_some());
}

#[test]
fn a_late_success_cannot_erase_a_terminal_failure() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], true);
    recorder.observe(&event("failed"));
    recorder.observe(&event("succeeded"));
    assert_eq!(
        recorder.snapshot().instruments[0].state,
        ExecutionState::Failed
    );
    assert!(recorder.snapshot().protocol_error.is_some());
}

#[test]
fn a_running_provider_that_never_answers_is_not_passed_or_left_running() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], true);
    recorder.observe(&event("running"));
    let final_record = recorder.finish(None);
    assert_eq!(final_record.instruments[0].state, ExecutionState::Failed);
    assert!(final_record.instruments[0].ran);
    assert!(final_record.protocol_error.is_some());
    // The process can have completed successfully; this is a provider record error.
    assert!(final_record.error.is_none());
}

#[test]
fn repeated_completion_does_not_clear_an_original_process_error() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], true);
    let error = CutError::new(error_codes::SIDECAR, "could not run", "original reason");
    recorder.finish(Some(&error));
    assert_eq!(recorder.finish(None).error, Some(error));
}

#[test]
fn concurrent_jobs_do_not_lose_each_others_records_or_mix_projects() {
    let dir = tempfile::tempdir().unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], true);
                recorder.asked();
                recorder.observe(&event("succeeded"));
                assert!(recorder.finish(None).storage_error.is_none());
            });
        }
    });
    let records = load_attempts(dir.path(), "a1", "hash").unwrap();
    assert_eq!(records.len(), 8);
    assert_eq!(
        records
            .iter()
            .map(|r| &r.attempt_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        8
    );
    let other_project = tempfile::tempdir().unwrap();
    assert!(load_attempts(other_project.path(), "a1", "hash")
        .unwrap()
        .is_empty());
}

#[test]
fn a_deleted_project_is_never_recreated_to_record_an_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("deleted-project/receipts");
    let recorder = AttemptRecorder::new(&missing, "a1", "hash", &["scenes"], true);
    assert!(recorder.finish(None).storage_error.is_some());
    assert!(!missing.exists());
}
