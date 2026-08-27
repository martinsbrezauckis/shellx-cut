//! In-process still capture for the server-side `debug.screenshot` verb.

use super::{
    align_ffmpeg_env, capture_session_control::CaptureSessionControl, new_capture_id, record_err,
    reserve_capture,
};
use cut_core::{error_codes, CutError};
use std::path::Path;

/// Capture a single still of the primary display (or a chosen monitor / opaque
/// window id) to `out_png`, returning its dimensions. Unlike `ui.screenshot`,
/// this uses the native recorder and therefore does not depend on a connected
/// WebView. It records a short clip, extracts frame zero, then cleans its scratch
/// capture directory best-effort.
pub fn capture_screenshot_png(
    out_png: &Path,
    monitor: Option<u32>,
    window: Option<String>,
) -> Result<(u32, u32), CutError> {
    align_ffmpeg_env();
    let cap = record_capture::live_capture().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "no screen-capture backend on this build/OS",
            "debug.screenshot needs a desktop session built with the capture feature",
        )
    })?;
    // Screenshot capture owns no input listener, even on builds whose recording
    // backend does. Keep its private lifecycle stream declaration truthful.
    let control = CaptureSessionControl::new(Some(220), false, false, false);
    let stop = control.stop_signal();
    let _reservation = reserve_capture(new_capture_id(), control.clone())?;
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("cutd_shot_{uniq}"));
    std::fs::create_dir_all(&tmp).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "create screenshot scratch dir",
            e.to_string(),
        )
    })?;
    let cfg = record_capture::CaptureConfig {
        duration_ms: Some(220),
        fps: 4.0,
        capture_cursor: true,
        monitor,
        monitor_id: None,
        window,
        audio: false,
        microphone_source: record_capture::MicrophoneSource::SystemDefault,
        system_audio: false,
        capture_keys: false,
        out_dir: tmp.to_string_lossy().into_owned(),
        checkpoint: None,
        clock: None,
    };
    let captured = cap.capture(&cfg, stop);
    control.terminalize();
    let result = captured.map_err(record_err).and_then(|out| {
        let mut command = std::process::Command::new(cut_media::toolpath::ffmpeg());
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&out.source_video)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(out_png);
        let status = crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "extract screen-record screenshot frame",
        )?
        .status;
        if !status.success() || !out_png.is_file() {
            return Err(CutError::new(
                error_codes::IO,
                "screenshot frame extract failed",
                "ffmpeg could not write the PNG from the captured frame",
            ));
        }
        let mut command = std::process::Command::new(cut_media::toolpath::ffprobe());
        command
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height",
                "-of",
                "csv=p=0:s=x",
            ])
            .arg(out_png);
        Ok(crate::dispatch::run_bounded_foreground_command(
            &mut command,
            "probe screen-record screenshot dimensions",
        )
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| {
            let (width, height) = output.trim().split_once('x')?;
            Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
        })
        .unwrap_or((0, 0)))
    });
    let _ = std::fs::remove_dir_all(&tmp);
    result
}
