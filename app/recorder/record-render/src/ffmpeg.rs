//! ffmpeg.rs — decode / encode / probe via the `ffmpeg` + `ffprobe` binaries.
//!
//! We shell out to ffmpeg (same approach as ShellX Cut) rather than link libav:
//! simpler, fewer build deps, trivially cross-platform. Binary location honours
//! `SHELLX_RECORD_FFMPEG` / `SHELLX_RECORD_FFPROBE` overrides (needed when running
//! the Windows .exe via WSL interop, where ffmpeg lives at a Windows path), else
//! falls back to `ffmpeg` / `ffprobe` on PATH.

use std::io::Read;
use std::process::{ChildStdout, Command, Output, Stdio};
use std::time::Duration;

use record_core::{error_codes, RecordError, Result};

mod pipeline;
mod process;
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

/// Read one complete raw frame. A partial frame is an error, never a usable
/// camera frame. The child owner can interrupt a blocked read on cancellation.
fn read_raw_frame(reader: &mut impl Read, frame_bytes: usize) -> Result<Option<Vec<u8>>> {
    let mut frame = vec![0; frame_bytes];
    let mut filled = 0;
    while filled < frame_bytes {
        match reader.read(&mut frame[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(ff_err("raw frame decode", "truncated RGBA frame")),
            Ok(n) => filled += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ff_err("raw frame decode", error)),
        }
    }
    Ok(Some(frame))
}

/// A single-frame camera cache advanced in lockstep with output frames. Long
/// recordings keep constant camera memory instead of buffering the full track.
pub struct CameraFrameStream {
    child: ManagedChild,
    stdout: Option<ChildStdout>,
    stderr: Option<process::Reader>,
    frame_bytes: usize,
    next_index: usize,
    current_index: Option<usize>,
    current: Vec<u8>,
    ended: bool,
}

impl CameraFrameStream {
    pub fn at(&mut self, index: usize) -> Result<Option<&[u8]>> {
        if self.ended {
            return Ok(None);
        }
        if self.current_index == Some(index) {
            return Ok(Some(&self.current));
        }
        if self.current_index.is_some_and(|current| index < current) {
            return Err(ff_err("webcam decode", "camera frame index moved backward"));
        }
        while self.next_index <= index {
            let frame = read_raw_frame(
                self.stdout.as_mut().expect("owned camera stdout"),
                self.frame_bytes,
            )?;
            let Some(frame) = frame else {
                // EOF is an admissible tail only when the decoder itself exited
                // successfully. A failed decoder must not hide a lost overlay.
                let status = self.child.wait()?;
                let stderr = process::finish_reader(
                    self.stderr.take().expect("owned camera stderr"),
                    &mut self.child,
                    "read webcam stderr",
                )?;
                if !status.success() {
                    return Err(ff_err(
                        "webcam decode failed",
                        String::from_utf8_lossy(&stderr),
                    ));
                }
                self.ended = true;
                self.current.clear();
                return Ok(None);
            };
            self.current = frame;
            self.current_index = Some(self.next_index);
            self.next_index += 1;
        }
        Ok(Some(&self.current))
    }

