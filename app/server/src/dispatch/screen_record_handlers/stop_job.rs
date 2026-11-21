//! Keep a stopped Windows take owned while its files finish saving.

use super::*;

#[cfg(windows)]
use std::collections::{HashMap, VecDeque};
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
mod eligibility;
#[cfg(windows)]
use eligibility::{has_first_screen_frame, is_ordinary_marker};

#[cfg(windows)]
type CaptureKey = (usize, PathBuf, String);

#[cfg(windows)]
#[derive(Clone, PartialEq, Eq)]
struct StopOptions {
    autoedit: bool,
    mux_raw: bool,
    raw_path: Option<String>,
}

#[cfg(windows)]
impl From<&super::StopArgs> for StopOptions {
    fn from(args: &super::StopArgs) -> Self {
        Self {
            autoedit: args.autoedit,
            mux_raw: args.mux_raw,
            raw_path: args.raw_path.clone(),
        }
    }
}
#[cfg(windows)]
#[derive(Clone)]
struct StopJobEntry {
    job_id: String,
    options: StopOptions,
    finished: bool,
}

#[cfg(windows)]
#[derive(Default)]
struct StopJobs {
    entries: HashMap<CaptureKey, StopJobEntry>,
    completed: VecDeque<CaptureKey>,
}

// Retain bounded completed replies; changed output options use ordinary Stop.
#[cfg(windows)]
fn pending() -> &'static Mutex<StopJobs> {
    static PENDING: OnceLock<Mutex<StopJobs>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(StopJobs::default()))
}

#[cfg(windows)]
fn lock_pending() -> std::sync::MutexGuard<'static, StopJobs> {
    pending()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(windows)]
fn mark_finished(key: &CaptureKey) {
    let mut jobs = lock_pending();
    if let Some(entry) = jobs.entries.get_mut(key) {
        entry.finished = true;
        jobs.completed.push_back(key.clone());
    }
    while jobs.completed.len() > 32 {
        if let Some(oldest) = jobs.completed.pop_front() {
            if jobs
                .entries
                .get(&oldest)
                .is_some_and(|entry| entry.finished)
            {
                jobs.entries.remove(&oldest);
            }
        }
    }
}

#[cfg(windows)]
async fn existing_stop(
    state: &AppState,
    key: &CaptureKey,
    options: &StopOptions,
    app_exit: bool,
) -> Option<Result<VerbResult, CutError>> {
    let existing = lock_pending().entries.get(key).cloned()?;
    if existing.options != *options {
        if app_exit && !existing.finished {
            return Some(wait_for_saved_job(state, &existing.job_id).await);
        }
        return (!existing.finished).then(|| {
            Err(CutError::new(
                error_codes::CONFLICT,
                "the recording is already saving",
                "this take was stopped with different save options",
            )
            .with_suggested_action(
                "wait for the save job to finish before asking for another output",
            ))
        });
    }
    if app_exit || existing.finished {
        Some(wait_for_saved_job(state, &existing.job_id).await)
    } else {
        Some(Ok(pending_reply(&key.2, &existing.job_id)))
    }
}

#[cfg(windows)]
fn pending_reply(capture_id: &str, job_id: &str) -> VerbResult {
    VerbResult::ok(json!({
        "capture_id": capture_id,
        "status": "finalizing",
        "job_id": job_id,
    }))
}

#[cfg(windows)]
async fn settle_stop_job(
    state: &AppState,
    job_id: &str,
    worker: tokio::task::JoinHandle<Result<VerbResult, CutError>>,
) -> Result<VerbResult, CutError> {
    let result = match worker.await {
        Ok(result) => result,
        Err(_) => Err(CutError::new(
            error_codes::JOB_FAILED,
            "recording save stopped unexpectedly",
            "the save worker ended before it reported a finished recording",
        )
        .with_suggested_action("reopen the project and recover the saved recording parts")),
    };
    let result = match result {
        Ok(reply) if reply.ok => match serde_json::to_value(&reply) {
            Ok(envelope) => {
                state.jobs.finish(job_id, envelope);
                Ok(reply)
            }
            Err(error) => {
                let error = CutError::new(
                    error_codes::JOB_FAILED,
                    "recording save result could not be reported",
                    format!("the finished Stop response could not be stored: {error}"),
                );
                state.jobs.fail(job_id, error.clone());
                Err(error)
            }
        },
        Ok(reply) => {
            let error = reply.error.clone().unwrap_or_else(|| {
                CutError::new(
                    error_codes::JOB_FAILED,
                    "recording save failed",
                    "the Stop pipeline returned an unsuccessful result",
                )
            });
            state.jobs.fail(job_id, error);
            Ok(reply)
        }
        Err(error) => {
            state.jobs.fail(job_id, error.clone());
            Err(error)
        }
    };
    result
}

