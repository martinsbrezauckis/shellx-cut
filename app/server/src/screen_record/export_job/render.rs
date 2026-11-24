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
                let stage = cut_media::PrivateStageDir::new()?;
                progress.preparing_audio();
                let audio = capture_audio.prepare(stage.path(), &control)?;
                let staged_out = stage.reserve_named_file("recorder-export.mp4")?;
                progress.rendering_started();
                let frames = crate::screen_record::render_with_control_progress(
                    &source,
                    &plan,
                    &staged_out,
                    audio.path(),
                    &control,
                    |frames, expected_frames| progress.rendering(frames, expected_frames),
                )?;
                control
                    .check("publish recording export MP4")
                    .map_err(crate::screen_record::record_err)?;
                cut_media::publish_staged_file(&staged_out, &out, false)?;
                Ok(frames)
            }
            ExportFormat::Gif { fps, width } => {
                let stage = cut_media::PrivateStageDir::new()?;
                let staged_mp4 = stage.reserve_named_file("recorder-gif-src.mp4")?;
                let staged_gif = stage.reserve_named_file("recorder-export.gif")?;
                progress.preparing_audio();
                progress.rendering_started();
                let frames = crate::screen_record::render_with_control_progress(
                    &source,
                    &plan,
                    &staged_mp4,
                    None,
                    &control,
                    |frames, expected_frames| progress.rendering(frames, expected_frames),
                )?;
                progress.finalizing();
                control
                    .check("convert recording export to GIF")
                    .map_err(crate::screen_record::record_err)?;
                crate::screen_record::gif_with_control(
                    &staged_mp4,
                    &staged_gif,
                    fps,
                    width,
                    &control,
                )?;
                control
                    .check("publish recording export GIF")
                    .map_err(crate::screen_record::record_err)?;
                cut_media::publish_staged_file(&staged_gif, &out, false)?;
                Ok(frames)
            }
        }
    });
    await_bounded_export_work(EXPORT_TIMEOUT, jobs, job_id, work).await
}
