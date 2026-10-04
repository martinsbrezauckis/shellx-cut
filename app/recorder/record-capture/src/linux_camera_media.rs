//! Closed nameless movie probe and deterministic camera artifact identity.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use record_core::{CameraTerminalState, Result};
use sha2::{Digest, Sha256};

use super::error;
use crate::camera_finalization::{CameraMediaProbe, CameraNativeCloser, MeasuredCameraMediaFacts};

pub(super) struct ClosedStage(pub(super) Option<File>);

unsafe impl CameraNativeCloser for ClosedStage {
    fn close_media(&mut self) -> Result<File> {
        self.0
            .take()
            .ok_or_else(|| error("close Linux camera media", "closed stage already consumed"))
    }
}

pub(super) struct LinuxMediaProbe;

impl CameraMediaProbe for LinuxMediaProbe {
    fn measure(&self, closed_file: &File) -> Result<MeasuredCameraMediaFacts> {
        // ffprobe/ffmpeg are child processes; /proc/self would refer to their
        // own descriptor tables, where this O_CLOEXEC handle is absent.
        let path = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            closed_file.as_raw_fd()
        ));
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
        let facts = record_recovery::verify_media(&ffmpeg, &ffprobe, &path)
            .map_err(|cause| error("probe closed Linux camera movie", &cause.to_string()))?;
        if facts.has_audio {
            return Err(error(
                "probe closed Linux camera movie",
                "camera sidecar contains unexpected audio",
            ));
        }
        let rate = facts.r_frame_rate.or(facts.avg_frame_rate).ok_or_else(|| {
            error(
                "probe closed Linux camera movie",
                "frame rate is unavailable",
            )
        })?;
        Ok(MeasuredCameraMediaFacts {
            width: facts
                .width
                .ok_or_else(|| error("probe closed Linux camera movie", "width is unavailable"))?,
            height: facts
                .height
                .ok_or_else(|| error("probe closed Linux camera movie", "height is unavailable"))?,
            fps_num: u32::try_from(rate.num).map_err(|_| {
                error(
                    "probe closed Linux camera movie",
                    "FPS numerator is invalid",
                )
            })?,
            fps_den: u32::try_from(rate.den).map_err(|_| {
                error(
                    "probe closed Linux camera movie",
                    "FPS denominator is invalid",
                )
            })?,
            frame_count: facts.decoded_video_frames,
            duration_ms: facts.duration_ms,
        })
    }
}

pub(super) fn artifact_id(capture_dir: &Path, terminal: CameraTerminalState) -> String {
    let mut hash = Sha256::new();
    hash.update(b"shellx-cut/linux-camera-artifact/v1\0");
    hash.update(capture_dir.as_os_str().as_bytes());
    hash.update(format!("{terminal:?}").as_bytes());
    format!("linux_camera_{:x}", hash.finalize())
}