#[cfg(windows)]
async fn wait_for_saved_job(state: &AppState, job_id: &str) -> Result<VerbResult, CutError> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15 * 60);
    loop {
        let record = state.jobs.get(job_id).ok_or_else(|| {
            CutError::new(
                error_codes::JOB_FAILED,
                "recording save job disappeared",
                "the running save job no longer has a status record",
            )
        })?;
        match record.state {
            crate::jobs::JobState::Done => {
                return serde_json::from_value(record.result.ok_or_else(|| {
                    CutError::new(
                        error_codes::JOB_FAILED,
                        "recording save result is missing",
                        "the completed save job has no Stop response",
                    )
                })?)
                .map_err(|error| {
                    CutError::new(
                        error_codes::JOB_FAILED,
                        "recording save result is invalid",
                        format!("the saved Stop response could not be read: {error}"),
                    )
                });
            }
            crate::jobs::JobState::Failed => {
                return Err(record.error.unwrap_or_else(|| {
                    CutError::new(
                        error_codes::JOB_FAILED,
                        "recording save failed",
                        "the save job ended without an error detail",
                    )
                }));
            }
            _ if tokio::time::Instant::now() >= deadline => {
                return Err(CutError::new(
                    error_codes::JOB_FAILED,
                    "recording is still saving",
                    "the save job did not finish within fifteen minutes",
                )
                .with_suggested_action("keep the app open and inspect jobs.status"));
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
}

/// Windows ordinary takes answer promptly once Stop is signalled. The existing
/// complete Stop pipeline still owns the take and reports its exact envelope
/// in a persistent job when the recording and optional raw file are durable.
pub(in crate::dispatch) async fn screen_record_stop(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[cfg(not(windows))]
    {
        return super::screen_record_stop_complete(state, args, actor).await;
    }
    #[cfg(windows)]
    {
        let key_args: super::StopArgs = parse_args(args.clone())?;
        let app_exit = super::stop_outcome::app_exit(key_args.reason.as_deref())?;
        let options = StopOptions::from(&key_args);
        crate::screen_record::recovery::validate_capture_id(&key_args.capture_id)?;
        let (_, _, project_dir, _) = snapshot(state).await?;
        let key = (
            std::sync::Arc::as_ptr(&state.project_transition) as usize,
            project_dir.clone(),
            key_args.capture_id.clone(),
        );
        if let Some(result) = existing_stop(state, &key, &options, app_exit).await {
            return result;
        }
        if app_exit {
            return super::screen_record_stop_complete(state, args, actor).await;
        }
        if !crate::screen_record::is_capture_active(&key_args.capture_id)
            || !is_ordinary_marker(&project_dir, &key_args.capture_id)
            || !has_first_screen_frame(&key_args.capture_id).await
        {
            return super::screen_record_stop_complete(state, args, actor).await;
        }

        // The owned guard prevents project replacement through finalization.
        let admission_deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(800);
        let transition = loop {
            if let Ok(guard) = state.project_transition.clone().try_lock_owned() {
                break guard;
            }
            if let Some(result) = existing_stop(state, &key, &options, false).await {
                return result;
            }
            if tokio::time::Instant::now() >= admission_deadline {
                return Err(CutError::new(
                    error_codes::CONFLICT,
                    "the project is busy",
                    "the recording could not reserve its save boundary while the project was changing",
                )
                .with_suggested_action("try Stop again after the project finishes changing"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let (_, _, locked_dir, _) = snapshot(state).await?;
        if locked_dir != project_dir
            || !crate::screen_record::is_capture_active(&key_args.capture_id)
        {
            drop(transition);
            return super::screen_record_stop_complete(state, args, actor).await;
        }
        if let Some(result) = existing_stop(state, &key, &options, false).await {
            return result;
        }
        let Some(_) =
            crate::screen_record::existing_capture_dir(&project_dir, &key_args.capture_id)?
        else {
            drop(transition);
            return super::screen_record_stop_complete(state, args, actor).await;
        };

        let job = state.jobs.create("screen_record_stop");
        let job_id = job.job_id.clone();
        lock_pending().entries.insert(
            key.clone(),
            StopJobEntry {
                job_id: job_id.clone(),
                options,
                finished: false,
            },
        );
        let _ = crate::screen_record::stop_capture(&key_args.capture_id);
        let state = state.clone();
        let capture_id = key_args.capture_id;
        let final_job_id = job_id.clone();
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();

        // The outer watcher keeps project ownership through its job update.
        tokio::spawn(async move {
            let _transition = transition;
            state
                .jobs
                .progress(&final_job_id, 0.0, Some("Saving the recording".into()));
            let inner_state = state.clone();
            let worker = tokio::spawn(async move {
                super::screen_record_stop_complete(&inner_state, args, actor).await
            });
            let result = settle_stop_job(&state, &final_job_id, worker).await;
            mark_finished(&key);
            let _ = completed_tx.send(result);
        });

        match tokio::time::timeout(std::time::Duration::from_millis(100), completed_rx).await {
            Ok(Ok(result)) => result,
            _ => Ok(pending_reply(&capture_id, &job_id)),
        }
    }
}
#[cfg(all(test, windows))]
mod tests;
