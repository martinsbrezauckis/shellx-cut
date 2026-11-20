//! verify.judge only reads and writes ordinary files that belong to the open
//! project. A project received from someone else may contain links or files
//! placed at the paths the review uses; these tests prove that such a project
//! cannot redirect the review result outside the project, cannot hand the
//! reviewer another project's analysis, and cannot stall the review.

use super::{
    judge_fixture, lock_agent_cli_env, test_actor, wait_job, TestAdapterPythonEnv, JUDGE_ENV_LOCK,
};
use crate::dispatch::dispatch;
use crate::jobs::{JobRecord, JobState};
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SENTINEL: &[u8] = b"outside-project-sentinel\n";
const CANARY: &str = "SYNTHETIC_OTHER_PROJECT_CANARY";
const ORDINARY_WORD: &str = "ordinarytranscriptword";
/// The fixture receipt's output hash (see `judge_fixture`).
const FIXTURE_HASH: &str = super::JUDGE_FIXTURE_HASH;

/// A stand-in reviewer. A REVIEW (the call that carries `--render`) appends one
/// line to `seen.json`; any other call (the doctor's `detect` probe, which can
/// reach this script through the process-wide override while another test runs)
/// records nothing, so a probe can neither fake nor corrupt the review record.
/// Each call then writes its result to `--out` with a plain write, as a careless
/// adapter would.
fn recording_adapter(dir: &Path) -> (String, PathBuf) {
    let script = dir.join("recording_adapter.py");
    let seen = dir.join("seen.json");
    let code = format!(
        r#"import json, sys
args = sys.argv[1:]
def val(flag):
    return args[args.index(flag) + 1] if flag in args else None
report = val("--perception")
text = open(report, encoding="utf-8").read() if report else ""
if val("--render") is not None:
    line = json.dumps({{"perception": report, "render": val("--render"), "text": text}}) + "\n"
    with open({seen:?}, "a", encoding="utf-8") as record:
        record.write(line)
env = {{"schema": "shellx-cut/judge-review/1", "status": "completed",
       "backend": {{"name": "cli", "provider": "stub", "watched": True, "listened": False}},
       "review": {{"verdict": "pass", "issues": [], "confidence": 0.9, "summary": "stub"}},
       "not_run_reason": None}}
out = val("--out")
if out:
    open(out, "w").write(json.dumps(env))
print(json.dumps(env))
"#,
        seen = seen.display().to_string(),
    );
    std::fs::write(&script, code).unwrap();
    (script.display().to_string(), seen)
}

/// The one review the stand-in saw. Asserts exactly one: a retry or a second
/// run of the reviewer would send the person's material to the provider twice.
fn the_one_review(seen: &Path) -> Value {
    let text = std::fs::read_to_string(seen).expect("the reviewer ran");
    let reviews: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(reviews.len(), 1, "exactly one review must run: {text}");
    reviews.into_iter().next().unwrap()
}

fn report(hash: &str, word: &str) -> String {
    json!({
        "schema": "shellx-cut/perception/1",
        "asset_hash": hash,
        "words": {"words": [{"word": word, "start_ms": 0, "end_ms": 100}]},
    })
    .to_string()
}

fn project_paths(rpath: &Path) -> (PathBuf, PathBuf) {
    let receipts = rpath.parent().unwrap().to_path_buf();
    let project = receipts.parent().unwrap().to_path_buf();
    (project, receipts)
}

fn review_leaf(project: &Path) -> PathBuf {
    let bundle = project.join(".scratch/judge/render_001");
    std::fs::create_dir_all(&bundle).unwrap();
    bundle.join("envelope.json")
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

fn receipt_judge(rpath: &Path) -> Value {
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(rpath).unwrap()).unwrap();
    receipt
        .judge
        .expect("the review outcome is recorded on the receipt")
}

