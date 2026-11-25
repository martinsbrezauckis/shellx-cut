//! The polish bake as a job: visible, with progress, and bound to the project
//! it was asked in (seen in an installed run: two minutes with nothing in jobs.list,
//! and a clip added to the project another agent had opened meanwhile).

use super::*;
use crate::dispatch::dispatch;
use cut_core::Actor;
use std::sync::atomic::{AtomicBool, Ordering};

mod pending_retry {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/dispatch/tests/pending_retry.rs"
    ));
}

async fn two_projects() -> (tempfile::TempDir, PathBuf, PathBuf, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let mut dirs = Vec::new();
    for name in ["second", "first"] {
        let dir = temp.path().join(format!("{name}.cutproj"));
        let created = dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": dir}),
            Actor::system(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
        dirs.push(dir);
    }
    // "first" is open now.
    (temp, dirs[1].clone(), dirs[0].clone(), state)
}

/// A bake that reports frames and runs until `release` is set or it is
/// cancelled.
fn held_bake(release: Arc<AtomicBool>) -> BakeWork {
    Box::new(move |control, on_frame| {
        let mut frame = 0;
        while !release.load(Ordering::SeqCst) {
            control
                .check("test bake")
                .map_err(crate::screen_record::record_err)?;
            frame = (frame + 1).min(99);
            on_frame(frame, 100);
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    })
}

async fn polish_job(state: &AppState) -> Option<crate::jobs::JobRecord> {
    state.jobs.list().into_iter().find(|job| job.kind == KIND)
}

#[tokio::test]
async fn the_bake_is_a_visible_job_with_frame_progress() {
    let (_temp, first, _second, state) = two_projects().await;
    let release = Arc::new(AtomicBool::new(false));
    let task_state = state.clone();
    let bake_release = release.clone();
    let running =
        tokio::spawn(async move { bake(&task_state, "first", held_bake(bake_release)).await });
    let mut seen = None;
    for _ in 0..100 {
        if let Some(job) = polish_job(&state).await.filter(|job| job.progress > 0.0) {
            seen = Some(job);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let job = seen.expect("the polish shows as a job with progress");
    assert_eq!(job.state, crate::jobs::JobState::Running);
    assert!(
        job.message.as_deref().unwrap_or("").contains("frame"),
        "{job:?}"
    );
    assert!(
        state
            .jobs
            .active_jobs()
            .iter()
            .any(|job| job.kind == KIND && job.is_delivery()),
        "the quit question and a project change see it"
    );
    release.store(true, Ordering::SeqCst);
    let polish = running.await.unwrap().expect("bake finished");
    assert!(still_open(&state, &first).await);
    polish.placed(Ok(json!({"clip_id": "c1"}))).await;
    for _ in 0..100 {
        if polish_job(&state)
            .await
            .is_some_and(|job| job.state == crate::jobs::JobState::Done)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the job ends when the clip is placed");
}

#[tokio::test]
async fn a_project_change_while_polishing_asks_first() {
    let (_temp, first, second, state) = two_projects().await;
    let release = Arc::new(AtomicBool::new(false));
    let task_state = state.clone();
    let bake_release = release.clone();
    let running =
        tokio::spawn(async move { bake(&task_state, "first", held_bake(bake_release)).await });
    while polish_job(&state).await.is_none() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let refused = dispatch(
        &state,
        "project.open",
        json!({"path": second}),
        Actor::system(),
    )
    .await;
    let error = refused
        .error
        .expect("the change is refused while the polish runs");
    assert_eq!(error.code, "jobs_running");
    assert!(error.cause.contains(KIND), "{}", error.cause);
    assert!(still_open(&state, &first).await, "nothing changed");
    release.store(true, Ordering::SeqCst);
    running.await.unwrap().unwrap().placed(Ok(json!({}))).await;
}

#[tokio::test]
async fn a_forced_project_change_stops_the_bake_and_says_where_the_recording_stays() {
    let (_temp, first, second, state) = two_projects().await;
    let release = Arc::new(AtomicBool::new(false));
    let task_state = state.clone();
    let running = tokio::spawn(async move { bake(&task_state, "first", held_bake(release)).await });
    while polish_job(&state).await.is_none() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let forced = pending_retry::retry_pending(
        || {
            dispatch(
                &state,
                "project.open",
                json!({"path": second, "cancel_running_jobs": true}),
                Actor::system(),
            )
        },
        |reply| {
            reply
                .error
                .as_ref()
                .is_some_and(|error| error.code == "job_cancel_pending")
        },
    )
    .await;
    assert!(forced.ok, "{:?}", forced.error);
    let error = match running.await.unwrap() {
        Ok(_) => panic!("a forced change stops the bake"),
        Err(error) => error,
    };
    assert!(
        error.message.contains("stays in “first”"),
        "{}",
        error.message
    );
    assert!(!still_open(&state, &first).await);
}

#[tokio::test]
async fn a_clip_whose_project_is_no_longer_open_waits_there() {
    let (_temp, first, second, state) = two_projects().await;
    let opened = dispatch(
        &state,
        "project.open",
        json!({"path": second}),
        Actor::system(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);
    assert!(!still_open(&state, &first).await);
    let reply = waiting_reply(None, &first, "baked.mp4");
    assert_eq!(reply["placed"], json!(false));
    assert_eq!(reply["clip_id"], Value::Null);
    assert_eq!(reply["waiting_in_project"]["name"], json!("first"));
    let message = reply["message"].as_str().unwrap();
    assert!(message.contains("It waits in “first”"), "{message}");
    assert!(
        !message.contains("added to the current project"),
        "{message}"
    );
}