    pub fn finish(mut self) -> Result<()> {
        if !self.ended {
            // The screen can end before a longer camera source. Its unconsumed
            // tail is irrelevant to this render; close and reap our decoder.
            drop(self.stdout.take());
            self.child.stop_and_reap()?;
            if let Some(stderr) = self.stderr.take() {
                process::finish_reader(stderr, &mut self.child, "read webcam stderr")?;
            }
        }
        Ok(())
    }
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

/// Decode a webcam video into square `bp`×`bp` RGBA frames at `fps` (center-
/// cropped to square then scaled). The render path uses `stream_square_with_control`
/// to keep bounded memory; this compatibility helper materializes all frames.
pub fn decode_square(src: &str, bp: u32, fps: f64) -> Result<Vec<Vec<u8>>> {
    decode_square_with_control(src, bp, fps, &default_control())
}

pub fn decode_square_with_control(
    src: &str,
    bp: u32,
    fps: f64,
    control: &ProcessControl,
) -> Result<Vec<Vec<u8>>> {
    let mut stream = stream_square_with_control(src, bp, fps, control)?;
    let mut frames = Vec::new();
    loop {
        let next = read_raw_frame(
            stream.stdout.as_mut().expect("owned camera stdout"),
            stream.frame_bytes,
        )?;
        match next {
            Some(frame) => frames.push(frame),
            None => break,
        }
    }
    let status = stream.child.wait()?;
    let stderr = process::finish_reader(
        stream.stderr.take().expect("owned camera stderr"),
        &mut stream.child,
        "read webcam stderr",
    )?;
    if !status.success() {
        return Err(ff_err(
            "webcam decode failed",
            String::from_utf8_lossy(&stderr),
        ));
    }
    if frames.is_empty() {
        return Err(ff_err("webcam decode", "camera frames are missing"));
    }
    Ok(frames)
}

pub fn stream_square_with_control(
    src: &str,
    bp: u32,
    fps: f64,
    control: &ProcessControl,
) -> Result<CameraFrameStream> {
    const MAX_BUBBLE_PX: u32 = 8192;
    if bp == 0 || bp > MAX_BUBBLE_PX {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            "webcam decode",
            "bubble dimensions must be in 1..=8192",
        ));
    }
    if !(fps.is_finite() && fps > 0.0 && fps <= 240.0) {
        return Err(RecordError::new(
            error_codes::INVALID_ARGS,
            "webcam decode",
            "fps must be finite and in (0, 240]",
        ));
    }
    let frame_bytes = rgba_frame_bytes(bp, bp, "webcam decode")?;
    let vf = format!(
        "crop='min(iw,ih)':'min(iw,ih)',scale={bp}:{bp},fps={}",
        fps.round() as u32
    );
    let mut command = Command::new(ffmpeg_bin());
    command
        .args([
            "-v", "error", "-i", src, "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    let child = ManagedChild::spawn(&mut command, control.clone(), "webcam decode spawn")?;
    let stdout = child.take_stdout()?;
    let stderr = process::read_capped(child.take_stderr()?);
    Ok(CameraFrameStream {
        child,
        stdout: Some(stdout),
        stderr: Some(stderr),
        frame_bytes,
        next_index: 0,
        current_index: None,
        current: Vec::new(),
        ended: false,
    })
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
    let frame = read_raw_frame(&mut stdout, want)?;
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

#[cfg(test)]
mod tests {
    use super::{decode_square, read_raw_frame, rgba_frame_bytes};

    #[test]
    fn rgba_geometry_rejects_zero_and_overflow_before_allocation() {
        assert_eq!(rgba_frame_bytes(2, 3, "test").unwrap(), 24);
        assert!(rgba_frame_bytes(0, 3, "test").is_err());
        assert!(rgba_frame_bytes(u32::MAX, u32::MAX, "test").is_err());
    }

    #[test]
    fn webcam_decode_rejects_invalid_geometry_and_fps_before_spawning() {
        assert!(decode_square("unused", 0, 30.0).is_err());
        assert!(decode_square("unused", 8193, 30.0).is_err());
        assert!(decode_square("unused", 64, f64::NAN).is_err());
        assert!(decode_square("unused", 64, 241.0).is_err());
    }

    #[test]
    fn raw_frames_above_diagnostic_cap_are_complete_and_partial_frames_fail() {
        let full = vec![0x73; 800 * 400 * 4];
        let mut input = std::io::Cursor::new(full.clone());
        assert_eq!(read_raw_frame(&mut input, full.len()).unwrap(), Some(full));
        assert!(read_raw_frame(&mut input, 4).unwrap().is_none());
        let mut partial = std::io::Cursor::new(vec![7; 3]);
        assert!(read_raw_frame(&mut partial, 4).is_err());
    }

    #[test]
    fn grab_frame_accepts_raw_video_larger_than_diagnostic_cap() {
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        if !std::process::Command::new(&ffmpeg)
            .arg("-version")
            .output()
            .is_ok_and(|out| out.status.success())
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("wide.mp4");
        let encoded = std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=800x400:r=1:d=1",
                "-frames:v",
                "1",
                "-c:v",
                "libx264",
            ])
            .arg(&src)
            .output()
            .unwrap();
        assert!(encoded.status.success());
        let (w, h, pixels) = super::grab_frame(src.to_str().unwrap(), 0).unwrap();
        assert_eq!((w, h, pixels.len()), (800, 400, 800 * 400 * 4));
        assert!(pixels[0] > 150);
    }

    #[test]
    fn camera_stream_accepts_a_clean_short_tail_but_rejects_decode_failure() {
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        if !std::process::Command::new(&ffmpeg)
            .arg("-version")
            .output()
            .is_ok_and(|out| out.status.success())
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("short.mp4");
        let encoded = std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x64:r=10:d=0.3",
                "-c:v",
                "libx264",
            ])
            .arg(&src)
            .output()
            .unwrap();
        assert!(encoded.status.success());
        let mut camera = super::stream_square_with_control(
            src.to_str().unwrap(),
            32,
            10.0,
            &super::default_control(),
        )
        .unwrap();
        assert_eq!(camera.at(0).unwrap().unwrap().len(), 32 * 32 * 4);
        assert!(camera.at(10).unwrap().is_none());
        assert!(camera.at(11).unwrap().is_none());
        camera.finish().unwrap();

        let invalid = dir.path().join("invalid.mov");
        std::fs::write(&invalid, b"not a camera movie").unwrap();
        let mut failed = super::stream_square_with_control(
            invalid.to_str().unwrap(),
            32,
            10.0,
            &super::default_control(),
        )
        .unwrap();
        assert!(
            failed.at(0).is_err(),
            "nonzero ffmpeg exit must not be a clean tail"
        );

        let longer = dir.path().join("long-camera.mp4");
        let encoded = std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x64:r=10:d=5",
                "-c:v",
                "libx264",
            ])
            .arg(&longer)
            .output()
            .unwrap();
        assert!(encoded.status.success());
        let mut unused_tail = super::stream_square_with_control(
            longer.to_str().unwrap(),
            32,
            10.0,
            &super::default_control(),
        )
        .unwrap();
        assert!(unused_tail.at(0).unwrap().is_some());
        unused_tail.finish().unwrap();
    }
}
