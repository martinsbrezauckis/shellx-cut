//! Cancellation while the Antigravity capability probe owns its child process.
use super::*;

#[tokio::test]
async fn assets_generate_cancel_during_capability_probe_preserves_user_outcome() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project = dir.path().join("probe-cancel.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"probe-cancel","dir":project}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let witness = dir.path().join("help-started.txt");
    let invocations = dir.path().join("generation-started.txt");
    let _cli = generation_cli::FakeGenerationCli::install_as(
        dir.path(),
        "agy",
        generation_cli::FakeGenerationCliConfig::waiting()
            .with_delayed_probe(witness.clone())
            .log_invocations(invocations.clone()),
    );
    let queued = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"cancel before generation","provider":"antigravity","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "{:?}", queued.error);
    let id = queued.result.as_ref().unwrap()["job_id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !witness.is_file() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("real --help child must start before cancellation");
    let cancelled = dispatch(&state, "jobs.cancel", json!({"job_id":id}), test_actor()).await;
    assert!(cancelled.ok, "{:?}", cancelled.error);
    let record = state.jobs.get(id).unwrap();
    assert_eq!(record.state, crate::jobs::JobState::Failed);
    assert_eq!(record.outcome, Some(crate::jobs::JobOutcome::Cancelled));
    assert_eq!(
        record.outcome_reason,
        Some(crate::jobs::JobOutcomeReason::UserCancelled)
    );
    assert_eq!(record.error.as_ref().unwrap().code, "job_cancelled");
    assert!(
        !invocations.exists(),
        "generation must not start after probe cancellation"
    );
    let runs = project.join("cache/gen/runs");
    assert!(!runs.exists() || std::fs::read_dir(runs).unwrap().next().is_none());
}

#[tokio::test]
async fn assets_generate_uncancelled_provider_failure_remains_true_failure() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"probe-failure","dir":dir.path().join("probe-failure.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let _cli = generation_cli::FakeGenerationCli::install_as(
        dir.path(),
        "agy",
        generation_cli::FakeGenerationCliConfig::waiting(),
    );
    let queued = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"no output fixture","provider":"antigravity","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "{:?}", queued.error);
    let record = wait_job(
        &state,
        queued.result.as_ref().unwrap()["job_id"].as_str().unwrap(),
        5,
    )
    .await;
    assert_eq!(record.outcome, Some(crate::jobs::JobOutcome::Failed));
    assert_eq!(
        record.outcome_reason,
        Some(crate::jobs::JobOutcomeReason::TrueFailure)
    );
    assert_eq!(record.error.as_ref().unwrap().code, "generation_failed");
}
