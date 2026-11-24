use cut_core::{error_codes, CutError};
use cut_perception::analysis_attempts::{load_attempts, AttemptRecorder};

#[test]
fn failed_write_retention_has_a_byte_budget_without_truncating_the_latest_error() {
    let dir = tempfile::tempdir().unwrap();
    let error = CutError::new(
        error_codes::SIDECAR,
        "provider failed",
        "x".repeat(768 * 1024),
    );
    for n in 0..8 {
        let id = format!("a{n}");
        std::fs::create_dir(dir.path().join(format!("{id}.analysis-attempts.json"))).unwrap();
        let recorder = AttemptRecorder::new(dir.path(), &id, "hash", &["scenes"], true);
        let record = recorder.finish(Some(&error));
        assert_eq!(record.error.as_ref(), Some(&error));
        assert!(record.storage_error.is_some());
    }
    let retained = (0..8)
        .filter(|n| load_attempts(dir.path(), &format!("a{n}"), "hash").is_ok())
        .count();
    assert!(
        retained < 8,
        "Failed writes must not accumulate arbitrarily large error bodies"
    );
    assert_eq!(
        load_attempts(dir.path(), "a7", "hash").unwrap()[0]
            .error
            .as_ref(),
        Some(&error)
    );
}