/// The run stopped before the reviewer started, said so in plain words, and
/// left the same explanation on the receipt.
fn assert_refused(record: &JobRecord, rpath: &Path, seen: &Path, subject: &str) {
    assert_eq!(record.state, JobState::Failed, "{:?}", record.result);
    let error = record.error.as_ref().expect("refusal is reported");
    assert!(
        error.message.starts_with("review not run") && error.message.contains(subject),
        "plain refusal message naming {subject}: {error:?}"
    );
    assert!(error.suggested_action.is_some(), "{error:?}");
    assert!(!seen.exists(), "the reviewer must not start");
    let judge = receipt_judge(rpath);
    assert_eq!(judge["status"], "error");
    assert!(judge["review"].is_null());
    assert!(
        judge["not_run_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains(subject)),
        "{judge}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn linked_review_file_leaves_the_outside_file_unchanged() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (project, _) = project_paths(&rpath);
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    std::os::unix::fs::symlink(&outside, review_leaf(&project)).unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    assert_refused(&record, &rpath, &seen, "saved review file");
}

#[tokio::test]
async fn hardlinked_review_file_leaves_the_outside_file_unchanged() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (project, _) = project_paths(&rpath);
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    std::fs::hard_link(&outside, review_leaf(&project)).unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    assert_refused(&record, &rpath, &seen, "saved review file");
}

#[cfg(unix)]
#[tokio::test]
async fn linked_render_report_never_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (_, receipts) = project_paths(&rpath);
    let other = dir.path().join("other-project-perception.json");
    std::fs::write(&other, report("sha256:other-video", CANARY)).unwrap();
    std::os::unix::fs::symlink(&other, receipts.join("render_001.output.perception.json")).unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_refused(&record, &rpath, &seen, "analysis file");
    assert!(!serde_json::to_string(&receipt_judge(&rpath))
        .unwrap()
        .contains(CANARY));
}

#[tokio::test]
async fn hardlinked_render_report_never_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (_, receipts) = project_paths(&rpath);
    // Even a report that claims this render's hash is refused when the file
    // is shared with another location.
    let other = dir.path().join("other-project-perception.json");
    std::fs::write(&other, report(FIXTURE_HASH, CANARY)).unwrap();
    std::fs::hard_link(&other, receipts.join("render_001.output.perception.json")).unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_refused(&record, &rpath, &seen, "analysis file");
}

#[tokio::test]
async fn render_report_for_another_video_never_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (_, receipts) = project_paths(&rpath);
    let report_path = receipts.join("render_001.output.perception.json");
    std::fs::write(&report_path, report("sha256:different-video", CANARY)).unwrap();
    // Made by this engine, but for another video (for example a stale file).
    cut_media::cache_trust::remember(&report_path);
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_refused(&record, &rpath, &seen, "different video");
}

/// A received project can write any `asset_hash` into the render's analysis
/// file; one this engine did not make is never given to the reviewer, and the
/// result says so.
#[tokio::test]
async fn render_report_not_made_on_this_computer_never_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (_, receipts) = project_paths(&rpath);
    std::fs::write(
        receipts.join("render_001.output.perception.json"),
        report(FIXTURE_HASH, CANARY),
    )
    .unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    let review = the_one_review(&seen);
    assert!(review["perception"].is_null(), "{review}");
    assert!(!review["text"].as_str().unwrap_or("").contains(CANARY));
    let result = record.result.expect("job result");
    assert!(
        result["warnings"][0]
            .as_str()
            .is_some_and(|note| note.contains("not made by this engine on this computer")),
        "{result}"
    );
}

#[tokio::test]
async fn matching_render_report_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (project, receipts) = project_paths(&rpath);
    let report_path = receipts.join("render_001.output.perception.json");
    std::fs::write(&report_path, report(FIXTURE_HASH, ORDINARY_WORD)).unwrap();
    // As render.final does after analysing the render.
    cut_media::cache_trust::remember(&report_path);
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    let seen = the_one_review(&seen);
    assert!(seen["text"].as_str().unwrap().contains(ORDINARY_WORD));
    // The reviewer gets the run's own copy of exactly the checked bytes.
    assert_eq!(
        std::fs::read(seen["perception"].as_str().unwrap()).unwrap(),
        std::fs::read(&report_path).unwrap()
    );
    assert_eq!(receipt_judge(&rpath)["review"]["verdict"], "pass");
    // The saved copy stays where it always was.
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(project.join(".scratch/judge/render_001/envelope.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["status"], "completed");
}

#[cfg(unix)]
#[tokio::test]
async fn linked_render_output_outside_the_project_is_refused() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (project, _) = project_paths(&rpath);
    let private = dir.path().join("private-video.mp4");
    std::fs::write(&private, b"private-video-bytes").unwrap();
    let render = project.join("exports/render_001.mp4");
    std::fs::remove_file(&render).unwrap();
    std::os::unix::fs::symlink(&private, &render).unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    std::env::set_var("CUTD_JUDGE_ADAPTER", &adapter);
    let response = dispatch(&state, "verify.judge", json!({}), test_actor()).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");

    let error = response.error.expect("a linked render is refused");
    assert!(error.message.contains("render"), "{error:?}");
    assert!(!seen.exists(), "the reviewer must not start");
}

