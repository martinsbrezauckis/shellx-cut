//! Render review sheets and render checks use project files only as ordinary
//! files: a link or shared file planted in a received project's receipts
//! folder is never written through, and a planted pipe never stalls a verb or
//! a render job. The analysis tool is a local stand-in (no model runs).

use super::nest_media_io::create_nested_project;
use super::{lock_agent_cli_env, test_actor};
use crate::dispatch::dispatch;
use crate::jobs::{JobRecord, JobState};
use crate::state::AppState;
use serde_json::json;
use std::path::{Path, PathBuf};

const SENTINEL: &[u8] = b"outside-project-sentinel\n";

/// The stand-in writes review sheets with a plain write, as the image writer
/// does, and answers analysis requests with an empty report.
const STAND_IN: &str = r#"import json, os, sys
request = json.load(sys.stdin)
# Like the real tool, fail for missing media (other tests rely on that).
if not os.path.exists(str(request["media_path"])):
    sys.exit("media not found")
for key, name in (("contact_sheet", "contact_sheet.jpg"), ("qc_sheet", "qc_sheet.jpg")):
    if request.get(key) is not None:
        path = os.path.join(request[key], name)
        with open(path, "wb") as sheet:
            sheet.write(b"stand-in sheet")
        print(json.dumps({key: path, "scenes": []}))
        sys.exit(0)
print(json.dumps({"schema": "shellx-cut/perception/1", "asset_hash": request["asset_hash"],
                  "source_path": str(request["media_path"]),
                  "instruments_run": request["instruments"]}))
"#;

struct StandInTool {
    _dir: tempfile::TempDir,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl StandInTool {
    fn install() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("instruments.py"), STAND_IN).unwrap();
        let python = super::super::adapter_python::find_python_on_path().expect("python");
        let keys = [
            cut_perception::sidecar::ENV_SIDECAR_DIR,
            cut_perception::sidecar::ENV_PYTHON,
            cut_native_runtime_context::CONTEXT_ENV,
        ];
        let previous = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
        std::env::set_var(cut_perception::sidecar::ENV_SIDECAR_DIR, dir.path());
        std::env::set_var(cut_perception::sidecar::ENV_PYTHON, python);
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        Self {
            _dir: dir,
            previous,
        }
    }
}

impl Drop for StandInTool {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Wait for a job; on timeout, release a reader blocked on `pipe` and fail.
async fn finish(state: &AppState, job: &str, pipe: Option<&Path>) -> JobRecord {
    for _ in 0..1200 {
        if let Some(record) = state.jobs.get(job) {
            if matches!(record.state, JobState::Done | JobState::Failed) {
                return record;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if let Some(pipe) = pipe {
        let _ = std::fs::OpenOptions::new().write(true).open(pipe);
    }
    panic!("job {job} did not finish");
}

fn mkfifo(path: &Path) {
    let name = std::ffi::CString::new(path.display().to_string()).unwrap();
    // SAFETY: name is a valid NUL-terminated path for the duration of the call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
}

fn plant(kind: &str, outside: &Path, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    match kind {
        "link" => std::os::unix::fs::symlink(outside, at).unwrap(),
        _ => std::fs::hard_link(outside, at).unwrap(),
    }
}

async fn job_of(response: cut_core::VerbResult) -> String {
    assert!(response.ok, "{:?}", response.error);
    response.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[cfg(unix)]
#[test]
fn planted_contact_sheet_is_never_written_through() {
    // Same lock order as the other output tests: output folder first.
    let _output = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for kind in ["link", "hard link"] {
            let _tool = StandInTool::install();
            let root = tempfile::tempdir().unwrap();
            let (state, project) = create_nested_project(root.path()).await;
            let outside = root.path().join("outside.jpg");
            std::fs::write(&outside, SENTINEL).unwrap();
            // The next direct id counts the receipts entries, including the
            // planted folder itself, so it is predictable.
            plant(
                kind,
                &outside,
                &project.join("receipts/direct_002.contact/contact_sheet.jpg"),
            );

            let job =
                job_of(dispatch(&state, "render.direct", json!({}), test_actor()).await).await;
            let record = finish(&state, &job, None).await;

            assert_eq!(
                std::fs::read(&outside).unwrap(),
                SENTINEL,
                "{kind}: {record:?}"
            );
        }
    });
}

#[cfg(unix)]
#[test]
fn planted_review_sheet_is_never_written_through() {
    // Same lock order as the other output tests: output folder first.
    let _output = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        for kind in ["link", "hard link"] {
            let _tool = StandInTool::install();
            let root = tempfile::tempdir().unwrap();
            let (state, project) = create_nested_project(root.path()).await;
            std::fs::create_dir_all(project.join("exports")).unwrap();
            std::fs::copy(
                root.path().join("source.mp4"),
                project.join("exports/reframe_001.mp4"),
            )
            .unwrap();
            let outside = root.path().join("outside.jpg");
            std::fs::write(&outside, SENTINEL).unwrap();
            plant(
                kind,
                &outside,
                &project.join("receipts/reframe_001.qc/qc_sheet.jpg"),
            );

            let response = dispatch(
                &state,
                "render.qc",
                json!({"reframe_id": "reframe_001"}),
                test_actor(),
            )
            .await;
            let record = finish(&state, &job_of(response).await, None).await;

            assert_eq!(
                std::fs::read(&outside).unwrap(),
                SENTINEL,
                "{kind}: {record:?}"
            );
        }
    });
}

#[cfg(unix)]
#[test]
fn planted_reframe_receipt_pipe_does_not_stall_review() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let receipts = root.path().join("receipts");
        std::fs::create_dir_all(&receipts).unwrap();
        let pipe = receipts.join("reframe_001.json");
        mkfifo(&pipe);
        let dir = root.path().to_path_buf();
        let lookup = tokio::task::spawn_blocking(move || {
            super::super::rendering::resolve_reframe_output_for_qc(&dir, "reframe_001")
        });
        if tokio::time::timeout(std::time::Duration::from_secs(10), lookup)
            .await
            .is_err()
        {
            let _ = std::fs::OpenOptions::new().write(true).open(&pipe);
            panic!("render.qc stalled on a pipe planted among the receipts");
        }
    });
}

#[cfg(unix)]
#[test]
fn planted_transcript_pipe_does_not_stall_the_render_checks() {
    // Same lock order as the other output tests: output folder first.
    let _output = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let _env = lock_agent_cli_env();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let _tool = StandInTool::install();
        let root = tempfile::tempdir().unwrap();
        let (state, project) = create_nested_project(root.path()).await;
        crate::dispatch::update_asset(&state, "a1", |asset| {
            asset.transcript = Some("receipts/a1.words.json".into())
        })
        .await
        .unwrap();
        let pipe: PathBuf = project.join("receipts/a1.words.json");
        mkfifo(&pipe);

        let job = job_of(dispatch(&state, "render.final", json!({}), test_actor()).await).await;
        let record = finish(&state, &job, Some(&pipe)).await;

        assert_eq!(record.state, JobState::Done, "{:?}", record.error);
    });
}
