//! jobs.rs — background job system (server contract "Job system").
//!
//! Long tasks publish progress and persist `<proj>/jobs/<id>.json`, keeping status
//! observable across crashes. Callers are dispatch plus jobs.status/list/cancel.

mod dependency;
mod outcome;
mod persistence;
mod process;
mod queue;
mod runtime;

use crate::events::EventBus;
use cut_core::CutError;
pub use dependency::JobDependencyInfo;
pub(crate) use outcome::{JobOutcome, JobOutcomeReason};
use persistence::{persist, recover, JobPersistenceNotice};
pub(crate) use process::{run_owned, ProcessControl, ProcessTermination};
pub use queue::JobQueueInfo;
use runtime::JobTaskControl;
pub(crate) use runtime::{
    begin_current_blocking_worker, begin_current_process_worker, current_job_cancellation,
    JobCancellation, JobCancellationReason,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

#[cfg(not(test))]
const JOB_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const JOB_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
const DETACHED_JOB_TOMBSTONE_LIMIT: usize = 1024;

/// Job lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

/// Terminal quality for a successfully completed job. This is separate from
/// `JobState` so existing clients can keep treating `done` as terminal while
/// still distinguishing a complete result from an explicit partial result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobCompletion {
    Success,
    DoneWithWarnings,
}

/// Persistent record of one job — what jobs.status returns and what is
/// written to `<project>/jobs/<job_id>.json` on every transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    /// "job_NNN" id, unique per server run.
    pub job_id: String,
    /// Job kind: "probe" | "proxy" | "transcribe" | "perception" | "render"
    /// | "judge" | "import_chain".
    pub kind: String,
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<JobCompletion>,
    /// Why a terminal job ended. Absent for active jobs and legacy records
    /// written before outcome tracking was introduced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<JobOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason: Option<JobOutcomeReason>,
    /// 0.0..=1.0.
    pub progress: f32,
    /// Latest human-readable active/terminal phase. Optional so persisted
    /// records written before phase tracking remain readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Present only while a limited job is waiting for its execution slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<JobQueueInfo>,
    /// Active child job this orchestrator is currently awaiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<JobDependencyInfo>,
    /// RFC3339 created/updated stamps.
    pub created_ts: String,
    pub updated_ts: String,
    /// Structured outcome on Done (job-kind-specific, e.g. {render_id, path}).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Actionable error on Failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CutError>,
    /// Last persistence failure, when the in-memory state could not be mirrored
    /// to `<project>/jobs/<job_id>.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence_error: Option<String>,
}

/// In-memory job table + persistence dir. Cloneable handle (Arc inside).
// Contract scaffold: create/progress/finish/fail are wired by job-spawning verbs
// (media.import chain, render.final) — dead-code warnings would be noise now.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct JobManager {
    inner: Arc<Mutex<JobManagerInner>>,
    /// Bus to publish job_progress / render_done events on.
    pub events: EventBus,
}

// Contract scaffold: fields consumed by job-spawning verbs.
#[allow(dead_code)]
#[derive(Debug)]
struct JobManagerInner {
    jobs: HashMap<String, JobRecord>,
    /// Active Tokio task handles plus blocking-worker lifetime for jobs started
    /// in this server run.
    tasks: HashMap<String, JobTaskControl>,
    /// Per-kind execution slots for heavy background work.
    limiters: HashMap<String, Arc<Semaphore>>,
    /// Bounded proof that an absent id belonged to a project which was fully
    /// detached. Late callbacks for these ids are expected and stay at debug
    /// level; every other unknown id remains a warning.
    detached_job_ids: HashSet<String>,
    detached_job_order: VecDeque<String>,
    /// Where job JSON is persisted; None until a project is open.
    persist_dir: Option<PathBuf>,
    /// Corrupt records moved out of active history during project recovery.
    persistence_notices: Vec<JobPersistenceNotice>,
    next_seq: u64,
}

// Contract scaffold: create/progress/finish/fail are driven by job-spawning verbs.
#[allow(dead_code)]
impl JobManager {
    pub fn new(events: EventBus) -> Self {
        Self {
            inner: Arc::new(Mutex::new(JobManagerInner {
                jobs: HashMap::new(),
                tasks: HashMap::new(),
                limiters: HashMap::new(),
                detached_job_ids: HashSet::new(),
                detached_job_order: VecDeque::new(),
                persist_dir: None,
                persistence_notices: Vec::new(),
                next_seq: 0,
            })),
            events,
        }
    }

