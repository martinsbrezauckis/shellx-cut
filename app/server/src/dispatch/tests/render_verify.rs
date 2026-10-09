use super::test_actor;
use crate::dispatch::{dispatch, parse_args, publish_render_final_args, update_asset, PublishArgs};
use crate::state::AppState;
use cut_core::error_codes;
use serde_json::json;

// ----------------------------------------------------------------------
// render.queue — batch delivery orchestrator (validation contract).
// These unit tests pin the synchronous validation contract without ffmpeg or
// spawned render processes.
// ----------------------------------------------------------------------

/// An empty / missing `jobs` array is invalid_args — even with no project
/// open (parsed + checked before the project gate), so `{}` returns a
/// structured envelope.
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
#[test]
fn render_final_dry_run_plans_without_encoding() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
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
    });
}

// ----------------------------------------------------------------------
// export.publish — delegation contract to render.final. The footage QC
// `profile` must pass through so silent screen-demo
// publishes stop failing caption/loudness checks that don't apply).
// ----------------------------------------------------------------------

/// PURE delegation contract: the composed render.final args carry the platform
/// spec always, and the footage `profile` VERBATIM only when the caller sent it
/// — absence keeps the key out entirely (render.final's own default battery,
/// today's behavior).
#[test]
fn export_publish_profile_passes_through_to_render_final_args() {
    let spec = cut_media::render::platform_spec("tiktok").expect("tiktok is a known platform");

    // profile present → forwarded verbatim alongside the spec-derived args.
    let a: PublishArgs = parse_args(json!({
        "platform": "tiktok",
        "profile": "silent_screen_demo",
        "preset": "high",
        "hardware": "off",
    }))
    .expect("valid publish args must parse");
    let rf = publish_render_final_args(&a, &spec);
    assert_eq!(rf["profile"], json!("silent_screen_demo"));
    // Existing passthroughs and spec-derived args are untouched by the fix.
    assert_eq!(rf["preset"], json!("high"));
    assert_eq!(rf["hardware"], json!("off"));
    assert_eq!(rf["width"], json!(1080));
    assert_eq!(rf["height"], json!(1920));
    assert_eq!(rf["bitrate"], json!("12000k"));

    // profile absent → NO profile key (not null, not a default): render.final
    // must see exactly what a direct caller omitting the arg would send.
    let a: PublishArgs =
        parse_args(json!({ "platform": "tiktok" })).expect("minimal publish args must parse");
    let rf = publish_render_final_args(&a, &spec);
    assert!(
        !rf.contains_key("profile"),
        "absent profile must stay absent in the delegated args: {rf:?}"
    );
    // dry_run:false likewise stays out (render.final treats absence as false).
    assert!(!rf.contains_key("dry_run"));
}

/// Full-dispatch proof: export.publish{profile} passes the compiled schema gate
/// (the property now exists with additionalProperties:false) and the delegation
/// completes through the REAL render.final dry_run path with the platform's
/// explicit geometry. A bogus profile is rejected by the schema enum BEFORE
/// dispatch, naming the exact path.
#[test]
fn export_publish_accepts_profile_and_rejects_unknown_values() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new();
        dispatch(
            &state,
            "project.create",
            json!({"name":"p","dir": dir.path().join("p.cutproj")}),
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
        // Valid profile + dry_run: schema gate passes, render.final returns the plan
        // (no encode), and the publish annotation still lands on the result.
        let r = dispatch(
            &state,
            "export.publish",
            json!({"platform": "tiktok", "profile": "silent_screen_demo", "dry_run": true}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "{:?}", r.error);
        let res = r.result.unwrap();
        assert_eq!(res["dry_run"], true);
        assert_eq!(res["publish"]["platform"], "tiktok");
        // The tiktok spec's explicit 9:16 geometry reached render.final.
        assert_eq!(res["output"]["width"], 1080);
        assert_eq!(res["output"]["height"], 1920);
        // Unknown profile → the compiled input schema rejects it up front (enum),
        // naming the offending path — never a silent fall-through to talking_head.
        let r = dispatch(
            &state,
            "export.publish",
            json!({"platform": "tiktok", "profile": "bogus_profile", "dry_run": true}),
            test_actor(),
        )
        .await;
        assert!(!r.ok, "bogus profile must be rejected");
        let e = r.error.unwrap();
        assert_eq!(e.code, error_codes::INVALID_ARGS);
        assert!(
            e.message.contains("/profile") && e.message.contains("enum"),
            "the error names the offending path and constraint: {}",
            e.message
        );
    });
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

