//! Standalone raw-capture muxing, including the recorded system-audio offset.

use super::align_ffmpeg_env;
use cut_core::{error_codes, CutError};
use std::path::Path;

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
    cmd.arg("-y").arg("-i").arg(source);
    match (mic, system) {
        (None, None) => {
            cmd.args(["-map", "0:v:0", "-c", "copy"]);
        }
        (Some(audio), None) => {
            cmd.arg("-i").arg(audio).args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-shortest",
            ]);
        }
        (None, Some(system)) => {
            let offset = system_offset_ms.unwrap_or(0);
            if offset != 0 {
                cmd.arg("-itsoffset")
                    .arg(format!("{:.3}", offset as f64 / 1_000.0));
            }
            cmd.arg("-i").arg(system).args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-shortest",
            ]);
        }
        (Some(mic), Some(system)) => {
            cmd.arg("-i").arg(mic);
            let offset = system_offset_ms.unwrap_or(0);
            if offset != 0 {
                cmd.arg("-itsoffset")
                    .arg(format!("{:.3}", offset as f64 / 1_000.0));
            }
            cmd.arg("-i").arg(system);
            cmd.args([
                "-filter_complex",
                "[1:a]aresample=48000[m];[2:a]aresample=48000[s];[m][s]amix=inputs=2:duration=longest:normalize=0[a]",
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
                "-shortest",
            ]);
        }
    }
    cmd.arg(out);
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
}
