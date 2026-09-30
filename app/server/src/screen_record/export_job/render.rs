//! Controlled render worker for the admitted recorder export.
use super::*;

pub(super) async fn render_export(
    source: std::path::PathBuf,
    plan: record_core::EditPlan,
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