/// Cancelling a live render_queue must abort its currently awaited child before
/// the queue can enter a later delivery. The Status Bar invokes jobs.cancel, so
/// exercise that dispatcher route rather than calling the manager directly.
#[tokio::test]
async fn jobs_cancel_render_queue_cancels_waiting_child_before_next_delivery() {
    use crate::jobs::{JobOutcome, JobOutcomeReason, JobState};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::sync::Notify;

    struct NotifyOnDrop(Arc<Notify>);
    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            self.0.notify_waiters();
        }
    }

    let state = AppState::new();
    let child = state.jobs.create("render");
    let queue = state.jobs.create("render_queue");
    let child_started = Arc::new(Notify::new());
    let child_stopped = Arc::new(Notify::new());
    let parent_waiting = Arc::new(Notify::new());
    let later_delivery_started = Arc::new(AtomicBool::new(false));

    let child_started_signal = child_started.clone();
    let child_stopped_signal = child_stopped.clone();
    state.jobs.spawn(&child.job_id, async move {
        let _notify_on_drop = NotifyOnDrop(child_stopped_signal);
        child_started_signal.notify_one();
        std::future::pending::<()>().await;
    });
    child_started.notified().await;

    state.jobs.set_waiting_on(
        &queue.job_id,
        Some(crate::jobs::JobDependencyInfo {
            job_id: child.job_id.clone(),
            kind: "render".into(),
        }),
    );
    let parent_waiting_signal = parent_waiting.clone();
    let later_delivery_signal = later_delivery_started.clone();
    state.jobs.spawn(&queue.job_id, async move {
        parent_waiting_signal.notify_one();
        child_stopped.notified().await;
        // This is the queue's next sequential delivery. Parent cancellation
        // must abort this task before the child becoming terminal can reach it.
        later_delivery_signal.store(true, Ordering::SeqCst);
    });
    parent_waiting.notified().await;

    let cancelled = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": queue.job_id}),
        test_actor(),
    )
    .await;
    assert!(cancelled.ok, "{:?}", cancelled.error);
    assert_eq!(cancelled.result.unwrap()["cancelled"], true);

    tokio::task::yield_now().await;
    assert!(
        !later_delivery_started.load(Ordering::SeqCst),
        "a cancelled queue must not begin a queued sibling after its active child stops"
    );
    for job_id in [&queue.job_id, &child.job_id] {
        let record = state
            .jobs
            .get(job_id)
            .expect("terminal job record retained");
        assert_eq!(record.state, JobState::Failed, "{job_id}");
        assert_eq!(record.outcome, Some(JobOutcome::Cancelled), "{job_id}");
        assert_eq!(
            record.outcome_reason,
            Some(JobOutcomeReason::UserCancelled),
            "{job_id}"
        );
    }
    assert_eq!(state.jobs.get(&queue.job_id).unwrap().waiting_on, None);
}

