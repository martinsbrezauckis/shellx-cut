//! `screen_record.polish` as visible, project-bound work.
//!
//! Baking the polished clip of a 4K take took about two minutes with nothing
//! in `jobs.list`, the status bar or the quit question, and the clip was added
//! to whichever project was open when the bake ended. The verb still answers
//! when the clip is placed, but:
//!
//! - the bake runs as a `screen_record_polish` job with real progress (frames
//!   composed out of frames planned), so the status bar, `jobs.list` and the
//!   quit question see it. The job stays running until the clip is placed, so
//!   a project change asks first (`jobs::consent`) instead of moving the clip;
//! - the clip belongs to the project that was open when the verb was called.
//!   If another project is open when it is ready (the change was forced with
//!   `cancel_running_jobs`), nothing is added anywhere: the recording stays in
//!   its own project, which offers it again (`screen_record.pending`), and the
//!   reply says where it is.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cut_core::{error_codes, CutError};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::jobs::{JobCancellationReason, JobManager};
use crate::state::AppState;

pub(crate) const KIND: &str = "screen_record_polish";
const LIMIT_KEY: &str = "screen_record.export";
const BAKE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MIN_REPORT_INTERVAL: Duration = Duration::from_millis(750);
/// The bake is this share of the job; placing the clip is the rest.
const BAKE_SHARE: f32 = 0.95;
/// Placing the clip waits for its imports (each up to about 18 s).
const PLACE_TIMEOUT: Duration = Duration::from_secs(120);

/// Throttled frame progress for the job record (every call writes the record).
struct FrameProgress {
    jobs: JobManager,
    job_id: String,
    last: Mutex<(Instant, u64)>,
}

impl FrameProgress {
    fn frame(&self, done: u64, total: u64) {
        let total = total.max(1);
        let percent = done.min(total) * 100 / total;
        let mut last = self.last.lock().expect("polish progress lock");
        if last.0.elapsed() < MIN_REPORT_INTERVAL && percent == last.1 {
            return;
        }
        *last = (Instant::now(), percent);
        drop(last);
        self.jobs.progress(
            &self.job_id,
            BAKE_SHARE * done.min(total) as f32 / total as f32,
            Some(format!("Polishing the recording: frame {done} of {total}")),
        );
    }
}

/// The work of one bake: `ProcessControl` stops its ffmpeg children when the
/// job is cancelled; the callback reports frames composed of frames planned.
pub(crate) type BakeWork = Box<
    dyn FnOnce(
            &record_render::ffmpeg::ProcessControl,
            &mut dyn FnMut(u64, u64),
        ) -> Result<(), CutError>
        + Send,
>;

/// What one polish bakes: the stream-copied capture (`raw`) or the polished
/// render of `source` with `plan`, into the cache file `baked`.
pub(crate) struct BakeInput {
    pub(crate) cached: bool,
    pub(crate) raw: bool,
    pub(crate) source: PathBuf,
    pub(crate) baked: PathBuf,
    pub(crate) audio: Option<PathBuf>,
    pub(crate) plan: Option<record_core::EditPlan>,
}

/// The bake of `input`; nothing to do when the cache already holds it.
pub(crate) fn bake_work(input: BakeInput) -> BakeWork {
    Box::new(move |control, on_frame| {
        if input.cached {
            return Ok(());
        }
        super::cache_output::bake_seed(&input.baked, |staged| match (&input.plan, input.raw) {
            (_, true) => {
                super::mux_raw_with_control(&input.source, input.audio.as_deref(), staged, control)
            }
            (Some(plan), false) => super::render_polish_with_control_progress(
                &input.source,
                plan,
                staged,
                input.audio.as_deref(),
                control,
                on_frame,
            )
            .map(|_| ()),
            (None, false) => Err(CutError::new(
                error_codes::INVALID_ARGS,
                "the polish has no admitted plan",
                "a normal polish needs its EditPlan",
            )),
        })
        .map(|_| ())
    })
}

/// A finished bake whose clip is being placed. Report the placement with
/// [`Polish::placed`]; dropping it unreported ends the job as failed.
pub(crate) struct Polish {
    pub(crate) job_id: String,
    placed: Option<oneshot::Sender<Result<Value, CutError>>>,
    ended: Option<oneshot::Receiver<()>>,
}

impl Polish {
    /// Report the placement and wait until the job record is final, so the
    /// verb never answers while its job still blocks a project change.
    pub(crate) async fn placed(mut self, outcome: Result<Value, CutError>) {
        if let Some(sender) = self.placed.take() {
            let _ = sender.send(outcome);
        }
        if let Some(ended) = self.ended.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), ended).await;
        }
    }
}

impl Drop for Polish {
    fn drop(&mut self) {
        if let Some(sender) = self.placed.take() {
            let _ = sender.send(Err(CutError::new(
                error_codes::JOB_FAILED,
                "the polished clip was not placed",
                "screen_record.polish ended before it placed the clip",
            )));
        }
    }
}

fn stopped_for_project_change(project: &str) -> CutError {
    CutError::new(
        "job_cancelled",
        format!(
            "the polished clip was not made: the project was changed while it was being made, so it was not added to another project. The recording stays in “{project}”"
        ),
        "a project change stopped the polish job (it was confirmed with cancel_running_jobs)",
    )
    .with_suggested_action(format!(
        "open “{project}” with project.open; it offers the recording again (screen_record.pending), add it with screen_record.recover"
    ))
}

