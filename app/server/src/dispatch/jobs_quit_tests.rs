use super::*;
use cut_core::Actor;

fn pending_job(state: &AppState, kind: &str) -> String {
    let job = state.jobs.create(kind);
    let job_id = job.job_id.clone();
    state.jobs.spawn(&job_id, async {
        std::future::pending::<()>().await;
    });
    job_id
}

#[tokio::test]
async fn quitting_during_an_export_refuses_and_names_it() {
    let state = AppState::new();
    let export = pending_job(&state, "render");
    let reply = dispatch(&state, "jobs.before_quit", json!({}), Actor::system()).await;
    assert!(!reply.ok);
    let error = reply.error.unwrap();
    assert_eq!(error.code, "jobs_running");
    assert!(error.cause.contains(&export), "{}", error.cause);
    assert!(error.cause.contains("render"), "{}", error.cause);
    assert!(error
        .suggested_action
        .as_deref()
        .is_some_and(|action| action.contains("cancel_running_jobs:true")));
    // Nothing was stopped by asking.
    assert!(state
        .jobs
        .active_jobs()
        .iter()
        .any(|job| job.job_id == export));
}

#[tokio::test]
async fn a_confirmed_quit_stops_every_job_the_normal_way() {
    let state = AppState::new();
    let export = pending_job(&state, "render");
    let proxy = pending_job(&state, "proxy");
    let reply = dispatch(
        &state,
        "jobs.before_quit",
        json!({"cancel_running_jobs": true}),
        Actor::system(),
    )
    .await;
    assert!(reply.ok, "{:?}", reply.error);
    let cancelled = reply.result.unwrap()["cancelled_jobs"].clone();
    let ids: Vec<&str> = cancelled
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["job_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&export.as_str()) && ids.contains(&proxy.as_str()));
    assert!(state.jobs.active_jobs().is_empty());
}

#[tokio::test]
async fn automatic_work_alone_does_not_hold_a_quit() {
    let state = AppState::new();
    let _proxy = pending_job(&state, "proxy");
    let reply = dispatch(&state, "jobs.before_quit", json!({}), Actor::system()).await;
    assert!(reply.ok, "{:?}", reply.error);
}

/// F6: a job's message is free text. The quit question takes one record per
/// job from the reply, so a message with ';' and ',' stays one job.
#[tokio::test]
async fn the_refusal_names_each_running_job_once_as_a_record() {
    let state = AppState::new();
    let export = pending_job(&state, "render");
    let message = "encoding (Software encoding uses 32 of 96 threads; it will take longer, or choose HEVC to encode in hardware.)";
    state.jobs.progress(&export, 0.02, Some(message.into()));
    let reply = dispatch(&state, "jobs.before_quit", json!({}), Actor::system()).await;
    assert!(!reply.ok);
    assert_eq!(reply.error.as_ref().unwrap().code, "jobs_running");
    let running = reply.result.unwrap()["running"].as_array().unwrap().clone();
    assert_eq!(running.len(), 1, "{running:?}");
    assert_eq!(running[0]["job_id"], json!(export));
    assert_eq!(running[0]["kind"], json!("render"));
    assert_eq!(running[0]["message"], json!(message));
    assert!((running[0]["progress"].as_f64().unwrap() - 0.02).abs() < 1e-6);
}
