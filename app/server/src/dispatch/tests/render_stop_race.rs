//! A stop that meets an export's last step: the record says what happened.
//!
//! Field finding (clean Windows VM): "Quit and stop it" was pressed as the
//! export finished. The complete file stayed in exports with no receipt and
//! the job read "cancelled". Now:
//! - a stop that lands before the file is complete leaves no complete-looking
//!   file (no output, no staging, no receipt) and the job reads cancelled;
//! - a stop that lands after the file is published does not undo it: the
//!   receipt is written, the job reads done, and nothing is called interrupted.

use super::{dispatch_retry_pending, test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// One test at a time changes the shared pause.
static PAUSE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn make_clip(dir: &Path, seconds: u32) -> PathBuf {
    let out = dir.join("clip.mp4");
    let made = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=640x360:rate=25:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency=440:sample_rate=48000:duration={seconds}"
        ))
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ])
        .args(["-c:a", "aac", "-b:a", "96k"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(made.success());
    out
}

async fn project(seconds: u32) -> (tempfile::TempDir, AppState, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    // A stop waits for the encoder and the receipt like the app's quit does.
    state
        .jobs
        .set_drain_timeout_for_test(std::time::Duration::from_secs(30));
    let project = root.path().join("race.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "race", "dir": project}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = make_clip(root.path(), seconds);
    let imported = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    let job = wait_job(
        &state,
        imported.result.unwrap()["job_id"].as_str().unwrap(),
        60,
    )
    .await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    (root, state, project)
}

fn listing(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn start_render(state: &AppState) -> String {
    let rendered = dispatch(
        state,
        "render.final",
        json!({"hardware": "off", "preset": "high"}),
        test_actor(),
    )
    .await;
    assert!(rendered.ok, "{:?}", rendered.error);
    rendered.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_stop_before_the_file_is_complete_leaves_nothing_that_looks_complete() {
    let _pause = PAUSE.lock().await;
    let (_root, state, project) = project(40).await;
    let job_id = start_render(&state).await;
    // Wait until the encoder is writing, then stop.
    for _ in 0..200 {
        if state
            .jobs
            .get(&job_id)
            .is_some_and(|job| job.progress > 0.05)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let cancelled = dispatch_retry_pending(
        &state,
        "jobs.cancel",
        json!({"job_id": job_id}),
        test_actor(),
    )
    .await;
    assert!(cancelled.ok, "{:?}", cancelled.error);
    let job = wait_job(&state, &job_id, 60).await;
    assert_eq!(job.state, JobState::Failed);
    let record = serde_json::to_value(&job).unwrap();
    assert_eq!(record["outcome"], "cancelled", "{record}");
    let exports = listing(&project.join("exports"));
    assert!(
        exports.is_empty(),
        "no complete-looking or staging file: {exports:?}"
    );
    let receipts: Vec<String> = listing(&project.join("receipts"))
        .into_iter()
        .filter(|name| name.starts_with("render_"))
        .collect();
    assert!(receipts.is_empty(), "no receipt: {receipts:?}");
}

#[tokio::test]
async fn a_stop_after_the_file_is_published_does_not_disown_it() {
    let _pause = PAUSE.lock().await;
    let (_root, state, project) = project(3).await;
    super::super::rendering::PAUSE_AFTER_PUBLISH_MS.store(4_000, Ordering::Relaxed);
    let job_id = start_render(&state).await;
    let output = project.join("exports").join("render_001.mp4");
    let mut published = false;
    for _ in 0..1_200 {
        if output.is_file() {
            published = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(published, "the export file was published");
    // The stop lands while the job holds after publishing (the quit's stop).
    let stopped = dispatch(
        &state,
        "jobs.before_quit",
        json!({"cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    super::super::rendering::PAUSE_AFTER_PUBLISH_MS.store(0, Ordering::Relaxed);
    assert!(stopped.ok, "{:?}", stopped.error);
    let job = wait_job(&state, &job_id, 120).await;
    assert_eq!(job.state, JobState::Done, "the export completed: {job:?}");
    let listed = stopped.result.unwrap()["cancelled_jobs"].clone();
    assert!(
        !listed.to_string().contains(&job_id),
        "a completed export is not reported cancelled: {listed}"
    );
    assert!(output.is_file(), "the file is kept");
    assert!(
        project.join("receipts").join("render_001.json").is_file(),
        "the receipt is written"
    );
    let exports = listing(&project.join("exports"));
    assert_eq!(
        exports,
        vec!["render_001.mp4".to_string()],
        "no staging file left"
    );
}
