use super::*;

mod pending_retry {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/dispatch/tests/pending_retry.rs"
    ));
}

/// Reopening a project must advance the job sequence past the highest
/// persisted id, so a fresh server (next_seq=0) never re-mints — and thereby
/// overwrites — an existing job_NNN.json. Regression gate for the
/// `attach_project` next-sequence regression.
#[test]
fn attach_advances_seq_past_persisted_ids() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path();
    let jobs = proj.join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    // A prior run left a finished record and a damaged newer filename.
    let rec = JobRecord {
        job_id: "job_005".into(),
        kind: "render".into(),
        state: JobState::Done,
        completion: Some(JobCompletion::Success),
        outcome: Some(JobOutcome::Succeeded),
        outcome_reason: Some(JobOutcomeReason::Completed),
        progress: 1.0,
        message: Some("done".into()),
        queue: None,
        waiting_on: None,
        retry: None,
        created_ts: "2026-06-16T00:00:00.000Z".into(),
        updated_ts: "2026-06-16T00:00:00.000Z".into(),
        result: None,
        error: None,
        persistence_error: None,
    };
    std::fs::write(
        jobs.join("job_005.json"),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
    std::fs::write(jobs.join("job_007.json"), b"not valid json").unwrap();

    // Fresh manager (next_seq=0) attaches the project, then mints a job.
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(proj).unwrap();
    let created = mgr.create("render");
    assert_eq!(
        created.job_id, "job_008",
        "next id must follow every recovered filename/history id, not restart or reuse corruption"
    );
    // The prior record is still on disk (not clobbered).
    assert!(
        jobs.join("job_005.json").exists(),
        "persisted job_005 preserved"
    );
    assert_eq!(mgr.persistence_notices().len(), 1);
}

#[test]
fn create_persists_queued_job_before_worker_starts() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();

    let created = mgr.create("render");
    let persisted = dir
        .path()
        .join("jobs")
        .join(format!("{}.json", created.job_id));
    assert!(
        persisted.exists(),
        "queued job must be crash-visible immediately"
    );

    let text = std::fs::read_to_string(persisted).unwrap();
    let rec: JobRecord = serde_json::from_str(&text).unwrap();
    assert_eq!(rec.job_id, created.job_id);
    assert_eq!(rec.kind, "render");
    assert_eq!(rec.state, JobState::Queued);
    assert_eq!(rec.completion, None);
}

