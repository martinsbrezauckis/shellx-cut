//! Speaker labelling writes its receipt, analysis report and transcript only
//! as ordinary files of the project: a link or shared file placed at those
//! paths by a received project is replaced as a name, never written through.

#[cfg(unix)]
use super::{lock_agent_cli_env, narrowing_fixture, test_actor, wait_job};
use crate::diarize::{merge_diarization, parse_receipt};
#[cfg(unix)]
use crate::dispatch::dispatch;
#[cfg(unix)]
use crate::jobs::JobState;
use serde_json::json;
use std::path::{Path, PathBuf};

const SENTINEL: &[u8] = b"outside-project-sentinel\n";
const HASH: &str = "sha256:diarize-fixture";
const RECEIPT_LINE: &str = r#"{"schema":"shellx-cut/diarize/1","turns":[{"start_ms":0,"end_ms":1000,"speaker":"S1"}],"n_speakers":1,"model":"stand-in","backend":"stand-in"}"#;

fn sentinel(dir: &Path) -> PathBuf {
    let path = dir.join("outside-sentinel.txt");
    std::fs::write(&path, SENTINEL).unwrap();
    path
}

/// A receipts folder holding a transcript-bearing report for `a1`.
fn receipts_with_report(dir: &Path) -> PathBuf {
    let receipts = dir.join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    super::engine_reports::write_engine_report(
        receipts.join("a1.perception.json"),
        json!({
            "schema": cut_perception::PERCEPTION_SCHEMA,
            "asset_hash": HASH,
            "source_path": "/media/clip.mp4",
            "instruments_run": ["words"],
            "words": {"asset": "a1", "model": "fixture", "words": [
                {"idx": 0, "word": "hello", "start_ms": 100, "end_ms": 400}]},
        })
        .to_string(),
    )
    .unwrap();
    receipts
}

fn merge(receipts: &Path) -> Result<usize, cut_core::CutError> {
    merge_diarization(
        receipts,
        "a1",
        HASH,
        Path::new("/media/clip.mp4"),
        parse_receipt(RECEIPT_LINE).unwrap(),
    )
}

fn assert_plain_with_speaker(path: &Path) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.file_type().is_file() && !metadata.file_type().is_symlink());
    assert!(std::fs::read_to_string(path).unwrap().contains("\"S1\""));
}

#[cfg(unix)]
#[test]
fn linked_transcript_is_replaced_not_written_through() {
    let temp = tempfile::tempdir().unwrap();
    let receipts = receipts_with_report(temp.path());
    let outside = sentinel(temp.path());
    let words = receipts.join("a1.words.json");
    std::os::unix::fs::symlink(&outside, &words).unwrap();

    assert_eq!(merge(&receipts).unwrap(), 1);

    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    assert_plain_with_speaker(&words);
}

#[test]
fn shared_transcript_is_replaced_not_written_through() {
    let temp = tempfile::tempdir().unwrap();
    let receipts = receipts_with_report(temp.path());
    let outside = sentinel(temp.path());
    let words = receipts.join("a1.words.json");
    std::fs::hard_link(&outside, &words).unwrap();

    assert_eq!(merge(&receipts).unwrap(), 1);

    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    assert_plain_with_speaker(&words);
}

#[test]
fn shared_report_is_never_written_through() {
    let temp = tempfile::tempdir().unwrap();
    let receipts = receipts_with_report(temp.path());
    let report = receipts.join("a1.perception.json");
    let outside = temp.path().join("other-project-report.json");
    std::fs::rename(&report, &outside).unwrap();
    std::fs::hard_link(&outside, &report).unwrap();
    let original = std::fs::read(&outside).unwrap();

    let _ = merge(&receipts);

    assert_eq!(std::fs::read(&outside).unwrap(), original);
}

/// media.diarize with a stand-in runner (no service, no model): a link at
/// `receipts/<asset>.diarize.json` leaves its target unchanged.
#[cfg(unix)]
#[tokio::test]
async fn linked_diarize_receipt_is_replaced_not_written_through() {
    let _env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    let outside = sentinel(dir.path());
    let receipt = receipts.join("a1.diarize.json");
    std::os::unix::fs::symlink(&outside, &receipt).unwrap();
    let runner = dir.path().join("stand_in_diarize.py");
    std::fs::write(
        &runner,
        format!("import sys\nsys.stdin.read()\nprint('{RECEIPT_LINE}')\n"),
    )
    .unwrap();
    let python = super::super::adapter_python::find_python_on_path().expect("python on PATH");
    let previous = [
        ("DIARIZE_RUNNER_PY", std::env::var_os("DIARIZE_RUNNER_PY")),
        (
            "DIARIZE_RUNNER_SCRIPT",
            std::env::var_os("DIARIZE_RUNNER_SCRIPT"),
        ),
        (
            cut_native_runtime_context::CONTEXT_ENV,
            std::env::var_os(cut_native_runtime_context::CONTEXT_ENV),
        ),
    ];
    std::env::set_var("DIARIZE_RUNNER_PY", &python);
    std::env::set_var("DIARIZE_RUNNER_SCRIPT", &runner);
    std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);

    let response = dispatch(
        &state,
        "media.diarize",
        json!({"asset": "a1"}),
        test_actor(),
    )
    .await;
    let record = match &response.result {
        Some(result) => Some(wait_job(&state, result["job_id"].as_str().unwrap(), 30).await),
        None => None,
    };
    for (key, value) in previous {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    assert!(response.ok, "{:?}", response.error);
    let record = record.unwrap();
    assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    let metadata = std::fs::symlink_metadata(&receipt).unwrap();
    assert!(metadata.file_type().is_file() && !metadata.file_type().is_symlink());
}

#[cfg(windows)]
#[test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
fn windows_symlink_diarized_transcript_is_replaced_not_written_through() {
    let temp = tempfile::tempdir().unwrap();
    let receipts = receipts_with_report(temp.path());
    let outside = sentinel(temp.path());
    let words = receipts.join("a1.words.json");
    std::os::windows::fs::symlink_file(&outside, &words).unwrap();

    assert_eq!(merge(&receipts).unwrap(), 1);

    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
    assert_plain_with_speaker(&words);
}