/// `waiting_on` must be read only after the parent task has stopped. Its Drop
/// models a render queue moving from one completed child to its next child at
/// the same time cancellation is requested; only the current child may stop.
#[tokio::test]
async fn jobs_cancel_render_queue_uses_child_after_parent_drain() {
    use crate::jobs::{JobOutcome, JobOutcomeReason, JobState};
    use std::sync::Arc;
    use tokio::sync::Notify;

    struct ReplaceWaitingChildOnDrop {
        jobs: crate::jobs::JobManager,
        queue_id: String,
        child_id: String,
    }
    impl Drop for ReplaceWaitingChildOnDrop {
        fn drop(&mut self) {
            self.jobs.set_waiting_on(
                &self.queue_id,
                Some(crate::jobs::JobDependencyInfo {
                    job_id: self.child_id.clone(),
                    kind: "render".into(),
                }),
            );
        }
    }

    let state = AppState::new();
    let first_child = state.jobs.create("render");
    let current_child = state.jobs.create("render");
    let queue = state.jobs.create("render_queue");
    let first_started = Arc::new(Notify::new());
    let current_started = Arc::new(Notify::new());
    for (job_id, started) in [
        (first_child.job_id.clone(), first_started.clone()),
        (current_child.job_id.clone(), current_started.clone()),
    ] {
        state.jobs.spawn(&job_id, async move {
            started.notify_one();
            std::future::pending::<()>().await;
        });
    }
    first_started.notified().await;
    current_started.notified().await;

    state.jobs.set_waiting_on(
        &queue.job_id,
        Some(crate::jobs::JobDependencyInfo {
            job_id: first_child.job_id.clone(),
            kind: "render".into(),
        }),
    );
    let replacement = ReplaceWaitingChildOnDrop {
        jobs: state.jobs.clone(),
        queue_id: queue.job_id.clone(),
        child_id: current_child.job_id.clone(),
    };
    state.jobs.spawn(&queue.job_id, async move {
        let _replace_waiting_child = replacement;
        std::future::pending::<()>().await;
    });

    let cancelled = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": queue.job_id}),
        test_actor(),
    )
    .await;
    assert!(cancelled.ok, "{:?}", cancelled.error);
    let first = state.jobs.get(&first_child.job_id).unwrap();
    assert_ne!(first.state, JobState::Failed, "stale child stayed active");
    let current = state.jobs.get(&current_child.job_id).unwrap();
    assert_eq!(current.state, JobState::Failed, "current child cancelled");
    assert_eq!(current.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(
        current.outcome_reason,
        Some(JobOutcomeReason::UserCancelled)
    );
    let parent = state.jobs.get(&queue.job_id).unwrap();
    assert_eq!(parent.state, JobState::Failed);
    assert_eq!(parent.outcome, Some(JobOutcome::Cancelled));
}

/// A child with an already-running blocking worker can return
/// job_cancel_pending. The stopped parent control must remain retryable until
/// that child drains; otherwise the next jobs.cancel would lose ownership.
#[tokio::test]
async fn jobs_cancel_render_queue_retains_parent_authority_when_child_is_pending() {
    use crate::jobs::{JobOutcome, JobOutcomeReason, JobState};
    use std::sync::mpsc;
    use std::sync::Arc;
    use tokio::sync::Notify;

    let state = AppState::new();
    let child = state.jobs.create("render");
    let queue = state.jobs.create("render_queue");
    let started = Arc::new(Notify::new());
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let worker_started = started.clone();
    state.jobs.spawn(&child.job_id, async move {
        let _ = crate::dispatch::run_blocking("test.queue_child", move || {
            worker_started.notify_one();
            release_rx.recv().expect("test releases queue child");
            Ok(())
        })
        .await;
    });
    started.notified().await;
    state.jobs.set_waiting_on(
        &queue.job_id,
        Some(crate::jobs::JobDependencyInfo {
            job_id: child.job_id.clone(),
            kind: "render".into(),
        }),
    );
    state.jobs.spawn(&queue.job_id, std::future::pending());

    let pending = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": queue.job_id}),
        test_actor(),
    )
    .await;
    assert!(
        !pending.ok,
        "a live blocking child must not be claimed cancelled"
    );
    assert_eq!(
        pending.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancel_pending")
    );
    let retained = state.jobs.get(&queue.job_id).unwrap();
    assert_ne!(retained.state, JobState::Failed, "parent remains retryable");
    assert_eq!(
        retained
            .waiting_on
            .as_ref()
            .map(|child| child.job_id.as_str()),
        Some(child.job_id.as_str())
    );

    release_tx.send(()).unwrap();
    let retried = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": queue.job_id}),
        test_actor(),
    )
    .await;
    assert!(retried.ok, "{:?}", retried.error);
    for job_id in [&queue.job_id, &child.job_id] {
        let record = state.jobs.get(job_id).unwrap();
        assert_eq!(record.state, JobState::Failed, "{job_id}");
        assert_eq!(record.outcome, Some(JobOutcome::Cancelled), "{job_id}");
        assert_eq!(
            record.outcome_reason,
            Some(JobOutcomeReason::UserCancelled),
            "{job_id}"
        );
    }
}

