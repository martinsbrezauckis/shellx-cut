use std::path::{Path, PathBuf};
use std::time::Duration;

use super::await_bounded_export_work;
use crate::dispatch::run_blocking_cancellable;
use crate::output_paths::{fence_output_path, OutputPath, OutputPathPolicy};
use crate::state::AppState;
use cut_core::{error_codes, CutError};

async fn wait_terminal(state: &AppState, job_id: &str) -> crate::jobs::JobRecord {
    for _ in 0..200 {
        let record = state.jobs.get(job_id).expect("export job record");
        if matches!(
            record.state,
            crate::jobs::JobState::Done | crate::jobs::JobState::Failed
        ) {
            return record;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("export job {job_id} did not become terminal")
}

async fn reclaim(project: &Path, path: &Path) {
    for _ in 0..100 {
        if let Ok(lease) = fence_output_path(
            project,
            Some(path.to_str().expect("utf-8 temp path")),
            "exports/unused.mp4",
            OutputPathPolicy::MP4,
        ) {
            drop(lease);
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("output lease was not released for {}", path.display())
}

fn spawn_lease_job(
    state: &AppState,
    out: OutputPath,
    timeout: Duration,
    work: impl FnOnce(crate::jobs::JobCancellation, OutputPath) -> Result<(), CutError> + Send + 'static,
) -> String {
    let job = state.jobs.create("screen_record_export");
    let job_id = job.job_id.clone();
    let job_id_for_task = job_id.clone();
    let jobs = state.jobs.clone();
    state
        .jobs
        .spawn_limited(&job_id, "screen_record.export.test", 1, async move {
            jobs.progress(
                &job_id_for_task,
                0.1,
                Some("Rendering MP4 recording…".into()),
            );
            let bounded = await_bounded_export_work(
                timeout,
                &jobs,
                &job_id_for_task,
                run_blocking_cancellable("screen_record.export", move |cancel| work(cancel, out)),
            )
            .await;
            match bounded {
                Ok(()) => jobs.finish(&job_id_for_task, serde_json::json!({"path":"test.mp4"})),
                Err(error) => jobs.fail(&job_id_for_task, error),
            }
        });
    job_id
}

fn test_output(project: &Path, name: &str) -> (OutputPath, PathBuf) {
    let path = project.join(name);
    let output = fence_output_path(
        project,
        Some(path.to_str().expect("utf-8 temp path")),
        "exports/unused.mp4",
        OutputPathPolicy::MP4,
    )
    .expect("reserve test export output");
    (output, path)
}

#[tokio::test]
async fn export_output_lease_releases_after_success_and_failure() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("lease.cutproj");
    std::fs::create_dir_all(&project).unwrap();
    let state = AppState::new();

    let (out, path) = test_output(&project, "success.mp4");
    let success = spawn_lease_job(&state, out, Duration::from_secs(1), move |_cancel, out| {
        std::fs::write(&out, b"complete")?;
        Ok(())
    });
    assert!(matches!(
        wait_terminal(&state, &success).await.state,
        crate::jobs::JobState::Done
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"complete");
    reclaim(&project, &path).await;

    let (out, path) = test_output(&project, "failure.mp4");
    let failure = spawn_lease_job(&state, out, Duration::from_secs(1), |_cancel, _out| {
        Err(CutError::new(
            error_codes::FFMPEG,
            "test render failed",
            "fixture failure",
        ))
    });
    let record = wait_terminal(&state, &failure).await;
    assert_eq!(
        record.error.as_ref().map(|error| error.code.as_str()),
        Some(error_codes::FFMPEG)
    );
    reclaim(&project, &path).await;
}

#[tokio::test]
async fn export_output_lease_releases_after_cancel_and_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("lease.cutproj");
    std::fs::create_dir_all(&project).unwrap();
    let state = AppState::new();

    let (out, path) = test_output(&project, "cancel.mp4");
    let cancel = spawn_lease_job(&state, out, Duration::from_secs(1), |cancel, _out| loop {
        if cancel.is_cancelled() {
            return Err(CutError::new(
                error_codes::RENDER_CANCELLED,
                "test render cancelled",
                "fixture observed cancellation",
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    });
    for _ in 0..100 {
        if matches!(
            state.jobs.get(&cancel).map(|record| record.state),
            Some(crate::jobs::JobState::Running)
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        state.jobs.abort(&cancel).await.unwrap(),
        "cancel should drain worker"
    );
    reclaim(&project, &path).await;

    let (out, path) = test_output(&project, "timeout.mp4");
    let timeout = spawn_lease_job(
        &state,
        out,
        Duration::from_millis(20),
        |cancel, _out| loop {
            if cancel.is_cancelled() {
                return Err(CutError::new(
                    error_codes::RENDER_CANCELLED,
                    "test render cancelled",
                    "timeout requested cancellation",
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        },
    );
    let record = wait_terminal(&state, &timeout).await;
    assert_eq!(
        record.error.as_ref().map(|error| error.code.as_str()),
        Some(error_codes::FFMPEG)
    );
    reclaim(&project, &path).await;
}

/// The export job's wall-clock deadline must not merely abandon a blocking
/// ffmpeg wait. This uses a real sleeping child so the test covers the route
/// that signals the process tree, waits for its leader, then releases the
/// fenced output only after the worker returns.
#[cfg(unix)]
#[tokio::test]
async fn export_timeout_terminates_and_reaps_a_hanging_ffmpeg_child() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("lease.cutproj");
    std::fs::create_dir_all(&project).unwrap();
    let state = AppState::new();
    let (out, path) = test_output(&project, "hanging-child.mp4");

    let started = std::time::Instant::now();
    let timeout = spawn_lease_job(&state, out, Duration::from_millis(80), |cancel, _out| {
        let control =
            record_render::ffmpeg::ProcessControl::bounded(Duration::from_secs(5), move || {
                cancel.is_cancelled()
            });
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 60"]);
        record_render::ffmpeg::command_output_with_control(
            &mut command,
            &control,
            "screen-record export hanging-child fixture",
        )
        .map(|_| ())
        .map_err(crate::screen_record::record_err)
    });

    let record = wait_terminal(&state, &timeout).await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "deadline cancellation should reap the child promptly"
    );
    assert_eq!(
        record.error.as_ref().map(|error| error.code.as_str()),
        Some(error_codes::FFMPEG)
    );
    reclaim(&project, &path).await;
}

/// The installed matrix hits this path after its five-minute evidence deadline:
/// `jobs.cancel` may report completion only after the export's owned ffmpeg
/// child is gone. Keep the real-child proof separate from the wall-clock timeout
/// fixture above so an async-task abort cannot regress into a detached worker.
#[cfg(unix)]
#[tokio::test]
async fn export_user_cancel_terminates_and_reaps_a_hanging_ffmpeg_child() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("lease.cutproj");
    std::fs::create_dir_all(&project).unwrap();
    let state = AppState::new();
    let (out, path) = test_output(&project, "cancel-hanging-child.mp4");

    let job_id = spawn_lease_job(&state, out, Duration::from_secs(30), |cancel, _out| {
        let control =
            record_render::ffmpeg::ProcessControl::bounded(Duration::from_secs(30), move || {
                cancel.is_cancelled()
            });
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 60"]);
        record_render::ffmpeg::command_output_with_control(
            &mut command,
            &control,
            "screen-record export user-cancel hanging-child fixture",
        )
        .map(|_| ())
        .map_err(crate::screen_record::record_err)
    });

    for _ in 0..100 {
        if matches!(
            state.jobs.get(&job_id).map(|record| record.state),
            Some(crate::jobs::JobState::Running)
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        matches!(
            state.jobs.get(&job_id).map(|record| record.state),
            Some(crate::jobs::JobState::Running)
        ),
        "fixture export did not start its hanging child"
    );
    let started = std::time::Instant::now();
    assert!(state.jobs.abort(&job_id).await.unwrap());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "jobs.cancel should wait only until its child is reaped"
    );
    let record = state.jobs.get(&job_id).expect("cancelled export job");
    assert_eq!(record.state, crate::jobs::JobState::Failed);
    assert_eq!(
        record.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancelled")
    );
    reclaim(&project, &path).await;
}
