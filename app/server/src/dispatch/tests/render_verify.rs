use super::test_actor;
use crate::dispatch::{dispatch, update_asset};
use crate::state::AppState;
use cut_core::error_codes;
use serde_json::json;

// ----------------------------------------------------------------------
// render.queue — batch delivery orchestrator (validation contract).
// The actual sequential render + both output mp4s are proven by the LIVE
// test (a real cutd); these unit tests pin the synchronous validation
// contract (no ffmpeg, no spawned render) so a regression trips in CI.
// ----------------------------------------------------------------------

/// An empty / missing `jobs` array is invalid_args — even with no project
/// open (parsed + checked before the project gate), so the coverage audit's
/// `{}` POST returns a structured envelope.
#[tokio::test]
async fn render_queue_rejects_empty_jobs() {
    let state = AppState::new();
    for args in [json!({}), json!({"jobs": []})] {
        let r = dispatch(&state, "render.queue", args, test_actor()).await;
        assert!(!r.ok, "empty jobs must error");
        assert_eq!(r.error.unwrap().code, "invalid_args");
    }
}

/// With a non-empty queue but no open project, render.queue fails fast with
/// no_project (the up-front gate) — before any render.final dispatch.
#[tokio::test]
async fn render_queue_requires_open_project() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "render.queue",
        json!({"jobs": [{"output": "out.mp4"}]}),
        test_actor(),
    )
    .await;
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, "no_project");
}

/// `output` and `path` on the same entry are the deliver-page alias colliding
/// with the native key — invalid_args, caught before any render.final dispatch.
#[tokio::test]
async fn render_queue_rejects_output_and_path_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name": "q", "dir": dir.path().join("q.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "render.queue",
        json!({"jobs": [{"output": "a.mp4", "path": "b.mp4"}]}),
        test_actor(),
    )
    .await;
    assert!(!r.ok);
    let e = r.error.unwrap();
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.message.contains("output") && e.message.contains("path"),
        "names the colliding keys: {}",
        e.message
    );
}

/// FAIL-FAST: a malformed entry (bad profile) is rejected UP FRONT by the
/// compiled nested queue schema, tagged with its exact queue path — and because
/// validation precedes the spawn, the (valid) earlier entries are NOT rendered.
#[tokio::test]
async fn render_queue_failfast_tags_bad_entry_index() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name": "q", "dir": dir.path().join("q.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Entry 0 is valid (default render); entry 1 has a bogus profile.
    let r = dispatch(
        &state,
        "render.queue",
        json!({"jobs": [{}, {"profile": "bogus_profile"}]}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "a bad entry fails the whole queue");
    let e = r.error.unwrap();
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.message.contains("/jobs/1/profile") && e.message.contains("enum"),
        "the error names the offending queue path and constraint: {}",
        e.message
    );
    // Nothing was queued: no render_queue job record exists (failed before spawn).
    assert!(
        !state.jobs.list().iter().any(|j| j.kind == "render_queue"),
        "no queue job is created when validation fails"
    );
}

/// verify.pregate WIRING (the heuristic itself is covered exhaustively by the
/// cut-perception unit tests): it errors honestly with no project open, and on
/// an empty open project it returns a clean structured pass with the thresholds
/// echoed. The real-media empty_tail/slideshow paths are proven live.
#[tokio::test]
async fn verify_pregate_wiring_no_project_then_empty_pass() {
    let state = AppState::new();
    // No project open -> structured no_project error (never a panic / fake pass).
    let r = dispatch(&state, "verify.pregate", json!({}), test_actor()).await;
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, error_codes::NO_PROJECT);
    // Open an empty project.
    let dir = tempfile::tempdir().unwrap();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Empty timeline -> no picture, no clips -> clean pass, no risks.
    let r = dispatch(&state, "verify.pregate", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["pass"], true);
    assert!(res["risks"].as_array().unwrap().is_empty());
    assert!(res["summary"].is_string());
    // Thresholds echoed for audit (mirrors the other verify.* receipts).
    assert!(res["thresholds"]["empty_tail_tolerance_ms"].is_number());
    assert_eq!(res["perception_assets"], 0);
}

/// render.final{dry_run} returns the render PLAN (geometry, duration, checks)
/// WITHOUT encoding — no render job is created.
#[tokio::test]
async fn render_final_dry_run_plans_without_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    update_asset(&state, "a1", |a| {
        a.probe = Some(
            json!({"kind":"video","width":1920,"height":1080,"duration_ms":5000,"has_audio":true}),
        );
    })
    .await
    .unwrap();
    dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000],"ripple":false}),
        test_actor(),
    )
    .await;
    let r = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["dry_run"], true);
    assert_eq!(res["output"]["duration_ms"], 5000);
    assert_eq!(res["output"]["width"], 1920);
    assert!(res["checks"]
        .as_array()
        .unwrap()
        .contains(&json!("cut_on_word")));
    assert!(res["segment_count"].as_u64().unwrap() >= 1);
    // No render job was created (dry run is pure planning).
    let jobs = dispatch(&state, "jobs.list", json!({}), test_actor()).await;
    let render_jobs = jobs.result.unwrap()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|j| j["kind"] == "render")
        .count();
    assert_eq!(render_jobs, 0, "dry_run must not create a render job");
}

#[tokio::test]
async fn jobs_cancel_aborts_active_job_through_dispatch() {
    let state = AppState::new();
    let job = state.jobs.create("render");
    let job_id = job.job_id.clone();
    state.jobs.spawn(&job_id, async {
        std::future::pending::<()>().await;
    });

    let r = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": job_id}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["cancelled"], true);
}