#[test]
fn progress_persists_the_latest_human_phase_across_record_reads() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();
    let job = mgr.create("render_queue");

    mgr.progress(&job.job_id, 0.25, Some("rendering delivery 2/8".into()));
    mgr.progress(&job.job_id, 0.5, None);

    let active = mgr.get(&job.job_id).unwrap();
    assert_eq!(active.progress, 0.5);
    assert_eq!(active.message.as_deref(), Some("rendering delivery 2/8"));
    let persisted: JobRecord = serde_json::from_slice(
        &std::fs::read(dir.path().join("jobs").join("job_001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted.message, active.message);

    mgr.finish(&job.job_id, serde_json::json!({"ok": true}));
    assert_eq!(
        mgr.get(&job.job_id).unwrap().message.as_deref(),
        Some("done")
    );
}

#[tokio::test]
async fn detach_project_cancels_old_jobs_and_isolates_the_next_project() {
    let old = tempfile::tempdir().unwrap();
    let next = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(old.path()).unwrap();

    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let waiting = started.notified();
    let signal = started.clone();
    let job = mgr.create("enrich");
    mgr.spawn(&job.job_id, async move {
        signal.notify_one();
        std::future::pending::<()>().await;
    });
    waiting.await;

    pending_retry::retry_pending(
        || mgr.detach_project(),
        |result| {
            result
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "job_cancel_pending")
        },
    )
    .await
    .unwrap();
    assert_eq!(mgr.active_task_count_for_tests(), 0);
    assert!(mgr.list().is_empty());
    assert!(
        mgr.lock_inner().detached_job_ids.contains(&job.job_id),
        "only fully detached project jobs become quiet late-update tombstones"
    );

    let old_record: JobRecord = serde_json::from_str(
        &std::fs::read_to_string(old.path().join("jobs").join(format!("{}.json", job.job_id)))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(old_record.state, JobState::Failed);
    assert_eq!(old_record.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(
        old_record.outcome_reason,
        Some(JobOutcomeReason::ProjectSwitchCancelled)
    );
    assert_eq!(
        old_record.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancelled")
    );

    mgr.attach_project(next.path()).unwrap();
    mgr.finish(&job.job_id, serde_json::json!({"late": true}));
    assert!(
        !next
            .path()
            .join("jobs")
            .join(format!("{}.json", job.job_id))
            .exists(),
        "a late update from the detached project must not enter the next project"
    );
    let next_job = mgr.create("render");
    assert!(next
        .path()
        .join("jobs")
        .join(format!("{}.json", next_job.job_id))
        .exists());
}

#[tokio::test]
async fn detach_waits_for_spawn_blocking_worker_to_finish() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;
    use tokio::time::{sleep, timeout, Duration};

    let old = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(old.path()).unwrap();

    let job = mgr.create("render");
    let worker_started = Arc::new(Notify::new());
    let worker_finished = Arc::new(AtomicBool::new(false));
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started = worker_started.clone();
    let finished = worker_finished.clone();
    mgr.spawn(&job.job_id, async move {
        let _ = crate::dispatch::run_blocking("test.blocking_worker", move || {
            started.notify_one();
            release_rx.recv().expect("test releases worker");
            finished.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
    });
    worker_started.notified().await;

    let detach_mgr = mgr.clone();
    let detaching = tokio::spawn(async move { detach_mgr.detach_project().await });
    sleep(Duration::from_millis(50)).await;
    assert!(
        !detaching.is_finished(),
        "project detach must wait for an already-running blocking worker"
    );
    assert!(!worker_finished.load(Ordering::SeqCst));

    release_tx.send(()).unwrap();
    timeout(Duration::from_secs(1), detaching)
        .await
        .expect("detach should finish after worker exits")
        .expect("detach task should not panic")
        .expect("detach should succeed");
    assert!(worker_finished.load(Ordering::SeqCst));
    assert_eq!(mgr.active_task_count_for_tests(), 0);
}

#[tokio::test]
async fn switch_project_fails_closed_while_blocking_worker_is_alive() {
    use tokio::sync::Notify;
    use tokio::time::{timeout, Duration};

    let old = tempfile::tempdir().unwrap();
    let next = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(old.path()).unwrap();

    let job = mgr.create("render");
    let worker_started = Arc::new(Notify::new());
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started = worker_started.clone();
    mgr.spawn(&job.job_id, async move {
        let _ = crate::dispatch::run_blocking("test.blocking_worker", move || {
            started.notify_one();
            release_rx.recv().expect("test releases worker");
            Ok(())
        })
        .await;
    });
    worker_started.notified().await;

    let error = mgr.switch_project(next.path()).await.unwrap_err();
    assert_eq!(error.code, "job_cancel_pending");
    assert!(
        !next.path().join("jobs").exists(),
        "next project must not attach while an old blocking worker is alive"
    );

    release_tx.send(()).unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            if mgr
                .lock_inner()
                .tasks
                .values()
                .all(|task| !task.has_blocking_workers())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("blocking worker should exit");

    mgr.switch_project(next.path()).await.unwrap();
    let next_job = mgr.create("proxy");
    assert!(
        next.path()
            .join("jobs")
            .join(format!("{}.json", next_job.job_id))
            .exists(),
        "next project attaches only after every old worker is gone"
    );
}

#[test]
fn terminal_completion_quality_is_explicit_and_backward_compatible() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();

    let complete = mgr.create("proxy");
    mgr.finish(&complete.job_id, serde_json::json!({"proxy": true}));
    assert_eq!(
        mgr.get(&complete.job_id).unwrap().completion,
        Some(JobCompletion::Success)
    );
    assert_eq!(
        mgr.get(&complete.job_id).unwrap().outcome,
        Some(JobOutcome::Succeeded)
    );

    let partial = mgr.create("enrich");
    mgr.finish_with_warnings(
        &partial.job_id,
        serde_json::json!({"transcript": false, "warnings": ["stt unavailable"]}),
    );
    let partial = mgr.get(&partial.job_id).unwrap();
    assert_eq!(partial.state, JobState::Done);
    assert_eq!(partial.completion, Some(JobCompletion::DoneWithWarnings));
    assert_eq!(
        partial.outcome_reason,
        Some(JobOutcomeReason::CompletedWithWarnings)
    );
    let persisted = std::fs::read_to_string(
        dir.path()
            .join("jobs")
            .join(format!("{}.json", partial.job_id)),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&persisted).unwrap()["completion"],
        "done_with_warnings"
    );

    let legacy: JobRecord = serde_json::from_value(serde_json::json!({
        "job_id": "job_legacy",
        "kind": "render",
        "state": "done",
        "progress": 1.0,
        "created_ts": "2026-01-01T00:00:00Z",
        "updated_ts": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    assert_eq!(legacy.completion, None);
    assert_eq!(legacy.outcome, None);
    assert_eq!(legacy.outcome_reason, None);
}

#[test]
fn create_reports_persistence_failure_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();
    let jobs_path = dir.path().join("jobs");
    std::fs::remove_dir_all(&jobs_path).unwrap();
    std::fs::write(&jobs_path, b"not a directory").unwrap();

    let created = mgr.create("render");
    assert!(
        created.persistence_error.is_some(),
        "create should surface a queued-job persistence failure"
    );
    let live = mgr.get(&created.job_id).unwrap();
    assert_eq!(live.persistence_error, created.persistence_error);
}

#[test]
fn create_durable_refuses_persistence_failure_without_an_in_memory_record() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();
    let jobs_path = dir.path().join("jobs");
    std::fs::remove_dir_all(&jobs_path).unwrap();
    std::fs::write(&jobs_path, b"not a directory").unwrap();

    let error = mgr
        .create_durable("portable_package")
        .expect_err("durable admission must refuse an unwritable job store");
    assert_eq!(error.code, cut_core::error_codes::IO);
    assert!(
        mgr.list().is_empty(),
        "a rejected job must not become visible"
    );
}

#[test]
fn update_reports_persistence_failure_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = JobManager::new(EventBus::new());
    mgr.attach_project(dir.path()).unwrap();
    let created = mgr.create("render");
    let jobs_path = dir.path().join("jobs");
    std::fs::remove_dir_all(&jobs_path).unwrap();
    std::fs::write(&jobs_path, b"not a directory").unwrap();

    mgr.finish(&created.job_id, serde_json::json!({"path":"out.mp4"}));
    let live = mgr.get(&created.job_id).unwrap();
    assert_eq!(live.state, JobState::Done);
    assert_eq!(live.completion, Some(JobCompletion::Success));
    assert_eq!(live.result, Some(serde_json::json!({"path":"out.mp4"})));
    assert!(
        live.persistence_error.is_some(),
        "terminal transition should expose persistence failure"
    );
}

#[tokio::test]
async fn spawn_tracks_handle_and_abort_removes_it() {
    let mgr = JobManager::new(EventBus::new());
    let job = mgr.create("render");

    mgr.spawn(&job.job_id, async {
        std::future::pending::<()>().await;
    });
    assert_eq!(mgr.active_task_count_for_tests(), 1);

    assert!(pending_retry::retry_pending(
        || mgr.abort(&job.job_id),
        |result| result
            .as_ref()
            .err()
            .is_some_and(|error| error.code == "job_cancel_pending"),
    )
    .await
    .unwrap());
    assert_eq!(mgr.active_task_count_for_tests(), 0);
    let record = mgr.get(&job.job_id).unwrap();
    assert_eq!(record.state, JobState::Failed);
    assert_eq!(record.outcome, Some(JobOutcome::Cancelled));
    assert_eq!(record.outcome_reason, Some(JobOutcomeReason::UserCancelled));
}

/// 1.0 means finished work. A job that ended without finishing keeps the
/// progress it really reached; every ending is told by the event's `terminal`
/// flag, which is what a reader must use to learn the job ended.
#[tokio::test]
async fn terminal_transitions_publish_truthful_progress_and_the_terminal_flag() {
    for (transition, expected) in [("success", 1.0), ("warnings", 1.0), ("failure", 0.3)] {
        let bus = EventBus::new();
        let mut events = bus.subscribe();
        let mgr = JobManager::new(bus);
        let job = mgr.create("analysis");
        mgr.progress(&job.job_id, 0.3, Some("working".into()));
        match transition {
            "success" => mgr.finish(&job.job_id, serde_json::json!({"ok": true})),
            "warnings" => {
                mgr.finish_with_warnings(&job.job_id, serde_json::json!({"warnings": ["partial"]}))
            }
            "failure" => mgr.fail(
                &job.job_id,
                CutError::new("failed", "analysis failed", "test"),
            ),
            _ => unreachable!(),
        }
        let mut seen_running = false;
        loop {
            let event = events.recv().await.expect("job progress event");
            let crate::events::Event::JobProgress {
                job_id,
                kind,
                progress,
                message,
                terminal,
            } = event
            else {
                panic!("expected job progress");
            };
            assert_eq!(job_id, job.job_id);
            assert_eq!(kind, "analysis");
            if !terminal {
                assert_eq!(progress, 0.3, "the running event");
                seen_running = true;
                continue;
            }
            assert!(seen_running, "the running event comes first");
            assert_eq!(progress, expected, "{transition}");
            assert!(message.is_some());
            break;
        }
        assert_eq!(
            mgr.get(&job.job_id).unwrap().progress,
            expected,
            "{transition}"
        );
    }
}

#[test]
fn terminal_job_state_cannot_regress_to_running_or_other_terminal() {
    use serde_json::json;

    let mgr = JobManager::new(EventBus::new());
    let done = mgr.create("render");
    mgr.finish(&done.job_id, json!({"path":"out.mp4"}));
    mgr.progress(&done.job_id, 0.25, Some("late progress".into()));
    let rec = mgr.get(&done.job_id).unwrap();
    assert_eq!(rec.state, JobState::Done);
    assert_eq!(rec.progress, 1.0);
    assert_eq!(rec.result, Some(json!({"path":"out.mp4"})));

    let failed = mgr.create("render");
    let error = CutError::new("boom", "failed once", "test failure");
    mgr.fail(&failed.job_id, error.clone());
    mgr.finish(&failed.job_id, json!({"path":"late.mp4"}));
    let rec = mgr.get(&failed.job_id).unwrap();
    assert_eq!(rec.state, JobState::Failed);
    assert_eq!(rec.error, Some(error));
    assert_eq!(rec.result, None);
}

#[tokio::test]
async fn spawn_limited_serializes_jobs_with_the_same_key() {
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;
    use tokio::time::{sleep, timeout, Duration};

    let mgr = JobManager::new(EventBus::new());
    let first = mgr.create("enrich");
    let second = mgr.create("enrich");
    let running = Arc::new(AtomicUsize::new(0));
    let max_running = Arc::new(AtomicUsize::new(0));
    let second_started = Arc::new(AtomicBool::new(false));
    let first_gate = Arc::new(Notify::new());
    let release_first = Arc::new(Notify::new());
    let second_gate = Arc::new(Notify::new());

    let first_id = first.job_id.clone();
    let mgr_first = mgr.clone();
    let running_first = running.clone();
    let max_first = max_running.clone();
    let first_gate_task = first_gate.clone();
    let release_first_task = release_first.clone();
    mgr.spawn_limited(&first.job_id, "enrich", 1, async move {
        let active = running_first.fetch_add(1, Ordering::SeqCst) + 1;
        max_first.fetch_max(active, Ordering::SeqCst);
        first_gate_task.notify_one();
        release_first_task.notified().await;
        running_first.fetch_sub(1, Ordering::SeqCst);
        mgr_first.finish(&first_id, json!({"ok": true}));
    });

    let second_id = second.job_id.clone();
    let mgr_second = mgr.clone();
    let running_second = running.clone();
    let max_second = max_running.clone();
    let second_started_task = second_started.clone();
    let second_gate_task = second_gate.clone();
    mgr.spawn_limited(&second.job_id, "enrich", 1, async move {
        second_started_task.store(true, Ordering::SeqCst);
        let active = running_second.fetch_add(1, Ordering::SeqCst) + 1;
        max_second.fetch_max(active, Ordering::SeqCst);
        running_second.fetch_sub(1, Ordering::SeqCst);
        mgr_second.finish(&second_id, json!({"ok": true}));
        second_gate_task.notify_one();
    });

    timeout(Duration::from_secs(1), first_gate.notified())
        .await
        .expect("first limited job should start");
    sleep(Duration::from_millis(50)).await;
    let first_record = mgr.get(&first.job_id).unwrap();
    assert_eq!(first_record.state, JobState::Running);
    assert_eq!(first_record.queue, None);
    let second_record = mgr.get(&second.job_id).unwrap();
    assert_eq!(second_record.state, JobState::Queued);
    assert_eq!(
        second_record.queue,
        Some(JobQueueInfo {
            resource: "enrich".into(),
            max_running: 1,
        }),
        "the durable queued record names the constrained resource and capacity"
    );
    assert!(
        !second_started.load(Ordering::SeqCst),
        "second job must stay queued while the first holds the enrich slot"
    );

    release_first.notify_one();
    timeout(Duration::from_secs(1), second_gate.notified())
        .await
        .expect("second limited job should start after first finishes");
    assert_eq!(max_running.load(Ordering::SeqCst), 1);
    assert_eq!(mgr.get(&second.job_id).unwrap().queue, None);
}

#[tokio::test]
async fn with_limit_shares_slots_with_spawn_limited_jobs() {
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;
    use tokio::time::{sleep, timeout, Duration};

    let mgr = JobManager::new(EventBus::new());
    let job = mgr.create("enrich");
    let first_started = Arc::new(Notify::new());
    let release_first = Arc::new(Notify::new());
    let direct_started = Arc::new(AtomicBool::new(false));

    let job_id = job.job_id.clone();
    let mgr_job = mgr.clone();
    let first_started_task = first_started.clone();
    let release_first_task = release_first.clone();
    mgr.spawn_limited(&job.job_id, "analysis", 1, async move {
        first_started_task.notify_one();
        release_first_task.notified().await;
        mgr_job.finish(&job_id, json!({"ok": true}));
    });

    timeout(Duration::from_secs(1), first_started.notified())
        .await
        .expect("limited job should start first");

    let mgr_direct = mgr.clone();
    let direct_started_task = direct_started.clone();
    let direct = tokio::spawn(async move {
        mgr_direct
            .with_limit("analysis", 1, async move {
                direct_started_task.store(true, Ordering::SeqCst);
                42
            })
            .await
    });

    sleep(Duration::from_millis(50)).await;
    assert!(
        !direct_started.load(Ordering::SeqCst),
        "direct limited work must wait for the background job holding the same slot"
    );

    release_first.notify_one();
    let value = timeout(Duration::from_secs(1), direct)
        .await
        .expect("direct limited work should finish")
        .expect("direct task should not panic");
    assert_eq!(value, 42);
}
