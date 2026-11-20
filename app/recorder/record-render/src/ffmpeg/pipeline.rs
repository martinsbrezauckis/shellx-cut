//! Streaming encode/decode pipelines owned by bounded ffmpeg children.

use std::io::Write;
use std::process::{Command, Stdio};

use record_core::Result;

use super::process::{finish_reader, read_capped};
use super::{default_control, ff_err, ffmpeg_bin, fps_str, probe_with_control, rgba_frame_bytes};

mod encode;
mod parallel;
use super::{ManagedChild, ProcessControl};
pub use encode::encode_frames;
pub use parallel::compose_workers;

/// Audio filter for the normalized encode: loudness-normalize, then pad and cut
/// the audio to the video length (rounded up to a whole frame). `-shortest` with
/// an endless pad could keep FFmpeg encoding silence forever once the video
/// input had ended, so the length is stated instead of left to the muxer.
fn normalize_audio_args(video_ms: u64, fps: f64) -> Vec<String> {
    let frames = (video_ms as f64 * fps / 1000.0).ceil().max(1.0);
    let span_ms = (frames * 1000.0 / fps).ceil() as u64;
    let span = format!("{}.{:03}", span_ms / 1_000, span_ms % 1_000);
    vec![
        "-af".to_string(),
        format!("loudnorm=I=-16:TP=-1.5:LRA=11,apad=whole_dur={span},atrim=duration={span}"),
    ]
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
    mut compose: F,
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
        |frame, time_ms| Ok(compose(frame, time_ms)),
    )
}

/// Cancellable form used by the server's tracked recording jobs.
#[allow(clippy::too_many_arguments)]
pub fn render_pipe_with_control<F: FnMut(&[u8], u64) -> Result<Vec<u8>>>(
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
    // One worker: the whole frame is made in the in-order step.
    render_pipe_parallel_with_control(
        src,
        out,
        out_w,
        out_h,
        fps,
        audio_input,
        normalize,
        control,
        1,
        X264_EXPORT_PRESET,
        |frame, time_ms| compose(&frame, time_ms),
        |composed, _| Ok(composed),
    )
}

/// x264 speed of the default render (the recording export).
pub const X264_EXPORT_PRESET: &str = "medium";
/// x264 speed of the polished editable clip. A 62 s 4K take spent 38 s in
/// `medium` encoding alone on a 10-core Mac; `veryfast` at the same
/// quality target took 27 s (the floor set by moving RGBA frames through the
/// pipe) and made a smaller file of that synthetic source. The clip is an
/// editing intermediate: its export is encoded again.
pub const X264_POLISH_PRESET: &str = "veryfast";

/// Stream-render a source with `workers` frames composed at once.
///
/// `prepare` runs on the reading thread in frame order (it may hold stream
/// state, such as a camera reader) and receives the decoded RGBA frame;
/// `compose` runs for up to `workers` frames at the same time; the composed
/// frames are written to the encoder in order. A compose error or panic stops
/// the render with that error after every child is reaped.
#[allow(clippy::too_many_arguments)]
pub fn render_pipe_parallel_with_control<P, Prep, C>(
    src: &str,
    out: &str,
    out_w: u32,
    out_h: u32,
    fps: f64,
    audio_input: &str,
    normalize: bool,
    control: &ProcessControl,
    workers: usize,
    preset: &str,
    mut prepare: Prep,
    compose: C,
) -> Result<u64>
where
    P: Send,
    Prep: FnMut(Vec<u8>, u64) -> Result<P>,
    C: Fn(P, u64) -> Result<Vec<u8>> + Sync,
{
    let workers = workers.max(1);
    let probe = probe_with_control(src, control)?;
    let frame_bytes = rgba_frame_bytes(probe.w, probe.h, "decode source frames")?;
    // Rawvideo has no timestamps. Normalize the source's timestamped frames to
    // the planned output rate before that information is discarded: otherwise
    // ffmpeg's implicit CFR output follows a sparse/VFR input's high nominal
    // `r_frame_rate` and the encoder writes those duplicated frames at `fps`.
    let decode_filter = format!("fps={}", fps_str(fps));
    let mut decode = Command::new(ffmpeg_bin());
    decode.args(["-v", "error"]);
    super::push_literal_input(&mut decode, std::path::Path::new(src), control)?;
    decode
        .args([
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

    let audio_filter_args = if normalize {
        let source_ms = cut_media::probe(std::path::Path::new(src))
            .ok()
            .and_then(|probe| probe.duration_ms)
            .filter(|duration_ms| *duration_ms > 0)
            .ok_or_else(|| {
                ff_err(
                    "recording video has no measurable duration",
                    "cannot bound the normalized audio to the video length",
                )
            })?;
        normalize_audio_args(source_ms, fps)
    } else {
        Vec::new()
    };
    let size = format!("{out_w}x{out_h}");
    let rate = fps_str(fps);
    let audio_input_args = super::literal_input_args(std::path::Path::new(audio_input), control)?;
    let mut args: Vec<&str> = vec![
        "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &size, "-r", &rate, "-i",
        "-",
    ];
    args.extend(audio_input_args.iter().map(String::as_str));
    args.extend([
        "-map", "0:v:0", "-map", "1:a:0?", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20",
        "-preset", preset,
    ]);
    args.extend(audio_filter_args.iter().map(String::as_str));
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
    let mut frames_read = 0_u64;
    let result = parallel::stream(
        workers,
        control,
        || {
            let Some(frame) = parallel::read_frame(&mut stdout, frame_bytes)? else {
                return Ok(None);
            };
            let time_ms = (frames_read as f64 * 1000.0 / fps) as u64;
            frames_read += 1;
            Ok(Some((prepare(frame, time_ms)?, time_ms)))
        },
        &compose,
        &mut |frame: Vec<u8>| match stdin.write_all(&frame) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
            Err(error) => Err(control
                .check("write encode frame")
                .err()
                .unwrap_or_else(|| ff_err("write encode frame", error))),
        },
    );
    drop(stdin);
    drop(stdout);
    if result.is_err() {
        decoder.kill_and_reap();
        encoder.kill_and_reap();
    }
    let decode_status = decoder.wait();
    let encode_status = encoder.wait();
    let stderr = finish_reader(stderr_reader, &mut encoder, "read encode stderr");
    // Intentional pipe shutdown makes child exits nonzero after a compose error.
    // Preserve that first product/camera error after all children are reaped.
    let count = result?;
    let decode_status = decode_status?;
    let encode_status = encode_status?;
    let stderr = stderr?;
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
    command.args(["-v", "error", "-y"]);
    super::push_literal_input(&mut command, std::path::Path::new(src), control)?;
    command
        .args(["-filter_complex", &filter, "-loop", "0", out])
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

#[cfg(test)]
mod tests;
