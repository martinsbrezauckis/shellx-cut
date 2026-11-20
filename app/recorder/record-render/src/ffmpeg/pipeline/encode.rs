//! Raw-frame encode: frames from a closure go to ffmpeg's stdin under one deadline.

use std::io::Write;
use std::process::{Command, Stdio};

use record_core::Result;

use super::{
    default_control, ff_err, ffmpeg_bin, finish_reader, fps_str, read_capped, ManagedChild,
    ProcessControl,
};

/// Encode `nframes` RGBA frames produced by `frame` into an MP4 (libx264).
pub fn encode_frames<F: FnMut(u64) -> Vec<u8>>(
    out: &str,
    w: u32,
    h: u32,
    fps: f64,
    nframes: u64,
    frame: F,
) -> Result<u64> {
    encode_frames_with_control(
        &ffmpeg_bin(),
        out,
        w,
        h,
        fps,
        nframes,
        default_control(),
        frame,
    )
}

/// [`encode_frames`] under an explicit deadline and cancellation. The child's
/// watcher kills ffmpeg when the control expires, which also ends a write
/// blocked on an encoder that stays alive but stops reading; the wait then
/// reports the timeout (or cancellation), not the broken pipe.
#[allow(clippy::too_many_arguments)]
pub(super) fn encode_frames_with_control<F: FnMut(u64) -> Vec<u8>>(
    ffmpeg: &str,
    out: &str,
    w: u32,
    h: u32,
    fps: f64,
    nframes: u64,
    control: ProcessControl,
    mut frame: F,
) -> Result<u64> {
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &fps_str(fps),
            "-i",
            "-",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "20",
            "-preset",
            "medium",
            out,
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(&mut command, control.clone(), "ffmpeg encode spawn")?;
    let mut closed = false;
    let write = {
        let mut stdin = child.take_stdin()?;
        let stderr_reader = read_capped(child.take_stderr()?);
        let mut result = Ok(());
        for i in 0..nframes {
            if let Err(error) = stdin.write_all(&frame(i)) {
                // ffmpeg stopped reading: its exit status and stderr say why.
                closed = error.kind() == std::io::ErrorKind::BrokenPipe;
                if !closed {
                    // An expired deadline or a cancellation ended the write: say that.
                    result = Err(control
                        .check("write encode frame")
                        .err()
                        .unwrap_or_else(|| ff_err("write encode frame", error)));
                }
                break;
            }
        }
        (result, stderr_reader)
    };
    if write.0.is_err() {
        child.kill_and_reap();
    }
    let status = child.wait();
    let stderr = finish_reader(write.1, &mut child, "read encode stderr");
    write.0?;
    let status = status?;
    let stderr = stderr?;
    if !status.success() {
        return Err(ff_err(
            "ffmpeg encode failed",
            String::from_utf8_lossy(&stderr),
        ));
    }
    if closed {
        return Err(ff_err(
            "write encode frame",
            "ffmpeg closed its input early",
        ));
    }
    Ok(nframes)
}
