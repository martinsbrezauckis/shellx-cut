//! Copy a saved raw recording through the existing export fence and job owner.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::dispatch::{parse_args, run_blocking_cancellable, snapshot};
use crate::output_paths::{
    authorized_export_read_roots, fence_output_path, publish_output_atomic, OutputPath,
    OutputPathPolicy,
};
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};

const COPY_LIMIT_KEY: &str = "screen_record.copy_raw";

/// `source` is the successful Stop result's `raw_path`. Read authority is the
/// active project's existing export-read roots, never an arbitrary local file.
pub(crate) async fn screen_record_copy_raw(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        source: String,
        path: Option<String>,
        expected_origin_path_sha256: Option<String>,
    }
    let args: Args = parse_args(args)?;
    let _transition = state.project_transition.lock().await;
    crate::project_origin::admit(state, args.expected_origin_path_sha256.as_deref()).await?;
    let (_project, _edl, dir, _revision) = snapshot(state).await?;
    let source = resolve_saved_raw_source(&dir, &args.source)?;
    let input = File::open(&source).map_err(|error| io_error("open raw recording", error))?;
    if !input
        .metadata()
        .map_err(|error| io_error("inspect raw recording", error))?
        .is_file()
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "raw recording is not a regular file",
            "the saved raw source must be a local regular MP4",
        ));
    }
    let output = fence_output_path(
        &dir,
        args.path.as_deref(),
        "exports/raw_recording-copy.mp4",
        OutputPathPolicy::MP4,
    )?;
    if output.as_ref() == source {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "copy destination is the raw recording itself",
            "choose a different Save As path",
        ));
    }
    let path = output.display().to_string();
    let job = state.jobs.create("screen_record_copy_raw");
    let job_id = job.job_id.clone();
    let jobs = state.jobs.clone();
    let task_id = job_id.clone();
    let task_path = path.clone();
    state
        .jobs
        .spawn_limited(&job_id, COPY_LIMIT_KEY, 1, async move {
            let result = run_blocking_cancellable("screen_record.copy_raw", move |cancel| {
                copy_to_output(input, output, cancel)
            })
            .await;
            match result {
                Ok(bytes) => jobs.finish(&task_id, json!({"path": task_path, "bytes": bytes})),
                Err(error) => jobs.fail(&task_id, error),
            }
        });
    Ok(VerbResult::ok(
        json!({"job_id": job_id, "path": path, "status": "queued"}),
    ))
}

fn resolve_saved_raw_source(project_dir: &Path, requested: &str) -> Result<PathBuf, CutError> {
    let path = Path::new(requested);
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "raw source must be the absolute MP4 path returned by Stop",
            "pass screen_record.stop.raw_path",
        ));
    }
    for root in authorized_export_read_roots(project_dir) {
        if let Ok(source) = crate::screen_record::plain_existing_file_under_project(
            &root,
            requested,
            "saved raw recording",
            "pass screen_record.stop.raw_path from this project's authorized export folder",
        ) {
            return Ok(source);
        }
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "raw source is outside this project's authorized export folders or is not a plain file",
        "only a saved raw MP4 in the active project's export folder may be copied",
    ))
}

fn copy_to_output(
    mut input: File,
    output: OutputPath,
    cancel: crate::jobs::JobCancellation,
) -> Result<u64, CutError> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut stage = tempfile::Builder::new()
        .prefix(".raw-recording-copy-")
        .tempfile_in(parent)
        .map_err(|error| io_error("create private copy stage", error))?;
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let count = input
            .read(&mut buffer)
            .map_err(|error| io_error("read raw recording", error))?;
        if count == 0 {
            break;
        }
        stage
            .write_all(&buffer[..count])
            .map_err(|error| io_error("write private copy stage", error))?;
        bytes += count as u64;
    }
    stage
        .as_file()
        .sync_all()
        .map_err(|error| io_error("flush private copy stage", error))?;
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    // Close the stage handle before rename so Windows can publish it too.
    let stage = stage.into_temp_path();
    publish_output_atomic(stage.as_ref(), &output)?;
    Ok(bytes)
}

fn io_error(action: &str, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not {action}"),
        error.to_string(),
    )
}

