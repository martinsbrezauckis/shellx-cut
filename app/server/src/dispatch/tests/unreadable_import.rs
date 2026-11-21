//! A file that is not readable media never stays in the project.
//!
//! Text renamed `.mp4`/`.png`, an empty file, a folder: the verb or its import
//! job says plainly which file and why, with the stable `unreadable_media`
//! code, and no asset is left behind. Relink refuses the same files the same way.

use super::{test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::jobs::JobState;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

async fn open_project(state: &AppState, root: &Path) {
    let created = dispatch(
        state,
        "project.create",
        json!({"name": "unreadable", "dir": root.join("unreadable.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
}

async fn assets(state: &AppState) -> Value {
    let s = dispatch(state, "project.state", json!({}), test_actor()).await;
    assert!(s.ok, "{:?}", s.error);
    s.result.unwrap()["assets"].clone()
}

async fn op_verbs(state: &AppState) -> Vec<String> {
    let ops = dispatch(state, "project.ops", json!({}), test_actor()).await;
    ops.result.unwrap()["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["verb"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn text_bytes() -> Vec<u8> {
    b"hello this is not a video\n".repeat(200)
}

#[tokio::test]
async fn a_text_file_named_like_media_is_undone_with_a_plain_reason() {
    for name in ["textfile.mp4", "textfile.png"] {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        open_project(&state, root.path()).await;
        let media = root.path().join(name);
        std::fs::write(&media, text_bytes()).unwrap();
        let imported = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
        assert!(
            imported.ok,
            "the probe runs in the import job: {:?}",
            imported.error
        );
        let result = imported.result.unwrap();
        let job_id = result["job_id"].as_str().unwrap().to_string();
        let asset_id = result["asset_id"].as_str().unwrap().to_string();
        let job = wait_job(&state, &job_id, 30).await;
        assert_eq!(job.state, JobState::Failed, "{name}: {job:?}");
        let error = job.error.expect("a failed import names its error");
        assert_eq!(error.code, "unreadable_media", "{name}: {error:?}");
        assert!(
            error.message.starts_with(&format!("{name} cannot be used")),
            "{name}: the person reads which file: {}",
            error.message
        );
        assert!(
            error.message.contains("not a video, audio or image file"),
            "{}",
            error.message
        );
        assert!(
            error.cause.contains("the import was undone"),
            "{name}: the agent learns nothing stayed behind: {}",
            error.cause
        );
        assert!(
            error
                .suggested_action
                .as_deref()
                .is_some_and(|a| a.contains("media player")),
            "{name}: a next step a person can follow: {error:?}"
        );
        assert!(
            assets(&state).await.get(&asset_id).is_none(),
            "{name}: no dead asset stays in Assets"
        );
        assert_eq!(
            op_verbs(&state).await,
            ["project.create", "media.import", "media.remove"],
            "{name}: the undo is an ordinary, replay-safe op"
        );
    }
}

#[tokio::test]
async fn an_empty_file_and_a_folder_are_refused_before_anything_is_registered() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    open_project(&state, root.path()).await;
    let empty = root.path().join("empty.mp4");
    std::fs::write(&empty, b"").unwrap();
    let refused = dispatch(&state, "media.import", json!({"path": empty}), test_actor()).await;
    assert!(!refused.ok);
    let error = refused.error.unwrap();
    assert_eq!(error.code, "unreadable_media");
    assert!(error.message.contains("empty.mp4"), "{}", error.message);
    assert!(
        error.message.contains("empty (0 bytes)"),
        "{}",
        error.message
    );
    assert!(error.suggested_action.is_some());

    let folder = root.path().join("clips.mp4");
    std::fs::create_dir(&folder).unwrap();
    let refused = dispatch(
        &state,
        "media.import",
        json!({"path": folder}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    let error = refused.error.unwrap();
    assert_eq!(error.code, "invalid_args");
    assert!(error.message.contains("is a folder"), "{}", error.message);

    assert_eq!(assets(&state).await, json!({}));
    assert_eq!(op_verbs(&state).await, ["project.create"]);
}

#[tokio::test]
async fn relink_to_a_file_that_is_not_media_is_refused_and_keeps_the_source() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    open_project(&state, root.path()).await;
    let clip = root.path().join("clip.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x90:r=30",
            "-t",
            "0.5",
        ])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-y"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(made.success());
    let imported = dispatch(&state, "media.import", json!({"path": clip}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    let result = imported.result.unwrap();
    let job = wait_job(&state, result["job_id"].as_str().unwrap(), 30).await;
    assert_eq!(job.state, JobState::Done, "{job:?}");
    let before = assets(&state).await["a1"]["path"].clone();

    let text = root.path().join("replacement.mp4");
    std::fs::write(&text, text_bytes()).unwrap();
    let relinked = dispatch(
        &state,
        "media.relink",
        json!({"asset": "a1", "path": text}),
        test_actor(),
    )
    .await;
    assert!(!relinked.ok, "a text file must not become the source");
    let error = relinked.error.unwrap();
    assert_eq!(error.code, "unreadable_media", "{error:?}");
    assert!(error.message.starts_with("replacement.mp4 cannot be used"));
    assert_eq!(assets(&state).await["a1"]["path"], before);
}

#[tokio::test]
async fn import_analysis_that_cannot_be_recorded_is_failed_before_work_starts() {
    for missing_jobs_dir in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::new();
        open_project(&state, root.path()).await;
        for job in state.jobs.list() {
            wait_job(&state, &job.job_id, 30).await;
        }
        let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
        let jobs_dir = project_dir.join("jobs");
        let next = state
            .jobs
            .list()
            .iter()
            .map(|job| {
                job.job_id
                    .strip_prefix("job_")
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
            })
            .max()
            .unwrap_or(0)
            + 1;
        let record = jobs_dir.join(format!("job_{next:03}.json"));
        if missing_jobs_dir {
            std::fs::rename(&jobs_dir, project_dir.join("saved-jobs")).unwrap();
        } else {
            std::fs::create_dir(&record).unwrap();
        }
        let media = root.path().join("picture.png");
        std::fs::write(
            &media,
            include_bytes!("../../../../desktop/src-tauri/icons/32x32.png"),
        )
        .unwrap();
        let imported = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
        assert!(
            imported.ok,
            "the asset registration still succeeded: {:?}",
            imported.error
        );
        let result = imported.result.unwrap();
        let job_id = result["job_id"].as_str().unwrap();
        let job = state.jobs.get(job_id).unwrap();
        eprintln!("real job-storage fault: missing_dir={missing_jobs_dir}, answer=ok, job={job:?}");
        assert!(
            job.persistence_error.is_some(),
            "the filesystem fault must actually fail recording"
        );
        assert!(!record.is_file(), "no durable queued record was written");
        assert_eq!(
            job.state,
            JobState::Failed,
            "analysis must not be accepted without its job record"
        );
        let error = job.error.unwrap();
        assert_eq!(error.code, "io");
        assert!(
            error.message.contains("Media analysis did not start"),
            "{error:?}"
        );
        assert!(error.cause.contains("could not be saved"), "{error:?}");
        assert!(
            error
                .suggested_action
                .as_deref()
                .unwrap_or_default()
                .contains("reopen the project"),
            "{error:?}"
        );
        tokio::task::yield_now().await;
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let asset = &store.project.assets[result["asset_id"].as_str().unwrap()];
        assert!(asset.probe.is_none(), "no analysis worker started");
        assert!(asset.filmstrip.is_none(), "no thumbnail worker started");
        assert!(store
            .project
            .tracks
            .iter()
            .all(|track| track.clips.is_empty()));
        drop(guard);
        if missing_jobs_dir {
            std::fs::rename(project_dir.join("saved-jobs"), &jobs_dir).unwrap();
        } else {
            std::fs::remove_dir(&record).unwrap();
        }
        let reopened = dispatch(
            &state,
            "project.open",
            json!({"path": project_dir}),
            test_actor(),
        )
        .await;
        assert!(reopened.ok, "{:?}", reopened.error);
        let recovered = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if let Some(job) = state
                    .jobs
                    .list()
                    .into_iter()
                    .find(|job| job.kind == "import_chain" && job.job_id != job_id)
                {
                    break job;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("reopening must retry analysis for the saved asset");
        let recovered = wait_job(&state, &recovered.job_id, 30).await;
        assert_eq!(recovered.state, JobState::Done, "{recovered:?}");
        let guard = state.project.read().await;
        let assets = &guard.as_ref().unwrap().project.assets;
        assert_eq!(assets.len(), 1, "retry must reuse the saved asset");
        assert!(assets[result["asset_id"].as_str().unwrap()].probe.is_some());
    }
}