/// A direct child cancellation can already own and drain the active render
/// when the Status Bar cancels its parent queue. The queue must retain its
/// drained control and return pending until that child reaches a terminal
/// record; `abort(child) == false` alone is not a completed child.
#[tokio::test]
async fn jobs_cancel_render_queue_waits_for_concurrent_child_cancellation() {
    use crate::jobs::{JobOutcome, JobOutcomeReason, JobState};
    use std::sync::mpsc;
    use std::sync::Arc;
    use tokio::sync::Notify;

    let state = AppState::new();
    let child = state.jobs.create("render");
    let queue = state.jobs.create("render_queue");
    let started = Arc::new(Notify::new());
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let worker_started = started.clone();
    state.jobs.spawn(&child.job_id, async move {
        let _ = crate::dispatch::run_blocking("test.concurrent_queue_child", move || {
            worker_started.notify_one();
            release_rx.recv().expect("test releases queue child");
            Ok(())
        })
        .await;
    });
    started.notified().await;
    state.jobs.set_waiting_on(
        &queue.job_id,
        Some(crate::jobs::JobDependencyInfo {
            job_id: child.job_id.clone(),
            kind: "render".into(),
        }),
    );
    state.jobs.spawn(&queue.job_id, std::future::pending());

    let child_state = state.clone();
    let child_job_id = child.job_id.clone();
    let child_cancel = tokio::spawn(async move {
        dispatch(
            &child_state,
            "jobs.cancel",
            json!({"job_id": child_job_id}),
            test_actor(),
        )
        .await
    });
    for _ in 0..100 {
        if !state.jobs.has_active_task_for_tests(&child.job_id) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        !state.jobs.has_active_task_for_tests(&child.job_id),
        "the direct child cancellation must own the child control before queue cancellation"
    );
    assert!(
        !child_cancel.is_finished(),
        "the blocking child must still be draining while the parent cancellation runs"
    );

    let pending = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": queue.job_id}),
        test_actor(),
    )
    .await;
    assert!(
        !pending.ok,
        "the queue must not claim success while another cancellation owns its child"
    );
    assert_eq!(
        pending.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancel_pending")
    );
    let retained = state.jobs.get(&queue.job_id).unwrap();
    assert_ne!(retained.state, JobState::Failed, "parent remains retryable");
    assert_eq!(
        retained
            .waiting_on
            .as_ref()
            .map(|dependency| dependency.job_id.as_str()),
        Some(child.job_id.as_str())
    );

    release_tx.send(()).unwrap();
    let child_cancelled = child_cancel.await.unwrap();
    assert!(child_cancelled.ok, "{:?}", child_cancelled.error);

    let retried = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": queue.job_id}),
        test_actor(),
    )
    .await;
    assert!(retried.ok, "{:?}", retried.error);
    for job_id in [&queue.job_id, &child.job_id] {
        let record = state.jobs.get(job_id).unwrap();
        assert_eq!(record.state, JobState::Failed, "{job_id}");
        assert_eq!(record.outcome, Some(JobOutcome::Cancelled), "{job_id}");
        assert_eq!(
            record.outcome_reason,
            Some(JobOutcomeReason::UserCancelled),
            "{job_id}"
        );
    }
}