fn cancelled() -> CutError {
    CutError::new(
        "job_cancelled",
        "raw recording copy was cancelled",
        "the destination was not published",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::dispatch;
    use crate::jobs::JobState;
    use cut_core::Actor;
    use std::time::Duration;

    async fn wait_copy(state: &AppState, job_id: &str) -> Value {
        for _ in 0..200 {
            let record = state.jobs.get(job_id).expect("raw copy job");
            match record.state {
                JobState::Done => return record.result.expect("copy result"),
                JobState::Failed => panic!("raw copy failed: {:?}", record.error),
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        panic!("raw copy job did not finish")
    }

    #[test]
    fn copy_raw_is_fenced_collision_safe_and_byte_exact() {
        let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let project = temp.path().join("raw_copy.cutproj");
            let state = AppState::new();
            let created = dispatch(
                &state,
                "project.create",
                json!({"name":"raw_copy","dir":project}),
                Actor::system(),
            )
            .await;
            assert!(created.ok, "{:?}", created.error);
            let identity = dispatch(&state, "project.state", json!({}), Actor::system()).await;
            let origin = identity.result.unwrap()["project_identity"]["origin_path_sha256"]
                .as_str()
                .unwrap()
                .to_string();
            let source = project.join("exports/raw_recording.mp4");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, b"raw recording bytes").unwrap();

            for (index, expected) in ["raw_recording-copy.mp4", "raw_recording-copy-2.mp4"]
                .into_iter()
                .enumerate()
            {
                if index == 1 {
                    let renamed = dispatch(
                        &state,
                        "project.rename",
                        json!({"name":"renamed raw copy"}),
                        Actor::system(),
                    )
                    .await;
                    assert!(renamed.ok, "{:?}", renamed.error);
                }
                let queued = dispatch(
                    &state,
                    "screen_record.copy_raw",
                    json!({"source": source,"expected_origin_path_sha256":origin}),
                    Actor::system(),
                )
                .await;
                assert!(queued.ok, "{:?}", queued.error);
                let response = queued.result.unwrap();
                let path = PathBuf::from(response["path"].as_str().unwrap());
                assert_eq!(path.file_name().unwrap(), expected);
                let done = wait_copy(&state, response["job_id"].as_str().unwrap()).await;
                assert_eq!(done["bytes"], 19);
                assert_eq!(std::fs::read(path).unwrap(), b"raw recording bytes");
            }
            assert_eq!(std::fs::read(&source).unwrap(), b"raw recording bytes");

            let outside = temp.path().join("unrelated.mp4");
            std::fs::write(&outside, b"unrelated").unwrap();
            let denied = dispatch(
                &state,
                "screen_record.copy_raw",
                json!({"source": outside}),
                Actor::system(),
            )
            .await;
            assert!(
                !denied.ok,
                "a path outside this project's exports must fail"
            );
            #[cfg(unix)]
            {
                let linked = project.join("exports/linked_raw.mp4");
                std::os::unix::fs::symlink(&outside, &linked).unwrap();
                let denied = dispatch(
                    &state,
                    "screen_record.copy_raw",
                    json!({"source": linked}),
                    Actor::system(),
                )
                .await;
                assert!(!denied.ok, "a linked source must fail even under exports");
            }
            let same = dispatch(
                &state,
                "screen_record.copy_raw",
                json!({"source": source, "path": source}),
                Actor::system(),
            )
            .await;
            assert!(!same.ok, "the source must not be its own destination");
        });
    }

    #[test]
    fn cancelled_copy_removes_private_stage_and_never_publishes() {
        let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("cancel.cutproj");
        std::fs::create_dir_all(&project).unwrap();
        let source = project.join("source.mp4");
        std::fs::write(&source, b"capture").unwrap();
        let output = fence_output_path(
            &project,
            None,
            "exports/raw_recording-copy.mp4",
            OutputPathPolicy::MP4,
        )
        .unwrap();
        let path = output.to_path_buf();
        let cancel = crate::jobs::JobCancellation::test_active();
        cancel.request_cancel();
        let error = copy_to_output(File::open(source).unwrap(), output, cancel).unwrap_err();
        assert_eq!(error.code, "job_cancelled");
        assert!(!path.exists());
        assert!(std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".raw-recording-copy-")));
    }
}