/// Point the fixture receipt at a file outside the project and its export
/// folders, recording `hash` as the render's content hash.
fn receipt_names_outside_file(rpath: &Path, file: &Path, hash: &str) {
    let mut receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(rpath).unwrap()).unwrap();
    receipt.output_path = file.display().to_string();
    receipt.output_hash = hash.into();
    std::fs::write(rpath, serde_json::to_string_pretty(&receipt).unwrap()).unwrap();
}

#[tokio::test]
async fn unrelated_file_outside_the_project_is_never_reviewed() {
    // Same lock order as the other output tests: output folder first.
    let _output_dir = crate::output_paths::SESSION_OUTPUT_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let private = dir.path().join("private-video.mp4");
    std::fs::write(&private, b"private-video-bytes").unwrap();
    // A project cannot know the content hash of someone else's file.
    receipt_names_outside_file(&rpath, &private, "sha256:guessed");
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_refused(&record, &rpath, &seen, "render file");
}

#[tokio::test]
async fn render_saved_elsewhere_is_reviewed_when_it_matches_its_receipt() {
    use sha2::{Digest, Sha256};
    // Same lock order as the other output tests: output folder first.
    let _output_dir = crate::output_paths::SESSION_OUTPUT_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    // A one-off Save As destination is not a standing export folder.
    let saved = dir.path().join("save-as/render.mp4");
    std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
    std::fs::write(&saved, b"saved-as-render-bytes").unwrap();
    let hash = format!("sha256:{:x}", Sha256::digest(b"saved-as-render-bytes"));
    receipt_names_outside_file(&rpath, &saved, &hash);
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    let seen = the_one_review(&seen);
    // Reviewed as the run's copy that matched the receipt, removed afterwards.
    let given = seen["render"].as_str().unwrap();
    assert_ne!(given, saved.display().to_string());
    assert!(
        given.contains(".scratch") && given.ends_with("render.mp4"),
        "{given}"
    );
}

/// A pipe planted among the receipts must not block the review (and with it
/// the project) while the edit history for the review request is gathered.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planted_receipt_pipe_does_not_stall_the_review() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (_, receipts) = project_paths(&rpath);
    let pipe = receipts.join("render_000.json");
    let name = std::ffi::CString::new(pipe.display().to_string()).unwrap();
    // SAFETY: name is a valid NUL-terminated path for the duration of the call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (adapter, _seen) = recording_adapter(dir.path());

    std::env::set_var("CUTD_JUDGE_ADAPTER", &adapter);
    let task_state = state.clone();
    let request = tokio::spawn(async move {
        dispatch(&task_state, "verify.judge", json!({}), test_actor()).await
    });
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), request).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    if outcome.is_err() {
        // Release a reader blocked on the pipe so the runtime can shut down.
        let _ = std::fs::OpenOptions::new().write(true).open(&pipe);
        panic!("verify.judge stalled on a pipe planted among the receipts");
    }
    let response = outcome.unwrap().unwrap();
    assert!(response.ok, "{:?}", response.error);
    let job = response.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(wait_job(&state, &job, 30).await.state, JobState::Done);
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
async fn windows_symlink_review_file_leaves_the_outside_file_unchanged() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (project, _) = project_paths(&rpath);
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    std::os::windows::fs::symlink_file(&outside, review_leaf(&project)).unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    assert_refused(&record, &rpath, &seen, "saved review file");
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
async fn windows_symlink_render_report_never_reaches_the_reviewer() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let _python = TestAdapterPythonEnv::install();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let (_, receipts) = project_paths(&rpath);
    let other = dir.path().join("other-project-perception.json");
    std::fs::write(&other, report(FIXTURE_HASH, CANARY)).unwrap();
    std::os::windows::fs::symlink_file(&other, receipts.join("render_001.output.perception.json"))
        .unwrap();
    let (adapter, seen) = recording_adapter(dir.path());

    let record = run_review(&state, &adapter).await;

    assert_refused(&record, &rpath, &seen, "analysis file");
}
