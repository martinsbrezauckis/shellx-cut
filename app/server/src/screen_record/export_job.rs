//! Asynchronous, fenced recording exports.
//!
//! The request captures its output authorization before returning a job id. The
//! moved `OutputPath` lease survives a UI output-directory reset, then drops on
//! every terminal path only after its cancellable ffmpeg worker has reaped.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::dispatch::{parse_args, run_blocking_cancellable, snapshot};
use crate::output_paths::{fence_output_path, resolve_existing_project_file, OutputPathPolicy};
use crate::screen_record::export_progress::ExportProgressReporter;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};

const EXPORT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const EXPORT_MAX_RUNNING: usize = 1;
const EXPORT_LIMIT_KEY: &str = "screen_record.export";

mod retry;
mod retry_staging;
pub(crate) use retry::retry_screen_record_export;

#[derive(Clone, Debug)]
enum ExportFormat {
    Mp4,
    Gif { fps: u32, width: u32 },
}

struct PreparedExport {
    source: PathBuf,
    plan: PathBuf,
    capture_audio: crate::screen_record::CaptureExportAudio,
    out: crate::output_paths::OutputPath,
    format: ExportFormat,
    output_path: String,
    // Retry-only: owned copies of validated inputs. This remains held while a
    // queued worker waits for its limiter and through every renderer reopen.
    _retry_input_staging: Option<retry_staging::RetryInputStaging>,
}

impl ExportFormat {
    fn name(&self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Gif { .. } => "gif",
        }
    }
}

/// `screen_record.export` queues the expensive recorder render rather than
/// monopolizing the async request runtime. Its result is available from
/// `jobs.status`; the immediate response names the already-fenced destination.
pub(crate) async fn screen_record_export(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Args {
        source: String,
        plan: String,
        path: Option<String>,
        format: Option<String>,
        gif_fps: Option<u32>,
        gif_width: Option<u32>,
    }

    let args: Args = parse_args(args)?;
    let format = export_format(args.format.as_deref(), args.gif_fps, args.gif_width)?;
    let (_project, _edl, dir, revision) = snapshot(state).await?;
    let (source, plan, capture_audio) = resolve_export_inputs(&dir, &args.source, &args.plan)?;
    let retry = if args.path.is_some() {
        crate::jobs::JobRetry::ineligible(
            "explicit Save As destinations cannot be retried automatically",
        )
    } else {
        crate::jobs::JobRetry::screen_record_export(retry::retry_descriptor(
            &dir,
            revision,
            &source,
            &plan,
            &capture_audio,
            &format,
        )?)
    };
    let prepared = allocate_export(
        &dir,
        source,
        plan,
        capture_audio,
        format,
        args.path.as_deref(),
    )?;
    let output_path = prepared.output_path.clone();
    let format_name = prepared.format.name();
    let job = state
        .jobs
        .create_with_retry("screen_record_export", Some(retry));
    let job_id = job.job_id.clone();
    spawn_export_job(state, &job_id, prepared);

    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "path": output_path,
        "format": format_name,
        "status": "queued",
    })))
}

fn export_format(
    requested: Option<&str>,
    gif_fps: Option<u32>,
    gif_width: Option<u32>,
) -> Result<ExportFormat, CutError> {
    match requested.unwrap_or("mp4") {
        "mp4" => Ok(ExportFormat::Mp4),
        "gif" => Ok(ExportFormat::Gif {
            fps: gif_fps.unwrap_or(15),
            width: gif_width.unwrap_or(720),
        }),
        other => Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown screen_record.export format '{other}'"),
            "format must be mp4 | gif",
        )),
    }
}

fn resolve_export_inputs(
    dir: &Path,
    source_arg: &str,
    plan_arg: &str,
) -> Result<(PathBuf, PathBuf, crate::screen_record::CaptureExportAudio), CutError> {
    let source = crate::screen_record::plain_existing_file_under_project(
        dir,
        source_arg,
        "recording source",
        "pass the source path returned by screen_record.stop",
    )?;
    let plan = resolve_existing_project_file(
        dir,
        plan_arg,
        "EditPlan",
        "run screen_record.autoedit first and pass the returned plan path",
    )?;
    let capture_audio = crate::screen_record::export_audio_for_source(dir, &source)?;
    Ok((source, plan, capture_audio))
}

fn allocate_export(
    dir: &Path,
    source: PathBuf,
    plan: PathBuf,
    capture_audio: crate::screen_record::CaptureExportAudio,
    format: ExportFormat,
    requested_path: Option<&str>,
) -> Result<PreparedExport, CutError> {
    let out = fence_output_path(
        dir,
        requested_path,
        match format {
            ExportFormat::Mp4 => "exports/recording.mp4",
            ExportFormat::Gif { .. } => "exports/recording.gif",
        },
        match format {
            ExportFormat::Mp4 => OutputPathPolicy::MP4,
            ExportFormat::Gif { .. } => OutputPathPolicy::GIF,
        },
    )?;
    let output_path = out.display().to_string();
    Ok(PreparedExport {
        source,
        plan,
        capture_audio,
        out,
        format,
        output_path,
        _retry_input_staging: None,
    })
}

