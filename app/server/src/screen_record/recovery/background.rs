//! Preparing interrupted recordings is a visible background job.
//!
//! Recovering a long interrupted take reads, checks and stitches every saved
//! part: minutes for a long 4K recording. `project.open` and
//! `screen_record.start` used to do that work before they answered, holding
//! every other project verb (a 4-minute capture blocked opening for about
//! 4 minutes with no progress). They now start this job and answer at once;
//! the job shows in the status bar as "Preparing interrupted recording", and
//! the take is offered in Recording Recovery when it is ready. One job runs per
//! project at a time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::json;

use crate::state::AppState;
use cut_core::{error_codes, CutError};
use record_recovery::{is_plain_dir, read_manifest, MANIFEST_FILE};

pub(crate) const JOB_KIND: &str = "recording_recovery";
const MAX_CHECKED_CAPTURES: usize = 512;

fn running() -> &'static Mutex<BTreeMap<PathBuf, String>> {
    static RUNNING: OnceLock<Mutex<BTreeMap<PathBuf, String>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn key(project_dir: &Path) -> PathBuf {
    project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf())
}

/// The preparation job running for this project, if any.
pub(crate) fn running_job(project_dir: &Path) -> Option<String> {
    running()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key(project_dir))
        .cloned()
}

/// Refuse to touch a capture while its project's preparation job runs: two
/// owners must never recover the same capture at once.
pub(crate) fn refuse_while_preparing(project_dir: &Path) -> Result<(), CutError> {
    match running_job(project_dir) {
        None => Ok(()),
        Some(job_id) => Err(CutError::new(
            error_codes::CONFLICT,
            "this recording is still being prepared",
            format!(
                "job {job_id} is checking and joining the saved parts of interrupted recordings"
            ),
        )
        .with_suggested_action(
            "wait until the job in the status bar finishes, then add the recording again",
        )),
    }
}

/// Whether any capture of the project still has no receipt (cheap: reads the
/// small journals only, never media).
fn needs_work(cache: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return false;
    };
    entries.flatten().take(MAX_CHECKED_CAPTURES).any(|entry| {
        let root = entry.path();
        is_plain_dir(&root).unwrap_or(false)
            && root.join(MANIFEST_FILE).exists()
            && read_manifest(&root).map_or(true, |manifest| manifest.receipt.is_none())
    })
}

/// Start (or find) the project's preparation job. `None` when nothing needs
/// preparing. Never waits for the work itself.
pub(crate) fn start(state: &AppState, project_dir: &Path) -> Result<Option<String>, CutError> {
    let Some(cache) = crate::screen_record::containment::existing_cache_dir(project_dir)? else {
        return Ok(None);
    };
    let project_key = key(project_dir);
    let mut jobs_running = running()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(job_id) = jobs_running.get(&project_key) {
        return Ok(Some(job_id.clone()));
    }
    if !needs_work(&cache) {
        return Ok(None);
    }
    let job = state.jobs.create(JOB_KIND);
    let job_id = job.job_id.clone();
    jobs_running.insert(project_key.clone(), job_id.clone());
    drop(jobs_running);
    let jobs = state.jobs.clone();
    let task_job = job_id.clone();
    state.jobs.spawn(&job_id, async move {
        // Leaves the running set however the task ends (finished, failed,
        // cancelled with its project).
        struct Release(PathBuf);
        impl Drop for Release {
            fn drop(&mut self) {
                running()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&self.0);
            }
        }
        let release = Release(project_key);
        jobs.progress(
            &task_job,
            0.05,
            Some("Preparing an interrupted recording so it can be added to the timeline".into()),
        );
        let ffmpeg = cut_media::toolpath::ffmpeg();
        let ffprobe = cut_media::toolpath::ffprobe();
        let scan = tokio::task::spawn_blocking(move || {
            super::scan(
                &cache,
                &ffmpeg.to_string_lossy(),
                &ffprobe.to_string_lossy(),
            )
        })
        .await;
        drop(release);
        match scan {
            Ok(scan) => {
                let result = json!({
                    "recovered": scan.recovered,
                    "deferred": scan.deferred,
                    "failed_closed": scan.failed_closed,
                });
                if scan.failed_closed.is_empty() {
                    jobs.finish(&task_job, result);
                } else {
                    jobs.finish_with_problems(&task_job, result);
                }
            }
            Err(error) => jobs.fail(
                &task_job,
                CutError::new(
                    error_codes::JOB_FAILED,
                    "interrupted recordings could not be prepared",
                    error.to_string(),
                )
                .with_suggested_action("open the project again to retry"),
            ),
        }
    });
    Ok(Some(job_id))
}

/// The `recovery_scan` field of project.open and screen_record.start: the work
/// now runs in `job_id`; its result lists what was recovered.
pub(crate) fn started_report(job_id: Option<String>) -> serde_json::Value {
    json!({
        "recovered": [],
        "deferred": [],
        "failed_closed": [],
        "job_id": job_id,
        "state": if job_id.is_some() { "running" } else { "none" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_capture_without_a_receipt_needs_preparing() {
        let cache = tempfile::tempdir().unwrap();
        assert!(!needs_work(cache.path()));
        let capture = cache.path().join("cap_1");
        std::fs::create_dir(&capture).unwrap();
        assert!(
            !needs_work(cache.path()),
            "a folder without a journal is not a capture"
        );
        record_recovery::ManifestOwner::begin(
            &capture,
            record_recovery::CaptureStart::new("cap_1", 15_000),
        )
        .unwrap();
        assert!(needs_work(cache.path()));
    }

    #[test]
    fn a_running_preparation_refuses_a_second_owner_by_name() {
        let project = tempfile::tempdir().unwrap();
        assert!(refuse_while_preparing(project.path()).is_ok());
        running()
            .lock()
            .unwrap()
            .insert(key(project.path()), "job_042".into());
        let refusal = refuse_while_preparing(project.path()).unwrap_err();
        assert_eq!(refusal.code, error_codes::CONFLICT);
        assert!(refusal.cause.contains("job_042"));
        running().lock().unwrap().remove(&key(project.path()));
    }
}
