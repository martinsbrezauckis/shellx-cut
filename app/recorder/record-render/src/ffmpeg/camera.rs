//! Camera frame decoding and bounded streaming for the recording compositor.

use std::io::Read;
use std::process::{ChildStdout, Command, Stdio};

use record_core::{error_codes, RecordError, Result};

use super::process::{self, ManagedChild};
use super::{default_control, ff_err, ffmpeg_bin, rgba_frame_bytes, ProcessControl};

/// Read one complete raw frame. A partial frame is an error, never a usable
/// camera frame. The child owner can interrupt a blocked read on cancellation.
pub(super) fn read_raw_frame(
    reader: &mut impl Read,
    frame_bytes: usize,
) -> Result<Option<Vec<u8>>> {
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
            "-v",
            "error",
            "-protocol_whitelist",
            super::LOCAL_INPUT_PROTOCOLS,
            "-format_whitelist",
            super::LOCAL_INPUT_FORMATS,
            "-i",
            src,
            "-vf",
            &vf,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
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
