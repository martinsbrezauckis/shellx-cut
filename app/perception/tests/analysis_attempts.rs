use cut_core::{error_codes, CutError};
use cut_perception::analysis_attempts::{load_attempts, AttemptRecorder, ExecutionState};
use serde_json::json;

#[test]
fn every_state_and_actual_tool_version_survives_a_durable_round_trip() {
    for state in [
        "running",
        "succeeded",
        "failed",
        "not_applicable",
        "not_available",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let recorder = AttemptRecorder::new(dir.path(), "a1", "sha256:first", &["silence"], true);
        recorder.asked();
        let asked = load_attempts(dir.path(), "a1", "sha256:first").unwrap();
        assert_eq!(asked[0].instruments[0].state, ExecutionState::Asked);
        recorder.observe(&format!("CUT_ATTEMPT {}", json!({
            "instrument":"silence", "provider":"perception", "state":state,
            "at_unix_ms":1234, "tool_version":"Python 3.12.9", "version_reason":null,
            "error": if state == "failed" { json!({"reason":"decoder failed"}) } else { json!(null) }
        })));
        let saved = load_attempts(dir.path(), "a1", "sha256:first").unwrap();
        let event = &saved[0].instruments[0];
        assert_eq!(serde_json::to_value(event.state).unwrap(), state);
        assert_eq!(event.tool_version.as_deref(), Some("Python 3.12.9"));
        if state == "failed" {
            assert_eq!(event.error.as_ref().unwrap()["reason"], "decoder failed");
        }
        assert!(load_attempts(dir.path(), "a1", "sha256:other")
            .unwrap()
            .is_empty());
    }
}

#[test]
fn fallback_keeps_the_basic_success_and_the_enhancement_error_separate() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["silence"], true);
    recorder.asked();
    for (provider, state) in [("ffmpeg.silencedetect", "succeeded"), ("silero", "failed")] {
        recorder.observe(&format!("CUT_ATTEMPT {}", json!({
            "instrument":"silence", "provider":provider, "state":state,
            "at_unix_ms":1234, "tool_version":null,
            "version_reason":"Version could not be read", "error":if state == "failed" { json!({"cause":"model damaged"}) } else { json!(null) }
        })));
    }
    let final_record = recorder.finish(None);
    assert!(final_record.error.is_none());
    let saved = load_attempts(dir.path(), "a1", "hash").unwrap();
    assert!(saved[0]
        .instruments
        .iter()
        .any(|i| i.provider == "ffmpeg.silencedetect" && i.state == ExecutionState::Succeeded));
    assert!(saved[0]
        .instruments
        .iter()
        .any(|i| i.provider == "silero" && i.state == ExecutionState::Failed));
    // A successful process without a parent-instrument event is not invented history.
    assert_eq!(saved[0].instruments[0].state, ExecutionState::Asked);
}

#[test]
fn process_failure_records_asked_but_not_run_and_retains_the_original_error() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes", "silence"], true);
    let error = CutError::new(
        error_codes::SIDECAR,
        "could not start",
        "configured Python is missing",
    )
    .with_suggested_action("Repair captions tools.");
    let failed = recorder.finish(Some(&error));
    assert_eq!(failed.error, Some(error));
    assert!(failed
        .instruments
        .iter()
        .all(|i| i.state == ExecutionState::Failed && i.started_at_unix_ms.is_none()));
    assert!(load_attempts(dir.path(), "a1", "hash").unwrap()[0]
        .finished_at_unix_ms
        .is_some());
}

#[test]
fn missing_legacy_history_and_a_received_untrusted_ledger_are_not_execution_evidence() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_attempts(dir.path(), "a1", "hash").unwrap().is_empty());
    let mut forged = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], false).snapshot();
    forged.instruments[0].state = ExecutionState::Succeeded;
    forged.instruments[0].ran = true;
    forged.instruments[0].tool_version = Some("forged version".into());
    std::fs::write(
        dir.path().join("a1.analysis-attempts.json"),
        serde_json::to_vec(&vec![forged]).unwrap(),
    )
    .unwrap();
    assert!(load_attempts(dir.path(), "a1", "hash").unwrap().is_empty());
}

#[test]
fn persistence_failure_is_visible_without_refusing_the_existing_analysis_result() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a1.analysis-attempts.json")).unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], true);
    recorder.asked();
    let finished = recorder.finish(None);
    assert!(finished.error.is_none());
    assert!(finished.storage_error.is_some());
    assert!(load_attempts(dir.path(), "a1", "hash").unwrap()[0]
        .storage_error
        .is_some());
    assert!(
        AttemptRecorder::new(dir.path(), "../escape", "hash", &["scenes"], true)
            .finish(None)
            .storage_error
            .is_some()
    );
}

#[test]
fn ephemeral_records_do_not_write_the_source_cache_and_late_or_invalid_events_do_not_forge_success()
{
    let dir = tempfile::tempdir().unwrap();
    let recorder = AttemptRecorder::new(dir.path(), "a1", "hash", &["scenes"], false);
    recorder.asked();
    assert!(!recorder.observe("ordinary progress"));
    assert!(recorder.observe("CUT_ATTEMPT not json"));
    assert!(recorder.finish(None).protocol_error.is_some());
    recorder.observe("CUT_ATTEMPT {}");
    assert_eq!(
        recorder.snapshot().instruments[0].state,
        ExecutionState::Asked
    );
    assert!(!dir.path().join("a1.analysis-attempts.json").exists());
}