fn spawn_export_job(state: &AppState, job_id: &str, prepared: PreparedExport) {
    let PreparedExport {
        source,
        plan,
        capture_audio,
        out,
        format,
        output_path,
        _retry_input_staging,
    } = prepared;
    let output_path_for_task = output_path;
    let format_name = format.name();
    let jobs = state.jobs.clone();
    let job_id_for_task = job_id.to_string();
    let format_for_task = format.clone();
    state
        .jobs
        .spawn_limited(job_id, EXPORT_LIMIT_KEY, EXPORT_MAX_RUNNING, async move {
            // Keep retry snapshots alive from queue admission until every FFmpeg
            // read has finished. Ordinary first exports carry None here.
            let retry_input_staging = _retry_input_staging;
            let progress = ExportProgressReporter::new(
                jobs.clone(),
                job_id_for_task.clone(),
                format_for_task.name(),
            );
            progress.preparing();
            let started = Instant::now();
            let result = render_export(
                source,
                plan,
                capture_audio,
                out,
                format_for_task,
                &jobs,
                &job_id_for_task,
                progress.clone(),
            )
            .await;
            match result {
                Ok(frames) => {
                    progress.finalizing();
                    jobs.finish(
                        &job_id_for_task,
                        json!({
                            "path": output_path_for_task,
                            "format": format_name,
                            "frames": frames,
                            "elapsed_ms": started.elapsed().as_millis() as u64,
                        }),
                    )
                }
                Err(error) => jobs.fail(&job_id_for_task, error),
            }
            drop(retry_input_staging);
        });
}

async fn render_export(
    source: std::path::PathBuf,
    plan: std::path::PathBuf,
    capture_audio: crate::screen_record::CaptureExportAudio,
    out: crate::output_paths::OutputPath,
    format: ExportFormat,
    jobs: &crate::jobs::JobManager,
    job_id: &str,
    progress: ExportProgressReporter,
) -> Result<u64, CutError> {
    let work = run_blocking_cancellable("screen_record.export", move |cancellation| {
        let child_cancellation = cancellation.clone();
        let control = record_render::ffmpeg::ProcessControl::bounded(EXPORT_TIMEOUT, move || {
            child_cancellation.is_cancelled()
        });
        match format {
            ExportFormat::Mp4 => {
                progress.preparing_audio();
                let audio = capture_audio.prepare(
                    out.parent().unwrap_or_else(|| std::path::Path::new(".")),
                    &control,
                )?;
                progress.rendering_started();
                crate::screen_record::render_with_control_progress(
                    &source,
                    &plan,
                    &out,
                    audio.path(),
                    &control,
                    |frames, expected_frames| progress.rendering(frames, expected_frames),
                )
            }
            ExportFormat::Gif { fps, width } => {
                let tmp = tempfile::Builder::new()
                    .prefix(".cut-recorder-export-")
                    .suffix(".mp4")
                    .tempfile_in(out.parent().unwrap_or_else(|| std::path::Path::new(".")))
                    .map_err(|error| {
                        CutError::new(
                            error_codes::IO,
                            format!("could not create a secure GIF intermediate: {error}"),
                            "creating the recorder export intermediate failed",
                        )
                    })?
                    .into_temp_path();
                progress.preparing_audio();
                progress.rendering_started();
                let frames = crate::screen_record::render_with_control_progress(
                    &source,
                    &plan,
                    tmp.as_ref(),
                    None,
                    &control,
                    |frames, expected_frames| progress.rendering(frames, expected_frames),
                )?;
                progress.finalizing();
                control
                    .check("convert recording export to GIF")
                    .map_err(crate::screen_record::record_err)?;
                crate::screen_record::gif_with_control(tmp.as_ref(), &out, fps, width, &control)?;
                Ok(frames)
            }
        }
    });
    await_bounded_export_work(EXPORT_TIMEOUT, jobs, job_id, work).await
}

/// On timeout, keep the job active while the signalled blocking worker exits.
/// This is the point that makes it safe to report a terminal timeout: its output
/// lease is dropped only after the worker has reaped every ffmpeg child.
async fn await_bounded_export_work<T: Send + 'static>(
    timeout: Duration,
    jobs: &crate::jobs::JobManager,
    job_id: &str,
    work: impl Future<Output = Result<T, CutError>>,
) -> Result<T, CutError> {
    let cancellation = crate::jobs::current_job_cancellation();
    tokio::pin!(work);
    match tokio::time::timeout(timeout, &mut work).await {
        Ok(result) => result,
        Err(_) => {
            cancellation.request_cancel();
            jobs.progress(
                job_id,
                0.95,
                Some("Export timed out; stopping ffmpeg…".to_string()),
            );
            let _ = work.await;
            Err(CutError::new(
                error_codes::FFMPEG,
                "screen-record export timed out",
                format!(
                    "ffmpeg exceeded the {} second export limit",
                    timeout.as_secs()
                ),
            )
            .with_suggested_action("reduce the recording length or retry the export"))
        }
    }
}

#[cfg(test)]
#[path = "export_job/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "export_job/retry_staging_tests.rs"]
mod retry_staging_tests;
