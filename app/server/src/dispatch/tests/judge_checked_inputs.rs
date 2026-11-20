//! The detached reviewer gets exactly the inputs verify.judge checked. Another
//! program may change the project folder after the check: these stand-in
//! reviewers first replace the checked files themselves (as that program
//! would), then read what they were given.

use super::{
    judge_fixture, lock_agent_cli_env, test_actor, wait_job, TestAdapterPythonEnv, JUDGE_ENV_LOCK,
};
use crate::dispatch::dispatch;
use crate::jobs::{JobRecord, JobState};
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const CANARY: &str = "SYNTHETIC_REPLACED_FACTS_CANARY";
const CHECKED_WORD: &str = "checkedtranscriptword";

fn report(hash: &str, word: &str) -> String {
    json!({
        "schema": "shellx-cut/perception/1",
        "asset_hash": hash,
        "words": {"words": [{"word": word, "start_ms": 0, "end_ms": 100}]},
    })
    .to_string()
}

/// A stand-in reviewer that, when it starts, overwrites `replace_report`
/// (with the same `asset_hash`, other facts) and `replace_render` (other
/// bytes), then records the analysis text and the render bytes it was given.
fn swapping_adapter(
    dir: &Path,
    replace_report: &Path,
    hash: &str,
    replace_render: &Path,
) -> (String, PathBuf) {
    let script = dir.join("swapping_adapter.py");
    let seen = dir.join("seen.json");
    let code = format!(
        r#"import hashlib, json, sys
args = sys.argv[1:]
def val(flag):
    return args[args.index(flag) + 1] if flag in args else None
if val("--render") is not None:
    open({replace_report:?}, "w").write({planted:?})
    open({replace_render:?}, "wb").write(b"swapped-render-bytes")
    report = val("--perception")
    text = open(report, encoding="utf-8").read() if report else ""
    render = open(val("--render"), "rb").read()
    line = json.dumps({{"perception": report, "render": val("--render"), "text": text,
                       "render_sha256": "sha256:" + hashlib.sha256(render).hexdigest()}}) + "\n"
    open({seen:?}, "a", encoding="utf-8").write(line)
env = {{"schema": "shellx-cut/judge-review/1", "status": "completed",
       "backend": {{"name": "cli", "provider": "stub", "watched": True, "listened": False}},
       "review": {{"verdict": "pass", "issues": [], "confidence": 0.9, "summary": "stub"}},
       "not_run_reason": None}}
out = val("--out")
if out:
    open(out, "w").write(json.dumps(env))
print(json.dumps(env))
"#,
        replace_report = replace_report.display().to_string(),
        planted = report(hash, CANARY),
        replace_render = replace_render.display().to_string(),
        seen = seen.display().to_string(),
    );
    std::fs::write(&script, code).unwrap();
    (script.display().to_string(), seen)
}

async fn run_review(state: &AppState, adapter: &str) -> JobRecord {
    std::env::set_var("CUTD_JUDGE_ADAPTER", adapter);
    let response = dispatch(state, "verify.judge", json!({}), test_actor()).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    assert!(response.ok, "{:?}", response.error);
    let job = response.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    wait_job(state, &job, 30).await
}

fn the_review(seen: &Path) -> Value {
    let text = std::fs::read_to_string(seen).expect("the reviewer ran");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one review: {text}");
    serde_json::from_str(lines[0]).unwrap()
}

