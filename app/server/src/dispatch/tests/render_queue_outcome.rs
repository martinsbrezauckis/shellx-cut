//! A render queue's own outcome follows its deliveries (row 201b): one failed
//! of one is a failed job; one failed of three is "done, but 1 of 3 failed"
//! (`items_failed`), never a plain "done"; and a recipe that waits on a queue
//! stops when some deliveries failed instead of carrying on.
//!
//! A delivery is made to fail the way the Windows run saw it: its explicit
//! output folder is gone (here replaced by a file) when the render writes.

use super::render_check_outcome::{project_with, StandInAnalysis, ON_TARGET_LUFS};
use super::{lock_agent_cli_env, test_actor, wait_job};
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A delivery folder inside the project, present for the admission dry-run.
fn delivery_dir(root: &Path) -> PathBuf {
    let dir = root.join("outcome.cutproj").join("deliveries");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Make the folder unusable: a plain file where the folder was.
fn vanish(dir: &Path) {
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::write(dir, b"not a folder").unwrap();
}

async fn queue(state: &AppState, jobs: Value) -> (Value, String) {
    let queued = dispatch(state, "render.queue", json!({"jobs": jobs}), test_actor()).await;
    assert!(queued.ok, "{:?}", queued.error);
    let id = queued.result.unwrap()["queue_id"]
        .as_str()
        .unwrap()
        .to_string();
    (Value::Null, id)
}

async fn settle(state: &AppState, id: &str) -> Value {
    serde_json::to_value(wait_job(state, id, 300).await).unwrap()
}

#[tokio::test]
async fn a_queue_whose_only_delivery_failed_is_a_failed_job() {
    let _env = lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(ON_TARGET_LUFS);
    let (root, state) = project_with("one.mp4", 100).await;
    let dir = delivery_dir(root.path());
    let (_, id) = queue(
        &state,
        json!([{"output": dir.join("only.mp4"), "preset": "draft", "hardware": "off"}]),
    )
    .await;
    vanish(&dir);
    let job = settle(&state, &id).await;
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["outcome"], "failed", "{job}");
    assert_eq!(job["outcome_reason"], "true_failure", "{job}");
    assert!(job["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("None of the 1 deliveries was written: "));
    let result = &job["result"];
    assert_eq!(result["failed"], 1, "{job}");
    assert_eq!(result["succeeded"], 0, "{job}");
    assert_eq!(
        result["summary_line"],
        "render queue: 0/1 delivered, 1 failed"
    );
    assert!(result["failures"][0]
        .as_str()
        .unwrap()
        .starts_with("delivery 1: "));
}

#[tokio::test]
async fn a_queue_with_one_failed_delivery_of_three_says_so() {
    let _env = lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(ON_TARGET_LUFS);
    let (root, state) = project_with("three.mp4", 100).await;
    let dir = delivery_dir(root.path());
    // The failing delivery is the LAST one, so its folder is gone long before
    // the queue reaches it (deliveries render one after another).
    let (_, id) = queue(
        &state,
        json!([
            {"preset": "draft", "hardware": "off"},
            {"preset": "draft", "hardware": "off"},
            {"output": dir.join("third.mp4"), "preset": "draft", "hardware": "off"},
        ]),
    )
    .await;
    vanish(&dir);
    let job = settle(&state, &id).await;
    assert_eq!(job["state"], "done", "{job}");
    assert_eq!(job["completion"], "done_with_warnings", "{job}");
    assert_eq!(job["outcome_reason"], "items_failed", "{job}");
    assert_eq!(job["outcome"], "partial", "not a success: {job}");
    assert_eq!(job["message"], "done, but 1 of 3 failed", "{job}");
    let result = &job["result"];
    assert_eq!(
        result["summary_line"],
        "render queue: 2/3 delivered, 1 failed"
    );
    assert_eq!(result["failed"], 1);
    assert_eq!(result["succeeded"], 2);
    let failures = result["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{job}");
    assert!(failures[0].as_str().unwrap().starts_with("delivery 3: "));
    assert_eq!(result["warnings"][0], failures[0]);
    // jobs.list carries the same record.
    let listed = dispatch(&state, "jobs.list", json!({}), test_actor()).await;
    let row = listed.result.unwrap()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["job_id"] == id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(row["outcome_reason"], "items_failed");
    assert_eq!(row["message"], "done, but 1 of 3 failed");
}

#[tokio::test]
async fn a_recipe_waiting_on_a_queue_stops_when_a_delivery_failed() {
    let _env = lock_agent_cli_env();
    let _analysis = StandInAnalysis::install(ON_TARGET_LUFS);
    let (root, state) = project_with("recipe.mp4", 100).await;
    let dir = delivery_dir(root.path());
    let target = dir.join("second.mp4");
    let recipe: crate::recipes::Recipe = serde_json::from_value(json!({
        "name": "queue-then-marker", "title": "queue", "description": "queue then a marker",
        "params": {},
        "stages": [
            { "id": "q", "verb": "render.queue", "args": {"jobs": [
                {"preset": "draft", "hardware": "off"},
                {"output": target, "preset": "draft", "hardware": "off"}
            ]}},
            { "id": "after", "verb": "edit.add_marker", "args": {"at_ms": 0, "label": "after the queue"} }
        ]
    }))
    .unwrap();
    let started: cut_core::VerbResult =
        super::super::run_resolved_recipe(&state, recipe, json!({}), None, None, test_actor())
            .await
            .into();
    assert!(started.ok, "{:?}", started.error);
    // Wait until the queue job exists, then take its second folder away.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let listed = dispatch(&state, "jobs.list", json!({}), test_actor()).await;
        let has_queue = listed.result.unwrap()["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["kind"] == "render_queue");
        if has_queue {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no queue started");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    vanish(&dir);
    let id = started.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let job = settle(&state, &id).await;
    assert_eq!(
        job["state"], "failed",
        "a stopped recipe is not done: {job}"
    );
    let report = &job["result"];
    assert_eq!(report["status"], "failed", "{job}");
    assert_eq!(report["stages_run"], 1, "the next stage never ran: {job}");
    assert!(
        job["error"]["message"]
            .as_str()
            .unwrap()
            .contains("done, but 1 of 2 failed"),
        "{job}"
    );
    let guard = state.project.read().await;
    assert!(
        guard.as_ref().unwrap().project.markers.is_empty(),
        "no stage ran after the queue"
    );
}