/// Queue admission retains its already-authorized destinations while it waits
/// for its scheduler slot. Later folder choices affect other requests only.
#[test]
fn render_queue_delayed_children_keep_admitted_output_authorization() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use crate::jobs::JobState;
        use crate::output_paths::set_session_output_dir;
        use std::sync::Arc;
        use tokio::sync::Notify;

        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().canonicalize().unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let chosen_path = chosen.path().canonicalize().unwrap();
        let later = tempfile::tempdir().unwrap();
        let later_path = later.path().canonicalize().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&chosen_path, &later_path] {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let media = dir_path.join("clip.mp4");
        assert!(std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=160x90:r=30",
                "-t",
                "0.2",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&media)
            .status()
            .unwrap()
            .success());

        for explicit in [true, false] {
            let state = AppState::new();
            let project = dir_path.join(if explicit {
                "explicit.cutproj"
            } else {
                "default.cutproj"
            });
            let created = dispatch(
                &state,
                "project.create",
                json!({"name": if explicit { "explicit" } else { "default" }, "dir": project}),
                test_actor(),
            )
            .await;
            assert!(created.ok, "{:?}", created.error);
            assert!(
                dispatch(&state, "media.import", json!({"path":media}), test_actor())
                    .await
                    .ok
            );
            update_asset(&state, "a1", |a| {
                a.probe = Some(
                    json!({"kind":"video","width":160,"height":90,"duration_ms":200,"has_audio":false}),
                );
            })
            .await
            .unwrap();
            assert!(
                dispatch(
                    &state,
                    "edit.insert",
                    json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,200],"ripple":false}),
                    test_actor()
                )
                .await
                .ok
            );

            let occupied = state.jobs.create("render_queue");
            let ready = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let ready_task = ready.clone();
            let release_task = release.clone();
            state
                .jobs
                .spawn_limited(&occupied.job_id, "render_queue", 1, async move {
                    ready_task.notify_one();
                    release_task.notified().await;
                });
            ready.notified().await;
            set_session_output_dir(Some(chosen_path.clone()));
            let mut delivery = json!({"preset":"draft","hardware":"off","width":160,"height":90});
            if explicit {
                delivery["output"] = json!(chosen_path.join("selected.mp4"));
            }
            let deliveries = if explicit {
                vec![delivery]
            } else {
                vec![delivery.clone(), delivery]
            };
            let response = dispatch(
                &state,
                "render.queue",
                json!({"jobs":deliveries}),
                test_actor(),
            )
            .await;
            assert!(response.ok, "{:?}", response.error);
            let result = response.result.unwrap();
            if explicit {
                let planned = std::path::PathBuf::from(result["jobs"][0]["output"].as_str().unwrap());
                assert_eq!(planned.parent(), Some(chosen_path.as_path()));
            } else {
                assert!(result["jobs"][0]["output"].is_null());
                assert!(result["jobs"][1]["output"].is_null(), "a second dry_run must not claim the first child output");
            }
            let queue_id = result["queue_id"].as_str().unwrap();
            set_session_output_dir(None); // the native picker restores its old default
            set_session_output_dir(Some(later_path.clone())); // another normal folder choice
            let unrelated = dispatch(
                &state,
                "render.final",
                json!({"path":chosen_path.join("unrelated.mp4"),"dry_run":true,"hardware":"off"}),
                test_actor(),
            )
            .await;
            assert!(
                !unrelated.ok,
                "a later REST request cannot inherit queue authority"
            );
            release.notify_one();
            let terminal = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    let record = state.jobs.get(queue_id).unwrap();
                    if matches!(record.state, JobState::Done | JobState::Failed) {
                        break record;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            let result = terminal.result.unwrap();
            assert_eq!(result["succeeded"], if explicit { 1 } else { 2 }, "{result}");
            assert_eq!(result["failed"], 0, "{result}");
            let mut outputs = std::collections::HashSet::new();
            for child in result["jobs"].as_array().unwrap() {
                let child_id = child["job_id"].as_str().unwrap();
                assert_eq!(state.jobs.get(child_id).unwrap().state, JobState::Done);
                let output = std::path::PathBuf::from(child["output"].as_str().unwrap());
                assert_eq!(output.parent(), Some(chosen_path.as_path()));
                assert!(output.is_file());
                assert!(outputs.insert(output), "queue children must retain separate outputs");
            }
            let later_default = crate::output_paths::fence_output_path(
                &project,
                None,
                "exports/after.mp4",
                crate::output_paths::OutputPathPolicy::MP4,
            )
            .unwrap();
            assert_eq!(
                later_default.parent(),
                Some(later_path.as_path()),
                "queue completion must not overwrite the new preference"
            );
        }
        set_session_output_dir(None);
    });
}
