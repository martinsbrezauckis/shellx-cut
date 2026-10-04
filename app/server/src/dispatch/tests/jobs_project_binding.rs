//! Stale job requests must not cross an open-project transition by reused ID.

use super::*;
use std::sync::Arc;

async fn create(state: &AppState, name: &str, root: &std::path::Path) {
    let result = dispatch(
        state,
        "project.create",
        json!({"name":name,"dir":root.join(format!("{name}.cutproj"))}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{name}: {:?}", result.error);
}

async fn digest(state: &AppState) -> String {
    let result = dispatch(state, "project.state", json!({}), test_actor()).await;
    result.result.unwrap()["project_identity"]["origin_path_sha256"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn scoped_status_refuses_recovered_job_id_collision_after_project_switch() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    create(&state, "a", root.path()).await;
    let a_digest = digest(&state).await;
    let a_job = state.jobs.create("screen_record_copy_raw");

    create(&state, "b", root.path()).await;
    // Independently persisted B history begins at the same sequence number.
    let b_root = root.path().join("b.cutproj");
    let seed = crate::jobs::JobManager::new(crate::events::EventBus::new());
    seed.attach_project(&b_root).unwrap();
    let b_job = seed.create("screen_record_copy_raw");
    assert_eq!(a_job.job_id, b_job.job_id);

    let reopened = dispatch(
        &state,
        "project.open",
        json!({"path":root.path().join("a.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(reopened.ok, "{:?}", reopened.error);
    let reopened = dispatch(&state, "project.open", json!({"path":b_root}), test_actor()).await;
    assert!(reopened.ok, "{:?}", reopened.error);
    let b_digest = digest(&state).await;
    assert_ne!(a_digest, b_digest);
    let stale = dispatch(
        &state,
        "jobs.status",
        json!({"job_id":a_job.job_id,"expected_origin_path_sha256":a_digest}),
        test_actor(),
    )
    .await;
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let scoped = dispatch(
        &state,
        "jobs.status",
        json!({"job_id":a_job.job_id,"expected_origin_path_sha256":b_digest}),
        test_actor(),
    )
    .await;
    assert!(scoped.ok, "{:?}", scoped.error);
    assert_eq!(scoped.result.unwrap()["job_id"], a_job.job_id);
    let legacy = dispatch(
        &state,
        "jobs.status",
        json!({"job_id":a_job.job_id}),
        test_actor(),
    )
    .await;
    assert!(legacy.ok, "{:?}", legacy.error);

    let rename = dispatch(
        &state,
        "project.rename",
        json!({"name":"new label"}),
        test_actor(),
    )
    .await;
    assert!(rename.ok, "{:?}", rename.error);
    assert_eq!(digest(&state).await, b_digest);
    let after_rename = dispatch(
        &state,
        "jobs.status",
        json!({"job_id":a_job.job_id,"expected_origin_path_sha256":b_digest}),
        test_actor(),
    )
    .await;
    assert!(after_rename.ok, "{:?}", after_rename.error);
}

#[tokio::test]
async fn scoped_cancel_refuses_active_foreign_job_and_unscoped_cancel_still_works() {
    let root = tempfile::tempdir().unwrap();
    let a = AppState::new();
    create(&a, "a", root.path()).await;
    let a_digest = digest(&a).await;

    let b = AppState::new();
    create(&b, "b", root.path()).await;
    let b_digest = digest(&b).await;
    let job = b.jobs.create("screen_record_copy_raw");
    b.jobs.spawn(&job.job_id, std::future::pending());
    let stale = dispatch(
        &b,
        "jobs.cancel",
        json!({"job_id":job.job_id,"expected_origin_path_sha256":a_digest}),
        test_actor(),
    )
    .await;
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    assert_eq!(
        b.jobs.get(&job.job_id).unwrap().state,
        crate::jobs::JobState::Queued
    );
    let scoped = dispatch(
        &b,
        "jobs.status",
        json!({"job_id":job.job_id,"expected_origin_path_sha256":b_digest}),
        test_actor(),
    )
    .await;
    assert!(scoped.ok, "{:?}", scoped.error);
    let renamed = dispatch(
        &b,
        "project.rename",
        json!({"name":"b renamed"}),
        test_actor(),
    )
    .await;
    assert!(renamed.ok, "{:?}", renamed.error);
    assert_eq!(digest(&b).await, b_digest);
    let cancelled = dispatch(
        &b,
        "jobs.cancel",
        json!({"job_id":job.job_id,"expected_origin_path_sha256":b_digest}),
        test_actor(),
    )
    .await;
    assert!(cancelled.ok, "{:?}", cancelled.error);
    assert_eq!(cancelled.result.unwrap()["cancelled"], true);

    let legacy_job = b.jobs.create("screen_record_copy_raw");
    b.jobs.spawn(&legacy_job.job_id, std::future::pending());
    let legacy = dispatch(
        &b,
        "jobs.cancel",
        json!({"job_id":legacy_job.job_id}),
        test_actor(),
    )
    .await;
    assert!(legacy.ok, "{:?}", legacy.error);
    assert_eq!(legacy.result.unwrap()["cancelled"], true);
}

#[tokio::test]
async fn malformed_or_empty_expected_origin_never_falls_back_to_unscoped_lookup() {
    let state = AppState::new();
    for digest in ["", "sha256:not-a-digest", "/private/project.cutproj"] {
        for name in ["jobs.status", "jobs.cancel"] {
            let result = dispatch(
                &state,
                name,
                json!({"job_id":"job_000001","expected_origin_path_sha256":digest}),
                test_actor(),
            )
            .await;
            assert_eq!(
                result.error.unwrap().code,
                error_codes::INVALID_ARGS,
                "{name}: {digest}"
            );
        }
    }
}

#[tokio::test]
async fn queued_project_switch_finishes_before_scoped_job_lookup() {
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::new());
    create(&state, "a", root.path()).await;
    let a_digest = digest(&state).await;
    let a_job = state.jobs.create("screen_record_copy_raw");
    create(&state, "b", root.path()).await;
    let b_path = root.path().join("b.cutproj");
    let a_path = root.path().join("a.cutproj");
    let opened = dispatch(&state, "project.open", json!({"path":a_path}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);

    let hold = state.project_transition.lock().await;
    let open_state = Arc::clone(&state);
    let open = tokio::spawn(async move {
        dispatch(
            &open_state,
            "project.open",
            json!({"path":b_path}),
            test_actor(),
        )
        .await
    });
    tokio::task::yield_now().await;
    let status_state = Arc::clone(&state);
    let status = tokio::spawn(async move {
        dispatch(
            &status_state,
            "jobs.status",
            json!({"job_id":a_job.job_id,"expected_origin_path_sha256":a_digest}),
            test_actor(),
        )
        .await
    });
    tokio::task::yield_now().await;
    assert!(!status.is_finished());
    drop(hold);
    assert!(open.await.unwrap().ok);
    assert_eq!(
        status.await.unwrap().error.unwrap().code,
        error_codes::CONFLICT
    );
}
