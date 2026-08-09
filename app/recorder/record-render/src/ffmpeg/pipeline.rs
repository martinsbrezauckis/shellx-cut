//! Streaming encode/decode pipelines owned by bounded ffmpeg children.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use record_core::Result;

use super::process::{finish_reader, read_capped};
use super::{default_control, ff_err, ffmpeg_bin, fps_str, probe_with_control, rgba_frame_bytes};
use super::{ManagedChild, ProcessControl};

/// Encode `nframes` RGBA frames produced by `frame` into an MP4 (libx264).
pub fn encode_frames<F: FnMut(u64) -> Vec<u8>>(
    out: &str,
    w: u32,
    h: u32,
    fps: f64,
    nframes: u64,
    mut frame: F,
) -> Result<u64> {
    let mut command = Command::new(ffmpeg_bin());
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
    let mut child = ManagedChild::spawn(&mut command, default_control(), "ffmpeg encode spawn")?;
    let write = {
        let mut stdin = child.take_stdin()?;
        let stderr_reader = read_capped(child.take_stderr()?);
        let mut result = Ok(());
        for i in 0..nframes {
            if let Err(error) = stdin.write_all(&frame(i)) {
                result = Err(ff_err("write encode frame", error));
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
    Ok(nframes)
}

/// Stream-render a source through `compose`, then encode the output MP4.
#[allow(clippy::too_many_arguments)]
pub fn render_pipe<F: FnMut(&[u8], u64) -> Vec<u8>>(
    src: &str,
    out: &str,
    out_w: u32,
    out_h: u32,
    fps: f64,
    audio_input: &str,
    normalize: bool,
    compose: F,
) -> Result<u64> {
    render_pipe_with_control(
        src,
        out,
        out_w,
        out_h,
        fps,
        audio_input,
        normalize,
        &default_control(),
        compose,
    )
}

/// Cancellable form used by the server's tracked recording jobs.
#[allow(clippy::too_many_arguments)]
pub fn render_pipe_with_control<F: FnMut(&[u8], u64) -> Vec<u8>>(
    src: &str,
    out: &str,
    out_w: u32,
    out_h: u32,
    fps: f64,
    audio_input: &str,
    normalize: bool,
    control: &ProcessControl,
    mut compose: F,
) -> Result<u64> {
    let probe = probe_with_control(src, control)?;
    let frame_bytes = rgba_frame_bytes(probe.w, probe.h, "decode source frames")?;
    // Rawvideo has no timestamps. Normalize the source's timestamped frames to
    // the planned output rate before that information is discarded: otherwise
    // ffmpeg's implicit CFR output follows a sparse/VFR input's high nominal
    // `r_frame_rate` and the encoder writes those duplicated frames at `fps`.
    let decode_filter = format!("fps={}", fps_str(fps));
    let mut decode = Command::new(ffmpeg_bin());
    decode
        .args([
            "-v",
            "error",
            "-i",
            src,
            "-vf",
            &decode_filter,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut decoder = ManagedChild::spawn(&mut decode, control.clone(), "ffmpeg decode spawn")?;

    let size = format!("{out_w}x{out_h}");
    let rate = fps_str(fps);
    let mut args = vec![
        "-v",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &size,
        "-r",
        &rate,
        "-i",
        "-",
        "-i",
        audio_input,
        "-map",
        "0:v:0",
        "-map",
        "1:a:0?",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "20",
        "-preset",
        "medium",
    ];
    if normalize {
        args.extend(["-af", "loudnorm=I=-16:TP=-1.5:LRA=11,apad", "-shortest"]);
    }
    args.extend(["-c:a", "aac", out]);
    let mut encode = Command::new(ffmpeg_bin());
    encode
        .args(&args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped());
    let mut encoder = ManagedChild::spawn(&mut encode, control.clone(), "ffmpeg encode spawn")?;

    let mut stdout = decoder.take_stdout()?;
    let mut stdin = encoder.take_stdin()?;
    let stderr_reader = read_capped(encoder.take_stderr()?);
    let result = (|| {
        let mut frame = vec![0; frame_bytes];
        let mut count = 0;
        loop {
            control.check("render recorder frames")?;
            let mut filled = 0;
            while filled < frame.len() {
                match stdout.read(&mut frame[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(error) => return Err(ff_err("read decode frame", error)),
                }
            }
            if filled == 0 || filled != frame.len() {
                break;
            }
            let time_ms = (count as f64 * 1000.0 / fps) as u64;
            let composed = compose(&frame, time_ms);
            control.check("compose recorder frame")?;
            if let Err(error) = stdin.write_all(&composed) {
                if error.kind() == std::io::ErrorKind::BrokenPipe {
                    break;
                }
                return Err(control
                    .check("write encode frame")
                    .err()
                    .unwrap_or_else(|| ff_err("write encode frame", error)));
            }
            count += 1;
        }
        Ok(count)
    })();
    drop(stdin);
    drop(stdout);
    if result.is_err() {
        decoder.kill_and_reap();
        encoder.kill_and_reap();
    }
    let decode_status = decoder.wait();
    let encode_status = encoder.wait();
    let stderr = finish_reader(stderr_reader, &mut encoder, "read encode stderr");
    let decode_status = decode_status?;
    let encode_status = encode_status?;
    let stderr = stderr?;
    let count = result?;
    if !decode_status.success() {
        return Err(ff_err(
            "ffmpeg decode failed",
            format!("exit {decode_status}"),
        ));
    }
    if !encode_status.success() {
        return Err(ff_err(
            "ffmpeg encode failed",
            String::from_utf8_lossy(&stderr),
        ));
    }
    Ok(count)
}

/// Convert an MP4 to an animated GIF using palettegen/paletteuse.
pub fn mp4_to_gif(src: &str, out: &str, fps: u32, width: u32) -> Result<()> {
    mp4_to_gif_with_control(src, out, fps, width, &default_control())
}

pub fn mp4_to_gif_with_control(
    src: &str,
    out: &str,
    fps: u32,
    width: u32,
    control: &ProcessControl,
) -> Result<()> {
    let filter = format!(
        "[0:v] fps={fps},scale={width}:-1:flags=lanczos,split [a][b];[a] palettegen=stats_mode=diff [p];[b][p] paletteuse=dither=bayer:bayer_scale=3"
    );
    let mut command = Command::new(ffmpeg_bin());
    command
        .args([
            "-v",
            "error",
            "-y",
            "-i",
            src,
            "-filter_complex",
            &filter,
            "-loop",
            "0",
            out,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(&mut command, control.clone(), "ffmpeg gif spawn")?;
    let stderr_reader = read_capped(child.take_stderr()?);
    let status = child.wait();
    let stderr = finish_reader(stderr_reader, &mut child, "read gif stderr");
    let status = status?;
    let stderr = stderr?;
    if !status.success() {
        return Err(ff_err(
            "ffmpeg gif failed",
            String::from_utf8_lossy(&stderr),
        ));
    }
    Ok(())
}