#[tokio::test]
async fn an_analysis_replaced_after_its_check_never_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let receipts = rpath.parent().unwrap().to_path_buf();
    let report_path = receipts.join("render_001.output.perception.json");
    std::fs::write(
        &report_path,
        report(super::JUDGE_FIXTURE_HASH, CHECKED_WORD),
    )
    .unwrap();
    // As render.final does after analysing the render.
    cut_media::cache_trust::remember(&report_path);
    let untouched = dir.path().join("untouched-render-target");
    let (adapter, seen) = swapping_adapter(
        dir.path(),
        &report_path,
        super::JUDGE_FIXTURE_HASH,
        &untouched,
    );

    let record = run_review(&state, &adapter).await;

    assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    let review = the_review(&seen);
    let text = review["text"].as_str().unwrap();
    assert!(
        text.contains(CHECKED_WORD) && !text.contains(CANARY),
        "the reviewer read facts written after the check: {text}"
    );
    let given = PathBuf::from(review["perception"].as_str().unwrap());
    let project = receipts.parent().unwrap().canonicalize().unwrap();
    assert!(
        given
            .canonicalize()
            .unwrap()
            .starts_with(project.join(".scratch/judge/render_001")),
        "the reviewer gets the run's own copy, not the project file: {given:?}"
    );
}

#[tokio::test]
async fn a_render_outside_the_project_replaced_after_its_check_is_not_reviewed() {
    use sha2::{Digest, Sha256};
    let _output_dir = crate::output_paths::SESSION_OUTPUT_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let saved = dir.path().join("save-as/render.mp4");
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, b"saved-as-render-bytes").unwrap();
    let hash = format!("sha256:{:x}", Sha256::digest(b"saved-as-render-bytes"));
    let mut receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    receipt.output_path = saved.display().to_string();
    receipt.output_hash = hash.clone();
    std::fs::write(&rpath, serde_json::to_string_pretty(&receipt).unwrap()).unwrap();
    let unused_report = dir.path().join("unused-report.json");
    let (adapter, seen) = swapping_adapter(dir.path(), &unused_report, &hash, &saved);

    let record = run_review(&state, &adapter).await;

    assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    let review = the_review(&seen);
    assert_eq!(
        review["render_sha256"], hash,
        "the reviewer read a render changed after its check"
    );
    assert_ne!(review["render"], saved.display().to_string());
    assert!(
        !Path::new(review["render"].as_str().unwrap()).exists(),
        "the render copy is removed after the review"
    );
}

fn receipt_judge(rpath: &Path) -> Value {
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(rpath).unwrap()).unwrap();
    receipt
        .judge
        .expect("the outcome is recorded on the receipt")
}

/// A render in the project that is not the file its receipt names (for
/// example replaced in a received project) is never reviewed.
#[tokio::test]
async fn a_render_in_the_project_that_is_not_its_receipts_file_is_not_reviewed() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    std::fs::write(&receipt.output_path, b"another video entirely").unwrap();
    let unused = dir.path().join("unused");
    let (adapter, seen) = swapping_adapter(dir.path(), &unused, "sha256:x", &unused);

    let record = run_review(&state, &adapter).await;

    assert_eq!(record.state, JobState::Failed, "{:?}", record.result);
    let error = record.error.expect("a plain refusal");
    assert!(
        error
            .message
            .contains("does not match its receipt before the review"),
        "{error:?}"
    );
    assert!(
        error.cause.contains(super::JUDGE_FIXTURE_HASH),
        "names what differs: {error:?}"
    );
    assert!(!seen.exists(), "the reviewer must not start");
    assert_eq!(receipt_judge(&rpath)["status"], "error");
}

/// A render in the project changed while the reviewer runs: the verdict is
/// about other bytes, so it is discarded instead of attached.
#[tokio::test]
async fn a_verdict_on_a_render_changed_during_the_review_is_discarded() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    let render = PathBuf::from(&receipt.output_path);
    let unused = dir.path().join("unused");
    let (adapter, seen) = swapping_adapter(dir.path(), &unused, "sha256:x", &render);

    let record = run_review(&state, &adapter).await;

    assert!(seen.exists(), "the reviewer ran");
    assert_eq!(record.state, JobState::Failed, "{:?}", record.result);
    let judge = receipt_judge(&rpath);
    assert_eq!(judge["status"], "error", "{judge}");
    assert!(judge["review"].is_null(), "no verdict is attached: {judge}");
    assert!(
        judge["not_run_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("after the review")
                && reason.contains("verdict was discarded")),
        "{judge}"
    );
}
