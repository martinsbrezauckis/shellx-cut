//! Standalone raw-capture muxing, including the recorded system-audio offset.

use super::align_ffmpeg_env;
use cut_core::{error_codes, CutError};
use std::path::Path;

/// Seconds of the copied video, formatted for an FFmpeg filter argument.
/// Audio is padded and cut to this length instead of relying on `-shortest`:
/// with an endless pad FFmpeg could keep encoding silence forever when the
/// video stream had already ended.
pub(super) fn video_duration_arg(source: &Path) -> Result<String, CutError> {
    let video_duration_ms = cut_media::probe(source)?
        .duration_ms
        .filter(|duration_ms| *duration_ms > 0)
        .ok_or_else(|| {
            CutError::new(
                error_codes::FFMPEG,
                "raw source video has no measurable duration",
                "cannot bound the mixed recording audio to its source video",
            )
        })?;
    Ok(format!(
        "{}.{:03}",
        video_duration_ms / 1_000,
        video_duration_ms % 1_000
    ))
}

pub(crate) fn mux_raw_sources(
    source: &Path,
    mic: Option<&Path>,
    system: Option<&Path>,
    system_offset_ms: Option<u64>,
    out: &Path,
) -> Result<(), CutError> {
    align_ffmpeg_env();
    let ffmpeg = cut_media::toolpath::ffmpeg();
    let mut cmd = std::process::Command::new(&ffmpeg);
    cmd.arg("-y");
    cut_media::ffmpeg::push_ffmpeg_literal_input(&mut cmd, source)?;
    match (mic, system) {
        (None, None) => {
            cmd.args(["-map", "0:v:0", "-c", "copy"]);
        }
        (Some(audio), None) => {
            let video_duration = video_duration_arg(source)?;
            cut_media::ffmpeg::push_ffmpeg_literal_input(&mut cmd, audio)?;
            cmd.args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                // Keep copied video through its final packet when audio ends
                // first, and stop the audio where the video stops.
                "-af",
                &format!("apad=whole_dur={video_duration},atrim=duration={video_duration}"),
            ]);
        }
        (None, Some(system)) => {
            let video_duration = video_duration_arg(source)?;
            let offset = system_offset_ms.unwrap_or(0);
            if offset != 0 {
                cmd.arg("-itsoffset")
                    .arg(format!("{:.3}", offset as f64 / 1_000.0));
            }
            cut_media::ffmpeg::push_ffmpeg_literal_input(&mut cmd, system)?;
            cmd.args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-af",
                &format!("apad=whole_dur={video_duration},atrim=duration={video_duration}"),
            ]);
        }
        (Some(mic), Some(system)) => {
            let video_duration = video_duration_arg(source)?;
            cut_media::ffmpeg::push_ffmpeg_literal_input(&mut cmd, mic)?;
            cut_media::ffmpeg::push_ffmpeg_literal_input(&mut cmd, system)?;
            // Pad each source to the video length before mixing instead of
            // padding the mixed result: FFmpeg can end a mix early when the
            // padding comes after the mixer, which cut the audio track short
            // of the video on a busy machine.
            let filter = format!(
                "[1:a]aresample=48000,apad=whole_dur={video_duration}[m];\
                 [2:a]aresample=48000,adelay={}:all=1,apad=whole_dur={video_duration}[s];\
                 [m][s]amix=inputs=2:duration=longest:normalize=0,\
                 atrim=duration={video_duration}[a]",
                system_offset_ms.unwrap_or(0)
            );
            cmd.args([
                "-filter_complex",
                &filter,
                "-map",
                "0:v:0",
                "-map",
                "[a]",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-ar",
                "48000",
            ]);
        }
    }
    cut_media::stage_and_publish(out, false, |staged_out| {
        cmd.arg(staged_out);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let control = record_render::ffmpeg::ProcessControl::bounded(
            std::time::Duration::from_secs(30 * 60),
            || false,
        );
        let output = record_render::ffmpeg::command_output_with_control(
            &mut cmd,
            &control,
            "raw mux ffmpeg spawn",
        )
        .map_err(super::record_err)?;
        if !output.status.success() {
            let tail = String::from_utf8_lossy(&output.stderr);
            let last = tail.lines().last().unwrap_or("ffmpeg failed").to_string();
            return Err(CutError::new(
                error_codes::FFMPEG,
                format!("raw mux failed: {last}"),
                "ffmpeg combine of the raw recording's sound sources failed",
            ));
        }
        Ok(())
    })?;
    Ok(())
}
