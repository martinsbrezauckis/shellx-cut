//! ffmpeg.rs — decode / encode / probe via the `ffmpeg` + `ffprobe` binaries.
//!
//! We shell out to ffmpeg (same approach as ShellX Cut) rather than link libav:
//! simpler, fewer build deps, trivially cross-platform. Binary location honours
//! `SHELLX_RECORD_FFMPEG` / `SHELLX_RECORD_FFPROBE` overrides (needed when running
//! the Windows .exe via WSL interop, where ffmpeg lives at a Windows path), else
//! falls back to `ffmpeg` / `ffprobe` on PATH.

use std::process::{Command, Output, Stdio};
use std::time::Duration;

pub(crate) use cut_media::ffmpeg::{LOCAL_INPUT_FORMATS, LOCAL_INPUT_PROTOCOLS};
use record_core::{error_codes, RecordError, Result};

mod camera;
mod pipeline;
mod process;
#[cfg(test)]
mod tests;
pub use camera::{
    decode_square, decode_square_with_control, stream_square_with_control, CameraFrameStream,
};
pub use pipeline::{
    encode_frames, mp4_to_gif, mp4_to_gif_with_control, render_pipe, render_pipe_with_control,
};
use process::ManagedChild;
pub use process::ProcessControl;

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(30 * 60);

fn ffmpeg_bin() -> String {
    std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

fn ffprobe_bin() -> String {
    std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

fn ff_err(ctx: &str, e: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::FFMPEG, ctx, e.to_string()).with_action(
        "ensure ffmpeg/ffprobe are installed and on PATH (or set SHELLX_RECORD_FFMPEG)",
    )
}

fn default_control() -> ProcessControl {
    ProcessControl::bounded(DEFAULT_PROCESS_TIMEOUT, || false)
}

fn read_child_output(mut child: ManagedChild) -> Result<Output> {
    let stdout = child.take_stdout()?;
    let stderr = child.take_stderr()?;
    let stdout_reader = process::read_capped(stdout);
    let stderr_reader = process::read_capped(stderr);
    let status = child.wait();
    let stdout = process::finish_reader(stdout_reader, &mut child, "read ffmpeg stdout");
    let stderr = process::finish_reader(stderr_reader, &mut child, "read ffmpeg stderr");
    let status = status?;
    let stdout = stdout?;
    let stderr = stderr?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Run a finite ffmpeg-style command under the same cancellable child owner as
/// the compositor. Server-side raw-mux paths use this instead of `Command::output`
/// so a cancelled job does not leave a child behind.
pub fn command_output_with_control(
    command: &mut Command,
    control: &ProcessControl,
    context: &'static str,
) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    read_child_output(ManagedChild::spawn(command, control.clone(), context)?)
}

fn fps_str(fps: f64) -> String {
    format!("{}", fps)
}

fn rgba_frame_bytes(width: u32, height: u32, context: &str) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            context,
            "RGBA frame dimensions must be non-zero",
        ));
    }
    let width = usize::try_from(width).map_err(|_| {
        RecordError::new(
            error_codes::INVALID_ARGS,
            context,
            "RGBA frame width does not fit this platform",
        )
    })?;
    let height = usize::try_from(height).map_err(|_| {
        RecordError::new(
            error_codes::INVALID_ARGS,
            context,
            "RGBA frame height does not fit this platform",
        )
    })?;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            RecordError::new(
                error_codes::INVALID_ARGS,
                context,
                "RGBA frame byte size overflows this platform",
            )
        })
}

/// Probed source stream facts.
pub struct Probe {
    pub w: u32,
    pub h: u32,
    pub fps: f64,
}

/// Probe a media file's first video stream for width/height/fps.
pub fn probe(path: &str) -> Result<Probe> {
    probe_with_control(path, &default_control())
}

pub fn probe_with_control(path: &str, control: &ProcessControl) -> Result<Probe> {
    let mut command = Command::new(ffprobe_bin());
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            LOCAL_INPUT_PROTOCOLS,
            "-format_whitelist",
            LOCAL_INPUT_FORMATS,
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0:s=,",
            path,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = read_child_output(ManagedChild::spawn(
        &mut command,
        control.clone(),
        "ffprobe spawn",
    )?)?;
    if !out.status.success() {
        return Err(ff_err(
            "ffprobe failed",
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next().unwrap_or("").trim();
    let parts: Vec<&str> = line.split(',').collect();
    let w: u32 = parts
        .first()
        .and_then(|x| x.trim().parse().ok())
        .unwrap_or(0);
    let h: u32 = parts
        .get(1)
        .and_then(|x| x.trim().parse().ok())
        .unwrap_or(0);
    let fps = parts
        .get(2)
        .map(|r| {
            let mut it = r.trim().split('/');
            let n: f64 = it.next().unwrap_or("30").parse().unwrap_or(30.0);
            let d: f64 = it.next().unwrap_or("1").parse().unwrap_or(1.0);
            if d != 0.0 {
                n / d
            } else {
                30.0
            }
        })
        .unwrap_or(30.0);
    if w == 0 || h == 0 {
        return Err(ff_err(
            "bad probe dims",
            format!("ffprobe returned '{line}'"),
        ));
    }
    Ok(Probe { w, h, fps })
}

/// Grab a single RGBA frame near `t_ms` (for fast PNG previews / golden checks).
pub fn grab_frame(src: &str, t_ms: u64) -> Result<(u32, u32, Vec<u8>)> {
    grab_frame_with_control(src, t_ms, &default_control())
}

pub fn grab_frame_with_control(
    src: &str,
    t_ms: u64,
    control: &ProcessControl,
) -> Result<(u32, u32, Vec<u8>)> {
    let p = probe_with_control(src, control)?;
    let t = format!("{:.3}", t_ms as f64 / 1000.0);
    let mut command = Command::new(ffmpeg_bin());
    command
        .args([
            "-v",
            "error",
            "-ss",
            &t,
            "-protocol_whitelist",
            LOCAL_INPUT_PROTOCOLS,
            "-format_whitelist",
            LOCAL_INPUT_FORMATS,
            "-i",
            src,
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    let want = rgba_frame_bytes(p.w, p.h, "grab frame")?;
    let mut child = ManagedChild::spawn(&mut command, control.clone(), "ffmpeg grab spawn")?;
    let mut stdout = child.take_stdout()?;
    let stderr_reader = process::read_capped(child.take_stderr()?);
    let frame = camera::read_raw_frame(&mut stdout, want)?;
    // `-frames:v 1` bounds the producer, so no diagnostic-sized stdout cap is
    // needed for this binary frame.
    drop(stdout);
    let status = child.wait()?;
    let stderr = process::finish_reader(stderr_reader, &mut child, "read grab stderr")?;
    if !status.success() {
        return Err(ff_err(
            "ffmpeg grab failed",
            String::from_utf8_lossy(&stderr),
        ));
    }
    let frame = frame.ok_or_else(|| ff_err("grab size mismatch", "frame is missing"))?;
    Ok((p.w, p.h, frame))
}
