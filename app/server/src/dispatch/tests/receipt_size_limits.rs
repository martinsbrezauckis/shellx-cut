//! Every consumer of a render receipt reads it with one size limit. A project
//! received from someone else may hold a huge file at `receipts/render_001.json`;
//! these tests prove each consumer refuses it with a plain message naming the
//! file and the limit, without reading it into memory, and leaves it unchanged.

use super::{
    judge_fixture, lock_agent_cli_env, test_actor, write_stub_adapter, TestAdapterPythonEnv,
    JUDGE_ENV_LOCK,
};
use crate::dispatch::{attach_judge_to_receipt, dispatch, rendering, verify_judge, verify_rerun};
use cut_core::{CutError, VerbResult};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const LIMIT: u64 = 16 * 1024 * 1024;

/// Turn the fixture receipt into a VALID receipt one byte over the limit: the
/// same JSON followed by whitespace. Before the limit existed every consumer
/// read and accepted it whole.
fn pad_receipt_over_limit(rpath: &Path) {
    let mut text = std::fs::read_to_string(rpath).unwrap();
    text.push_str(&" ".repeat((LIMIT as usize + 1).saturating_sub(text.len())));
    std::fs::write(rpath, &text).unwrap();
    assert_eq!(std::fs::metadata(rpath).unwrap().len(), LIMIT + 1);
}

fn assert_size_refusal(error: &CutError) {
    assert!(
        error.message.contains("receipts/render_001.json") && error.message.contains("16 MiB"),
        "the refusal names the file and the limit: {error:?}"
    );
    assert!(
        error
            .suggested_action
            .as_deref()
            .is_some_and(|action| action.contains("receipts/render_001.json")),
        "the refusal says what to do: {error:?}"
    );
}

fn refused(response: VerbResult) -> CutError {
    assert!(!response.ok, "an oversized receipt must be refused");
    response.error.expect("refusal carries an error")
}

#[tokio::test]
async fn verify_checks_refuses_an_oversized_selected_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    pad_receipt_over_limit(&rpath);
    for args in [json!({"render_id": "render_001"}), json!({})] {
        let error = refused(dispatch(&state, "verify.checks", args, test_actor()).await);
        assert_size_refusal(&error);
    }
    assert_eq!(std::fs::metadata(&rpath).unwrap().len(), LIMIT + 1);
}

/// Runs `verify.judge` with a stand-in reviewer, so a review that wrongly
/// starts never reaches a real provider.
async fn judge_with_stand_in(
    state: &crate::state::AppState,
    dir: &Path,
    args: Value,
) -> Result<VerbResult, CutError> {
    let stub = write_stub_adapter(
        &dir.join("stand_in_adapter.py"),
        r#"{"schema": "shellx-cut/judge-review/1", "status": "not_run", "not_run_reason": "stand-in", "review": None}"#,
    );
    std::env::set_var("CUTD_JUDGE_ADAPTER", &stub);
    let result = verify_judge(state, args).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    result
}

#[tokio::test]
async fn verify_judge_refuses_an_oversized_selected_receipt_before_any_job() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    pad_receipt_over_limit(&rpath);
    let error = judge_with_stand_in(&state, dir.path(), json!({"render_id": "render_001"}))
        .await
        .expect_err("an oversized receipt must be refused");
    assert_size_refusal(&error);
    assert!(state.jobs.list().is_empty(), "no review job may start");
    assert_eq!(std::fs::metadata(&rpath).unwrap().len(), LIMIT + 1);
}

#[tokio::test]
async fn attaching_a_review_refuses_an_oversized_receipt_and_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    pad_receipt_over_limit(&rpath);
    let receipts = rpath.parent().unwrap();
    let error = attach_judge_to_receipt(&state, receipts, &rpath, json!({"status": "ok"}))
        .expect_err("an oversized receipt must be refused");
    assert_size_refusal(&error);
    assert_eq!(std::fs::metadata(&rpath).unwrap().len(), LIMIT + 1);
}

#[tokio::test]
async fn attaching_a_review_never_writes_a_receipt_past_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let before = std::fs::read(&rpath).unwrap();
    let receipts = rpath.parent().unwrap();
    let huge = json!({"status": "completed", "note": "x".repeat(LIMIT as usize)});
    let error = attach_judge_to_receipt(&state, receipts, &rpath, huge)
        .expect_err("a receipt the engine could not read back must not be written");
    assert!(error.message.contains("too large"), "{error:?}");
    assert_eq!(std::fs::read(&rpath).unwrap(), before);
}

#[tokio::test]
async fn review_handoff_refuses_an_oversized_selected_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    pad_receipt_over_limit(&rpath);
    let out = dir.path().join("handoff.cutreview");
    let error = refused(
        dispatch(
            &state,
            "comment.export",
            json!({"path": out, "allow_stale": true}),
            test_actor(),
        )
        .await,
    );
    assert_size_refusal(&error);
    assert!(!out.exists());
    let receipts = rpath.parent().unwrap();
    let error = rendering::read_receipt(receipts, Some("render_001")).unwrap_err();
    assert_size_refusal(&error);
}

#[tokio::test]
async fn verify_rerun_refuses_an_oversized_selected_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    pad_receipt_over_limit(&rpath);
    let error = verify_rerun(&state, json!({"render_id": "render_001"}))
        .await
        .expect_err("an oversized receipt must be refused");
    assert_size_refusal(&error);
    assert!(state.jobs.list().is_empty(), "no rerun job may start");
}

/// The limit is applied from the file's size before reading: a sparse
/// 64 GiB receipt (no disk space used) is refused at once. Reading it would
/// take minutes and more memory than the test machine has. Unix only: NTFS
/// would reserve the space.
#[cfg(unix)]
#[tokio::test]
async fn a_huge_sparse_receipt_is_refused_at_once() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&rpath)
        .unwrap()
        .set_len(64 * 1024 * 1024 * 1024)
        .unwrap();
    let started = Instant::now();
    let error = refused(dispatch(&state, "verify.checks", json!({}), test_actor()).await);
    let error_judge = judge_with_stand_in(&state, dir.path(), json!({}))
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_size_refusal(&error);
    assert_size_refusal(&error_judge);
}

#[tokio::test]
async fn latest_receipt_is_the_highest_render_number() {
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let receipts = rpath.parent().unwrap();
    let mut receipt: Value =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    for id in ["render_999", "render_1000"] {
        receipt["render_id"] = json!(id);
        std::fs::write(receipts.join(format!("{id}.json")), receipt.to_string()).unwrap();
    }
    let started = Instant::now();
    let response = dispatch(&state, "verify.checks", json!({}), test_actor()).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.result.unwrap()["render_id"], "render_1000");
}