/// Run the bake as a `screen_record_polish` job and wait for it. `project` is
/// the name of the project the clip belongs to, for the messages.
pub(crate) async fn bake(
    state: &AppState,
    project: &str,
    work: BakeWork,
) -> Result<Polish, CutError> {
    let jobs = state.jobs.clone();
    let job_id = jobs.create(KIND).job_id;
    let (baked_tx, baked_rx) = oneshot::channel::<Result<(), CutError>>();
    let (placed_tx, placed_rx) = oneshot::channel::<Result<Value, CutError>>();
    let (ended_tx, ended_rx) = oneshot::channel::<()>();
    let task_jobs = jobs.clone();
    let task_id = job_id.clone();
    let project_name = project.to_string();
    jobs.spawn(&job_id, async move {
        task_jobs.progress(&task_id, 0.0, Some("Preparing the polished clip…".into()));
        let progress = Arc::new(FrameProgress {
            jobs: task_jobs.clone(),
            job_id: task_id.clone(),
            last: Mutex::new((Instant::now() - MIN_REPORT_INTERVAL, 0)),
        });
        let baked = task_jobs
            .with_limit(
                LIMIT_KEY,
                1,
                crate::dispatch::run_blocking_cancellable("screen_record.polish", move |cancel| {
                    let child_cancel = cancel.clone();
                    let control =
                        record_render::ffmpeg::ProcessControl::bounded(BAKE_TIMEOUT, move || {
                            child_cancel.is_cancelled()
                        });
                    let result = work(&control, &mut |done, total| progress.frame(done, total));
                    match cancel.reason() {
                        Some(JobCancellationReason::ProjectSwitch) => {
                            Err(stopped_for_project_change(&project_name))
                        }
                        _ => result,
                    }
                }),
            )
            .await;
        let failure = baked.as_ref().err().cloned();
        let _ = baked_tx.send(baked);
        if let Some(error) = failure {
            task_jobs.fail(&task_id, error);
            return;
        }
        task_jobs.progress(
            &task_id,
            BAKE_SHARE,
            Some("Adding the polished clip to the timeline…".into()),
        );
        // Stay running until the clip is placed: a project change asks first,
        // and one forced with cancel_running_jobs waits (cooperatively, not
        // aborted) for the placement in this project to end before it
        // attaches the next project, so the clip never lands in that one.
        let _cooperative = crate::jobs::hold_cooperative_cancellation();
        match tokio::time::timeout(PLACE_TIMEOUT, placed_rx).await {
            Ok(Ok(Ok(result))) => task_jobs.finish(&task_id, result),
            Ok(Ok(Err(error))) => task_jobs.fail(&task_id, error),
            Ok(Err(_)) | Err(_) => task_jobs.fail(
                &task_id,
                CutError::new(
                    error_codes::JOB_FAILED,
                    "the polished clip was not placed",
                    "screen_record.polish did not report its placement",
                ),
            ),
        }
        let _ = ended_tx.send(());
    });
    match baked_rx.await {
        Ok(Ok(())) => Ok(Polish {
            job_id,
            placed: Some(placed_tx),
            ended: Some(ended_rx),
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(stopped_for_project_change(project)),
    }
}

/// The open project's folder, canonical, or `None` when no project is open.
pub(crate) async fn open_project_dir(state: &AppState) -> Option<PathBuf> {
    let guard = state.project.read().await;
    let dir = guard.as_ref()?.dir.clone();
    drop(guard);
    Some(dir.canonicalize().unwrap_or(dir))
}

/// Whether `bound` (the project the polish was asked in) is still open.
pub(crate) async fn still_open(state: &AppState, bound: &Path) -> bool {
    let bound = bound.canonicalize().unwrap_or_else(|_| bound.to_path_buf());
    open_project_dir(state)
        .await
        .is_some_and(|open| open == bound)
}

/// When the project the polish was asked in (`bound`) is no longer open, the
/// reply that says where the recording waits; `None` when it is still open.
pub(crate) async fn waiting_elsewhere(
    state: &AppState,
    bound: &Path,
    source: &Path,
    baked: &str,
) -> Option<Value> {
    if still_open(state, bound).await {
        return None;
    }
    let capture = super::pending_recordings::capture_id_of_source(source);
    Some(waiting_reply(capture.as_deref(), bound, baked))
}

/// The reply when the clip's project is no longer open: nothing was added,
/// the recording is offered again in its own project.
pub(crate) fn waiting_reply(capture_id: Option<&str>, project_dir: &Path, baked: &str) -> Value {
    let name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".cutproj").to_string())
        .unwrap_or_else(|| project_dir.display().to_string());
    if let Some(capture_id) = capture_id {
        // Offer it again when its project opens (or at the next start).
        super::pending_recordings::note_started(capture_id, project_dir);
    }
    json!({
        "placed": false,
        "clip_id": Value::Null,
        "baked": baked,
        "waiting_in_project": {
            "name": name,
            "path": project_dir.display().to_string(),
        },
        "message": format!(
            "The recording was polished for “{name}”, which is no longer open, so it was not added to the project open now. It waits in “{name}”: open it to add the recording to its timeline."
        ),
    })
}

#[cfg(test)]
#[path = "polish_job_tests.rs"]
mod tests;
