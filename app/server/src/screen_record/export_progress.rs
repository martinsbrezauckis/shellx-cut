//! Observable phases for a bounded screen-record export.
//!
//! The recorder compositor writes raw frames to ffmpeg rather than exposing
//! ffmpeg's generic console progress.  Translate confirmed frame flow into the
//! shared job contract so the Record panel and `jobs.status` show a useful
//! phase, percentage, and `updated_ts` for the last observed progress.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::jobs::JobManager;

const PREPARE_PROGRESS: f32 = 0.02;
const RENDER_PROGRESS_START: f32 = 0.08;
const RENDER_PROGRESS_END: f32 = 0.92;
const MIN_REPORT_INTERVAL: Duration = Duration::from_millis(750);

#[derive(Clone)]
pub(super) struct ExportProgressReporter {
    jobs: JobManager,
    job_id: String,
    format: &'static str,
    last: Arc<Mutex<RenderProgressMark>>,
}

struct RenderProgressMark {
    frames: u64,
    reported_at: Instant,
    completion_reported: bool,
}

impl ExportProgressReporter {
    pub(super) fn new(jobs: JobManager, job_id: String, format: &'static str) -> Self {
        Self {
            jobs,
            job_id,
            format,
            last: Arc::new(Mutex::new(RenderProgressMark {
                frames: 0,
                // The first observed frame is always reportable.
                reported_at: Instant::now() - MIN_REPORT_INTERVAL,
                completion_reported: false,
            })),
        }
    }

    pub(super) fn preparing(&self) {
        self.report(
            PREPARE_PROGRESS,
            format!("Preparing {} recorder export…", self.format.to_uppercase()),
        );
    }

    pub(super) fn preparing_audio(&self) {
        self.report(0.05, "Preparing recorder audio for ffmpeg…".to_string());
    }

    pub(super) fn rendering_started(&self) {
        self.report(
            RENDER_PROGRESS_START,
            format!(
                "Rendering {} recording with ffmpeg… awaiting frames",
                self.format.to_uppercase(),
            ),
        );
    }

    pub(super) fn rendering(&self, frames: u64, expected_frames: u64) {
        let mut mark = self.last.lock().expect("record export progress lock");
        if mark.completion_reported {
            return;
        }
        let interval_elapsed = mark.reported_at.elapsed() >= MIN_REPORT_INTERVAL;
        let total = expected_frames.max(1);
        let percent_changed = frames.min(total).saturating_mul(100) / total
            > mark.frames.min(total).saturating_mul(100) / total;
        if !interval_elapsed && !percent_changed {
            return;
        }
        mark.frames = frames;
        mark.reported_at = Instant::now();
        mark.completion_reported = frames >= total;
        drop(mark);

        let percent = (frames.saturating_mul(100) / total).min(99);
        self.report(
            render_progress(frames, total),
            format!(
                "Rendering {} recording with ffmpeg… {percent}% ({frames}/{total} frames)",
                self.format.to_uppercase(),
            ),
        );
    }

    pub(super) fn finalizing(&self) {
        self.report(
            RENDER_PROGRESS_END,
            format!("Finalizing {} recorder export…", self.format.to_uppercase()),
        );
    }

    fn report(&self, progress: f32, message: String) {
        self.jobs.progress(&self.job_id, progress, Some(message));
    }
}

fn render_progress(frames: u64, expected_frames: u64) -> f32 {
    let complete = (frames as f64 / expected_frames.max(1) as f64).clamp(0.0, 1.0) as f32;
    RENDER_PROGRESS_START + (RENDER_PROGRESS_END - RENDER_PROGRESS_START) * complete
}

#[cfg(test)]
mod tests {
    use super::{render_progress, ExportProgressReporter, PREPARE_PROGRESS, RENDER_PROGRESS_END};
    use crate::state::AppState;

    #[test]
    fn progress_stays_bounded_and_reaches_the_finalizing_boundary() {
        assert_eq!(render_progress(0, 120), 0.08);
        assert!((render_progress(60, 120) - 0.5).abs() < f32::EPSILON);
        assert!((render_progress(120, 120) - RENDER_PROGRESS_END).abs() < f32::EPSILON);
        assert!((render_progress(999, 120) - RENDER_PROGRESS_END).abs() < f32::EPSILON);
    }

    #[test]
    fn frame_progress_updates_the_job_phase_and_last_progress_timestamp() {
        let state = AppState::new();
        let job = state.jobs.create("screen_record_export");
        let reporter = ExportProgressReporter::new(state.jobs.clone(), job.job_id.clone(), "mp4");

        reporter.preparing();
        let prepared = state.jobs.get(&job.job_id).expect("prepared job");
        assert_eq!(prepared.progress, PREPARE_PROGRESS);
        assert_eq!(
            prepared.message.as_deref(),
            Some("Preparing MP4 recorder export…")
        );
        let prepared_at = prepared.updated_ts;

        reporter.rendering_started();
        let awaiting = state.jobs.get(&job.job_id).expect("awaiting frame job");
        assert_eq!(awaiting.progress, 0.08);
        assert_eq!(
            awaiting.message.as_deref(),
            Some("Rendering MP4 recording with ffmpeg… awaiting frames")
        );

        reporter.rendering(60, 120);
        let rendering = state.jobs.get(&job.job_id).expect("rendering job");
        assert!(rendering.progress > PREPARE_PROGRESS);
        assert!(rendering
            .message
            .as_deref()
            .is_some_and(|message| message.contains("50% (60/120 frames)")));
        assert!(rendering.updated_ts >= prepared_at);
    }

    #[test]
    fn frame_progress_stays_bounded_after_the_expected_frame_count() {
        let state = AppState::new();
        let mut events = state.events.subscribe();
        let job = state.jobs.create("screen_record_export");
        let reporter = ExportProgressReporter::new(state.jobs.clone(), job.job_id.clone(), "mp4");

        for frames in 1..=128 {
            reporter.rendering(frames, 1);
        }

        let reports = std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event, crate::events::Event::JobProgress { job_id, .. } if job_id == &job.job_id))
            .count();
        assert_eq!(reports, 1, "only the completion-boundary report is allowed");
    }
}
