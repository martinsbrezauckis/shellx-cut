//! A project change must not cancel running work silently (item 40).
use super::*;

async fn project_with_job(kind: &str) -> (AppState, tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let a = dir.path().join("a.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"a","dir":a}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let b = dir.path().join("b.cutproj");
    let made = dispatch(
        &state,
        "project.create",
        json!({"name":"b","dir":b}),
        test_actor(),
    )
    .await;
    assert!(made.ok, "{:?}", made.error);
    // Back on a.cutproj, then start the job there.
    let reopened = dispatch(&state, "project.open", json!({"path":a}), test_actor()).await;
    assert!(reopened.ok, "{:?}", reopened.error);
    let job = state.jobs.create(kind);
    state
        .jobs
        .progress(&job.job_id, 0.4, Some("working".into()));
    state.jobs.spawn(&job.job_id, std::future::pending());
    (state, dir, b, job.job_id)
}

async fn open_project_name(state: &AppState) -> String {
    let current = dispatch(state, "project.state", json!({}), test_actor()).await;
    current.result.unwrap()["name"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn running_render_is_not_cancelled_by_a_refused_project_open() {
    let (state, _dir, b, job_id) = project_with_job("render").await;
    for (verb, args) in [
        ("project.open", json!({"path": b})),
        ("project.close", json!({})),
        (
            "project.create",
            json!({"name":"c","dir":b.with_file_name("c.cutproj")}),
        ),
    ] {
        let refused = dispatch(&state, verb, args, test_actor()).await;
        let error = refused
            .error
            .unwrap_or_else(|| panic!("{verb} must ask first"));
        assert_eq!(error.code, "jobs_running", "{verb}");
        assert!(error.cause.contains(&job_id), "{verb}: {}", error.cause);
        assert!(
            error
                .suggested_action
                .as_deref()
                .is_some_and(|a| a.contains("cancel_running_jobs")),
            "{verb}"
        );
        assert_eq!(
            open_project_name(&state).await,
            "a",
            "{verb}: project stays open"
        );
        let job = state.jobs.get(&job_id).unwrap();
        assert_eq!(
            job.state,
            crate::jobs::JobState::Running,
            "{verb}: the render keeps running"
        );
        assert_eq!(job.outcome, None, "{verb}");
    }
}

#[tokio::test]
async fn confirmed_project_open_cancels_the_render_and_names_it() {
    let (state, _dir, b, job_id) = project_with_job("render").await;
    let opened = dispatch_retry_pending(
        &state,
        "project.open",
        json!({"path": b, "cancel_running_jobs": true}),
        test_actor(),
    )
    .await;
    assert!(opened.ok, "{:?}", opened.error);
    let cancelled = &opened.result.unwrap()["cancelled_jobs"];
    assert_eq!(cancelled[0]["job_id"], job_id);
    assert_eq!(cancelled[0]["kind"], "render");
    assert_eq!(open_project_name(&state).await, "b");
}

#[tokio::test]
async fn other_running_jobs_are_cancelled_and_listed_not_hidden() {
    let (state, _dir, b, job_id) = project_with_job("proxy").await;
    let opened =
        dispatch_retry_pending(&state, "project.open", json!({"path": b}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);
    let cancelled = &opened.result.unwrap()["cancelled_jobs"];
    assert_eq!(cancelled[0]["job_id"], job_id);
    assert_eq!(cancelled[0]["kind"], "proxy");
}

#[tokio::test]
async fn a_project_change_without_running_jobs_has_no_cancelled_jobs_field() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"a","dir":dir.path().join("a.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    assert!(created.result.unwrap().get("cancelled_jobs").is_none());
}

/// Item 135: work someone asked for that was missing from the old list (the
/// director render of a reframe, its QC sheet, a raw recording copy, a
/// generation, a transcript) is no longer cancelled without asking.
#[tokio::test]
async fn requested_work_beyond_renders_asks_before_a_project_change() {
    for kind in [
        "reframe-direct",
        "reframe-qc",
        "screen_record_copy_raw",
        "asset_generate",
        "transcribe",
    ] {
        let (state, _dir, b, job_id) = project_with_job(kind).await;
        let refused = dispatch(&state, "project.open", json!({"path": b}), test_actor()).await;
        let error = refused
            .error
            .unwrap_or_else(|| panic!("{kind}: project.open must ask first"));
        assert_eq!(error.code, "jobs_running", "{kind}");
        assert!(error.cause.contains(&job_id), "{kind}: {}", error.cause);
        assert_eq!(open_project_name(&state).await, "a", "{kind}");
    }
}