    /// Stop every task owned by the current project and detach its persisted
    /// job table before another project is attached. Background work keeps
    /// absolute project paths, so letting it survive a project switch can write
    /// late results into a project the user has already closed (or deleted).
    pub async fn detach_project(&self) -> Result<(), CutError> {
        let (mut tasks, active_ids) = {
            let mut inner = self.inner.lock().expect("job lock");
            let tasks = inner.tasks.drain().collect::<Vec<_>>();
            let active_ids = inner
                .jobs
                .values()
                .filter(|record| matches!(record.state, JobState::Queued | JobState::Running))
                .map(|record| record.job_id.clone())
                .collect::<Vec<_>>();
            (tasks, active_ids)
        };

        for (_, task) in &tasks {
            task.request_cancel(JobCancellationReason::ProjectSwitch);
        }
        for job_id in &active_ids {
            self.cancel_for_project_switch(job_id);
        }

        // A Tokio abort cannot stop an already-running `spawn_blocking`
        // closure. Wait for every tracked worker to return so any synchronous
        // ffmpeg/Python child has been waited/reaped. If the grace period
        // expires, keep the current project attached and make the caller retry;
        // attaching the next project while an old worker is alive is forbidden.
        let deadline = tokio::time::Instant::now() + JOB_DRAIN_TIMEOUT;
        let mut pending = Vec::new();
        for (job_id, mut task) in tasks.drain(..) {
            if !task.wait_until(deadline).await {
                pending.push((job_id, task));
            }
        }
        if !pending.is_empty() {
            let pending_ids = pending
                .iter()
                .map(|(job_id, _)| job_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut inner = self.inner.lock().expect("job lock");
            inner.tasks.extend(pending);
            return Err(CutError::new(
                "job_cancel_pending",
                "project change is waiting for background work to stop",
                format!(
                    "workers still active after {} ms: {pending_ids}",
                    JOB_DRAIN_TIMEOUT.as_millis()
                ),
            )
            .with_suggested_action(
                "wait for the named worker to finish shutting down, then retry the project change",
            ));
        }

        let mut inner = self.inner.lock().expect("job lock");
        let detached_ids = inner.jobs.keys().cloned().collect::<Vec<_>>();
        for job_id in detached_ids {
            if inner.detached_job_ids.insert(job_id.clone()) {
                inner.detached_job_order.push_back(job_id);
            }
        }
        while inner.detached_job_order.len() > DETACHED_JOB_TOMBSTONE_LIMIT {
            if let Some(expired) = inner.detached_job_order.pop_front() {
                inner.detached_job_ids.remove(&expired);
            }
        }
        inner.jobs.clear();
        inner.tasks.clear();
        inner.limiters.clear();
        inner.persist_dir = None;
        inner.persistence_notices.clear();
        Ok(())
    }

    pub async fn switch_project(&self, project_dir: &std::path::Path) -> Result<(), CutError> {
        self.detach_project().await?;
        self.attach_project(project_dir)
    }

    /// Point persistence at `<project>/jobs/` when a project opens, and
    /// recover records from a previous run: anything persisted as
    /// Queued/Running records are recovered as Failed. Corrupt records move to
    /// quarantine and are disclosed through jobs.list instead of being skipped.
    pub fn attach_project(&self, project_dir: &std::path::Path) -> Result<(), CutError> {
        let recovered = recover(project_dir)?;
        let mut inner = self.inner.lock().expect("job lock");
        // Derive from both filenames and valid records before corrupt entries
        // are moved, so reopening never reuses a damaged history id.
        inner.next_seq = inner.next_seq.max(recovered.next_seq);
        inner.persistence_notices = recovered.notices;
        for rec in recovered.records {
            // Don't clobber records of THIS run; loaded history fills the gaps.
            inner.jobs.entry(rec.job_id.clone()).or_insert(rec);
        }
        inner.persist_dir = Some(recovered.dir);
        Ok(())
    }

    /// Create a job record in Queued state and return its id. The caller
    /// then spawns the tokio task and drives `progress`/`finish`/`fail`.
    pub fn create(&self, kind: &str) -> JobRecord {
        let mut inner = self.inner.lock().expect("job lock");
        inner.next_seq += 1;
        let now = cut_core::OpRecord::now_ts();
        let mut rec = JobRecord {
            job_id: format!("job_{:03}", inner.next_seq),
            kind: kind.to_string(),
            state: JobState::Queued,
            completion: None,
            outcome: None,
            outcome_reason: None,
            progress: 0.0,
            message: None,
            queue: None,
            waiting_on: None,
            created_ts: now.clone(),
            updated_ts: now,
            result: None,
            error: None,
            persistence_error: None,
        };
        if let Some(dir) = inner.persist_dir.clone() {
            if let Err(e) = persist(&dir.join(format!("{}.json", rec.job_id)), &rec) {
                tracing::error!(
                    job_id = %rec.job_id,
                    error = %e,
                    "failed to persist queued job"
                );
                rec.persistence_error = Some(e.to_string());
            }
        }
        inner.jobs.insert(rec.job_id.clone(), rec.clone());
        rec
    }

    /// Spawn and retain the task handle for a job created in this run. This keeps
    /// background work observable/cancellable instead of detaching every task.
    pub fn spawn<F>(&self, job_id: &str, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let old = self
            .inner
            .lock()
            .expect("job lock")
            .tasks
            .insert(job_id.to_string(), JobTaskControl::spawn(future));
        if let Some(old) = old {
            old.request_cancel(JobCancellationReason::Superseded);
        }
    }

    /// Run non-job async work behind the same keyed limiter used by
    /// `spawn_limited`. This lets synchronous verbs that spawn Python/ffmpeg
    /// share the pressure valve with background jobs.
    pub async fn with_limit<F, T>(&self, key: &str, max_running: usize, future: F) -> T
    where
        F: Future<Output = T>,
    {
        let limiter = self.limiter(key, max_running);
        let _permit = limiter.acquire_owned().await.expect("job limiter closed");
        future.await
    }

    fn limiter(&self, key: &str, max_running: usize) -> Arc<Semaphore> {
        let mut inner = self.inner.lock().expect("job lock");
        inner
            .limiters
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(max_running.max(1))))
            .clone()
    }

    /// Request cancellation for an active job from this run. The persisted
    /// record is terminal only after the task and every owned worker stopped.
    pub async fn abort(&self, job_id: &str) -> Result<bool, CutError> {
        let control = self.inner.lock().expect("job lock").tasks.remove(job_id);
        let Some(mut control) = control else {
            return Ok(false);
        };
        control.request_cancel(JobCancellationReason::CancelledByUser);
        // Await cancellation so task-owned subprocess and scratch guards have
        // dropped before jobs.cancel reports a terminal record to the caller.
        // An already-started blocking worker is counted separately and must
        // finish before cancellation can honestly report success.
        let deadline = tokio::time::Instant::now() + JOB_DRAIN_TIMEOUT;
        let drained = control.wait_until(deadline).await;
        // A task can cross its commit point and finish between removal of the
        // handle and cooperative cancellation. Never overwrite/report that completed
        // outcome as cancelled.
        if self
            .get(job_id)
            .is_some_and(|record| matches!(record.state, JobState::Done))
        {
            return Ok(false);
        }
        if !drained {
            self.inner
                .lock()
                .expect("job lock")
                .tasks
                .insert(job_id.to_string(), control);
            return Err(CutError::new(
                "job_cancel_pending",
                format!("job '{job_id}' is still stopping"),
                format!(
                    "a blocking worker did not finish within {} ms",
                    JOB_DRAIN_TIMEOUT.as_millis()
                ),
            )
            .with_suggested_action(
                "wait for the worker to finish shutting down, then retry jobs.cancel",
            ));
        }
        self.cancel_by_user(job_id);
        Ok(true)
    }

    /// Update progress (also flips Queued→Running) + publish job_progress.
    pub fn progress(&self, job_id: &str, progress: f32, message: Option<String>) {
        let persisted_message = message.clone();
        let kind = self.update(job_id, |r| {
            r.state = JobState::Running;
            r.queue = None;
            r.progress = progress.clamp(0.0, 1.0);
            if persisted_message.is_some() {
                r.message = persisted_message;
            }
        });
        if let Some(kind) = kind {
            self.events.publish(crate::events::Event::JobProgress {
                job_id: job_id.to_string(),
                kind,
                progress,
                message,
            });
        }
    }

    /// Fetch a record (jobs.status).
    pub fn get(&self, job_id: &str) -> Option<JobRecord> {
        self.inner
            .lock()
            .expect("job lock")
            .jobs
            .get(job_id)
            .cloned()
    }

    /// All records (debug surface / status bar).
    pub fn list(&self) -> Vec<JobRecord> {
        self.inner
            .lock()
            .expect("job lock")
            .jobs
            .values()
            .cloned()
            .collect()
    }

    /// Recovery-time corruption notices. Quarantine locations are project-
    /// relative, so API callers do not receive host filesystem paths.
    pub(crate) fn persistence_notices(&self) -> Vec<JobPersistenceNotice> {
        self.inner
            .lock()
            .expect("job lock")
            .persistence_notices
            .clone()
    }

    /// Shared mutate + persist + timestamp path. Returns the job kind so
    /// callers can publish typed events without re-locking.
    fn update(&self, job_id: &str, f: impl FnOnce(&mut JobRecord)) -> Option<String> {
        let mut inner = self.inner.lock().expect("job lock");
        let persist_dir = inner.persist_dir.clone();
        let Some(rec) = inner.jobs.get_mut(job_id) else {
            if inner.detached_job_ids.contains(job_id) {
                tracing::debug!(job_id, "ignored late update for detached job");
            } else {
                tracing::warn!(job_id, "ignored update for unknown job id");
            }
            return None;
        };
        if matches!(rec.state, JobState::Done | JobState::Failed) {
            return None;
        }
        f(rec);
        rec.updated_ts = cut_core::OpRecord::now_ts();
        let kind = rec.kind.clone();
        let terminal = matches!(rec.state, JobState::Done | JobState::Failed);
        // Persist every transition — crash-recoverable by design (server contract).
        if let Some(dir) = persist_dir {
            match persist(&dir.join(format!("{job_id}.json")), rec) {
                Ok(()) => rec.persistence_error = None,
                Err(e) => {
                    tracing::error!(
                        job_id,
                        error = %e,
                        "failed to persist job transition"
                    );
                    rec.persistence_error = Some(e.to_string());
                }
            }
        }
        if terminal {
            inner.tasks.remove(job_id);
        }
        Some(kind)
    }

    #[cfg(test)]
    fn active_task_count_for_tests(&self) -> usize {
        self.inner.lock().expect("job lock").tasks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        mgr.detach_project().await.unwrap();
        assert_eq!(mgr.active_task_count_for_tests(), 0);
        assert!(mgr.list().is_empty());
        assert!(
            mgr.inner
                .lock()
                .expect("job lock")
                .detached_job_ids
                .contains(&job.job_id),
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
                    .inner
                    .lock()
                    .expect("job lock")
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

        assert!(mgr.abort(&job.job_id).await.unwrap());
        assert_eq!(mgr.active_task_count_for_tests(), 0);
        let record = mgr.get(&job.job_id).unwrap();
        assert_eq!(record.state, JobState::Failed);
        assert_eq!(record.outcome, Some(JobOutcome::Cancelled));
        assert_eq!(record.outcome_reason, Some(JobOutcomeReason::UserCancelled));
    }

    #[tokio::test]
    async fn terminal_transitions_publish_progress_one() {
        for transition in ["success", "warnings", "failure"] {
            let bus = EventBus::new();
            let mut events = bus.subscribe();
            let mgr = JobManager::new(bus);
            let job = mgr.create("analysis");
            match transition {
                "success" => mgr.finish(&job.job_id, serde_json::json!({"ok": true})),
                "warnings" => mgr.finish_with_warnings(
                    &job.job_id,
                    serde_json::json!({"warnings": ["partial"]}),
                ),
                "failure" => mgr.fail(
                    &job.job_id,
                    CutError::new("failed", "analysis failed", "test"),
                ),
                _ => unreachable!(),
            }
            let event = events.recv().await.expect("terminal progress event");
            assert!(
                matches!(event, crate::events::Event::JobProgress { .. }),
                "expected job progress, got {event:?}"
            );
            if let crate::events::Event::JobProgress {
                job_id,
                kind,
                progress,
                message,
            } = event
            {
                assert_eq!(job_id, job.job_id);
                assert_eq!(kind, "analysis");
                assert_eq!(progress, 1.0);
                assert!(message.is_some());
            }
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
}
