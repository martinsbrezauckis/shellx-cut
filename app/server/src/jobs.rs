//! jobs.rs — background job system (server contract "Job system").
//!
//! Long tasks publish progress and persist `<proj>/jobs/<id>.json`, keeping status
//! observable across crashes. Callers are dispatch plus jobs.status/list/cancel.

mod consent;
mod dependency;
mod durable_admission;
pub(crate) mod interrupted;
mod locking;
#[cfg(test)]
mod locking_tests;
mod outcome;
mod persistence;
mod process;
mod queue;
mod render_queue_cancel;
mod retry;
mod runtime;

use crate::events::EventBus;
use cut_core::CutError;
pub use dependency::JobDependencyInfo;
pub(crate) use outcome::{JobOutcome, JobOutcomeReason};
use persistence::{persist, recover, JobPersistenceNotice};
pub(crate) use process::{run_owned, spawn_retrying_busy, ProcessControl, ProcessTermination};
pub use queue::JobQueueInfo;
pub use retry::{
    JobInputFingerprint, JobRetry, JobRetryDescriptor, ScreenRecordExportRetryDescriptor,
    ScreenRecordExportRetryFormat, VerifyRerunRetryDescriptor,
};
use runtime::JobTaskControl;
pub(crate) use runtime::{
    begin_current_blocking_worker, begin_current_process_worker, current_job_cancellation,
    hold_cooperative_cancellation, JobCancellation, JobCancellationReason,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// How long a project change waits for the old project's workers to stop. A stop
/// measured on an idle Linux box takes under 0.15 s; a busy Windows machine
/// (virus scanner, many ffmpeg children) was seen to need several seconds, so
/// the wait is generous: waiting longer costs a pause, refusing costs work.
#[cfg(not(test))]
const JOB_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
#[cfg(test)]
const JOB_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
const DETACHED_JOB_TOMBSTONE_LIMIT: usize = 1024;

/// One running or queued job a project change would cancel.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActiveJobInfo {
    pub job_id: String,
    pub kind: String,
    pub progress: f32,
    /// Latest phase text, when the job published one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ActiveJobInfo {
    /// Work someone asked for that a project change would throw away: the
    /// change asks first (item 135, see `consent`).
    pub fn is_delivery(&self) -> bool {
        consent::asks_before_cancel(&self.kind)
    }
}

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
    /// Job kind: "probe" | "proxy" | "proxy_check" | "transcribe" | "perception"
    /// | "render" | "judge" | "import_chain" (and more; see ui jobPresentation).
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
    /// Durable retry eligibility, lineage, and only the validated descriptor
    /// owned by the job kind. Legacy records have no retry projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<JobRetry>,
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
#[derive(Debug, Clone)]
pub struct JobManager {
    inner: Arc<Mutex<JobManagerInner>>,
    /// Bus to publish job_progress / render_done events on.
    pub events: EventBus,
    doctor: Option<crate::doctor::cache::DoctorCache>,
}

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
    /// How long a project change or a cancellation waits for workers to stop.
    drain_timeout: std::time::Duration,
}

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
                drain_timeout: JOB_DRAIN_TIMEOUT,
            })),
            events,
            doctor: None,
        }
    }

    pub(crate) fn with_doctor(mut self, doctor: crate::doctor::cache::DoctorCache) -> Self {
        self.doctor = Some(doctor);
        self
    }

    /// Stop every task owned by the current project and detach its persisted
    /// job table before another project is attached. Background work keeps
    /// absolute project paths, so letting it survive a project switch can write
    /// late results into a project the user has already closed (or deleted).
    fn drain_timeout(&self) -> std::time::Duration {
        self.lock_inner().drain_timeout
    }

    /// Tests that exercise ordering around a project change, not the drain
    /// budget itself, widen the wait so a slow machine cannot turn a worker
    /// that is still stopping into a refused change.
    #[cfg(test)]
    pub(crate) fn set_drain_timeout_for_test(&self, timeout: std::time::Duration) {
        self.lock_inner().drain_timeout = timeout;
    }

    /// Queued and running jobs of the open project: what closing or replacing it
    /// would cancel. Read-only; cancels nothing.
    pub fn active_jobs(&self) -> Vec<ActiveJobInfo> {
        let mut jobs: Vec<ActiveJobInfo> = self
            .lock_inner()
            .jobs
            .values()
            .filter(|record| matches!(record.state, JobState::Queued | JobState::Running))
            .map(|record| ActiveJobInfo {
                job_id: record.job_id.clone(),
                kind: record.kind.clone(),
                progress: record.progress,
                message: record.message.clone(),
            })
            .collect();
        jobs.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        jobs
    }

    pub async fn detach_project(&self) -> Result<(), CutError> {
        let (mut tasks, active_ids) = {
            let mut inner = self.lock_inner();
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
        let drain_timeout = self.drain_timeout();
        let deadline = tokio::time::Instant::now() + drain_timeout;
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
            let mut inner = self.lock_inner();
            inner.tasks.extend(pending);
            return Err(CutError::new(
                "job_cancel_pending",
                "project change is waiting for background work to stop",
                format!(
                    "workers still active after {} ms: {pending_ids}",
                    drain_timeout.as_millis()
                ),
            )
            .with_suggested_action(
                "the old project's running jobs were already cancelled and are stopping; wait a few seconds, then retry the project change (it completes once they have stopped)",
            ));
        }

        let mut inner = self.lock_inner();
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
        let mut inner = self.lock_inner();
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
        self.create_with_retry(kind, None)
    }

    /// Spawn and retain the task handle for a job created in this run. This keeps
    /// background work observable/cancellable instead of detaching every task.
    pub fn spawn<F>(&self, job_id: &str, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let old = self
            .lock_inner()
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
        let mut inner = self.lock_inner();
        inner
            .limiters
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(max_running.max(1))))
            .clone()
    }

    /// Request cancellation for an active job from this run. The persisted
    /// record is terminal only after the task and every owned worker stopped.
    pub async fn abort(&self, job_id: &str) -> Result<bool, CutError> {
        let control = self.lock_inner().tasks.remove(job_id);
        let Some(mut control) = control else {
            return Ok(false);
        };
        control.request_cancel(JobCancellationReason::CancelledByUser);
        // Await cancellation so task-owned subprocess and scratch guards have
        // dropped before jobs.cancel reports a terminal record to the caller.
        // An already-started blocking worker is counted separately and must
        // finish before cancellation can honestly report success.
        let drain_timeout = self.drain_timeout();
        let deadline = tokio::time::Instant::now() + drain_timeout;
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
            self.lock_inner().tasks.insert(job_id.to_string(), control);
            return Err(self.cancel_pending_error(job_id, drain_timeout));
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
                terminal: false,
            });
        }
    }

    /// Fetch a record (jobs.status).
    pub fn get(&self, job_id: &str) -> Option<JobRecord> {
        self.lock_inner().jobs.get(job_id).cloned()
    }

    /// All records (debug surface / status bar).
    pub fn list(&self) -> Vec<JobRecord> {
        self.lock_inner().jobs.values().cloned().collect()
    }

    /// Recovery-time corruption notices. Quarantine locations are project-
    /// relative, so API callers do not receive host filesystem paths.
    pub(crate) fn persistence_notices(&self) -> Vec<JobPersistenceNotice> {
        self.lock_inner().persistence_notices.clone()
    }

    /// Shared mutate + persist + timestamp path. Returns the job kind so
    /// callers can publish typed events without re-locking.
    fn update(&self, job_id: &str, f: impl FnOnce(&mut JobRecord)) -> Option<String> {
        let mut inner = self.lock_inner();
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
        // Persist every transition crash-recoverably, clearing any prior notice before serializing.
        rec.persistence_error = None;
        if let Some(dir) = persist_dir.as_ref() {
            match persist(&dir.join(format!("{job_id}.json")), rec) {
                Ok(()) => {}
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
            if let Some(project_dir) = persist_dir.as_deref().and_then(std::path::Path::parent) {
                interrupted::note_ended(project_dir, job_id);
            }
        }
        Some(kind)
    }

    #[cfg(test)]
    fn active_task_count_for_tests(&self) -> usize {
        self.lock_inner().tasks.len()
    }

    #[cfg(test)]
    pub(crate) fn has_active_task_for_tests(&self, job_id: &str) -> bool {
        self.lock_inner().tasks.contains_key(job_id)
    }
}
#[cfg(test)]
mod tests;
