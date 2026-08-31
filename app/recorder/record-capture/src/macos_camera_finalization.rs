//! macOS no-replace camera publication after AVFoundation closes its writer.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use record_core::{error_codes, CameraMediaFacts, RecordError, Result};
use sha2::{Digest, Sha256};

use crate::camera_finalization::CameraMediaSeal;

pub(super) struct MacCameraStage {
    capture_dir: PathBuf,
    stage_path: PathBuf,
    final_path: PathBuf,
    artifact_id: String,
    video: String,
    published: bool,
}

impl MacCameraStage {
    pub(super) fn prepare(capture_dir: &Path, capture_id: &str) -> Result<Self> {
        if !record_recovery::is_plain_dir(capture_dir).unwrap_or(false) {
            return Err(error(
                "reserve macOS camera output",
                "capture directory is not a plain local directory",
            ));
        }
        let camera_dir = ensure_child_dir(capture_dir, "camera")?;
        let staging_dir = ensure_child_dir(capture_dir, ".camera-staging")?;
        let artifact_id = artifact_id(capture_id);
        let leaf = format!("{artifact_id}.mp4");
        let stage_path = staging_dir.join(&leaf);
        let final_path = camera_dir.join(&leaf);
        if fs::symlink_metadata(&stage_path).is_ok() || fs::symlink_metadata(&final_path).is_ok() {
            return Err(error(
                "reserve macOS camera output",
                "camera output already exists",
            ));
        }
        Ok(Self {
            capture_dir: capture_dir.to_path_buf(),
            stage_path,
            final_path,
            artifact_id,
            video: format!("camera/{leaf}"),
            published: false,
        })
    }

    pub(super) fn stage_path(&self) -> &Path {
        &self.stage_path
    }

    pub(super) fn finalize(mut self) -> Result<CameraMediaSeal> {
        let before = plain_file(&self.stage_path)?;
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
        let facts = record_recovery::verify_media(&ffmpeg, &ffprobe, &self.stage_path)
            .map_err(|cause| error("verify macOS camera output", &cause.to_string()))?;
        if facts.has_audio {
            return Err(error(
                "verify macOS camera output",
                "camera sidecar unexpectedly contains audio",
            ));
        }
        let width = facts
            .width
            .ok_or_else(|| error("verify macOS camera output", "camera width is unavailable"))?;
        let height = facts
            .height
            .ok_or_else(|| error("verify macOS camera output", "camera height is unavailable"))?;
        let rate = facts.avg_frame_rate.or(facts.r_frame_rate).ok_or_else(|| {
            error(
                "verify macOS camera output",
                "camera frame rate is unavailable",
            )
        })?;
        let fps_num = u32::try_from(rate.num).map_err(|_| {
            error(
                "verify macOS camera output",
                "camera FPS numerator is too large",
            )
        })?;
        let fps_den = u32::try_from(rate.den).map_err(|_| {
            error(
                "verify macOS camera output",
                "camera FPS denominator is too large",
            )
        })?;
        let (sha256, bytes) = hash_file(&self.stage_path)?;
        if before.len() != bytes || bytes == 0 {
            return Err(error(
                "verify macOS camera output",
                "camera output changed during verification",
            ));
        }
        let mut permissions = before.permissions();
        permissions.set_mode(0o444);
        fs::set_permissions(&self.stage_path, permissions)
            .map_err(|cause| error("protect macOS camera output", &cause.to_string()))?;
        File::open(&self.stage_path)
            .and_then(|file| file.sync_all())
            .map_err(|cause| error("sync macOS camera output", &cause.to_string()))?;
        publish_no_replace(&self.stage_path, &self.final_path)?;
        self.published = true;
        sync_dir(self.final_path.parent().expect("camera leaf has a parent"))?;
        sync_dir(&self.capture_dir)?;
        if !record_recovery::is_plain_regular_file(&self.final_path).unwrap_or(false) {
            return Err(error(
                "verify published macOS camera output",
                "published camera leaf is not a plain file",
            ));
        }
        let (published_sha256, published_bytes) = hash_file(&self.final_path)?;
        if (published_sha256.as_str(), published_bytes) != (sha256.as_str(), bytes) {
            return Err(error(
                "verify published macOS camera output",
                "published camera bytes changed",
            ));
        }
        CameraMediaSeal::verified_publication(
            self.artifact_id.clone(),
            self.video.clone(),
            CameraMediaFacts {
                width,
                height,
                fps_num,
                fps_den,
                frame_count: facts.decoded_video_frames,
                duration_ms: facts.duration_ms,
                sha256,
            },
            bytes,
        )
    }
}

impl Drop for MacCameraStage {
    fn drop(&mut self) {
        if !self.published
            && record_recovery::is_plain_regular_file(&self.stage_path).unwrap_or(false)
        {
            let _ = fs::remove_file(&self.stage_path);
        }
    }
}

fn ensure_child_dir(parent: &Path, name: &str) -> Result<PathBuf> {
    let path = parent.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(cause) => return Err(error("create macOS camera directory", &cause.to_string())),
    }
    if !record_recovery::is_plain_dir(&path).unwrap_or(false) {
        return Err(error(
            "create macOS camera directory",
            "camera directory is linked or not a directory",
        ));
    }
    Ok(path)
}

fn artifact_id(capture_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"shellx-cut/macos-camera-artifact/v1\0");
    digest.update(capture_id.as_bytes());
    format!("camera_{}", &format!("{:x}", digest.finalize())[..32])
}

fn plain_file(path: &Path) -> Result<fs::Metadata> {
    if !record_recovery::is_plain_regular_file(path).unwrap_or(false) {
        return Err(error(
            "open macOS camera output",
            "camera stage is linked or not a regular file",
        ));
    }
    fs::metadata(path).map_err(|cause| error("open macOS camera output", &cause.to_string()))
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|cause| error("hash macOS camera output", &cause.to_string()))?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|cause| error("hash macOS camera output", &cause.to_string()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| error("hash macOS camera output", "camera size overflowed"))?;
    }
    Ok((format!("{:x}", digest.finalize()), bytes))
}

fn publish_no_replace(source: &Path, destination: &Path) -> Result<()> {
    let source = CString::new(source.as_os_str().as_encoded_bytes()).map_err(|_| {
        error(
            "publish macOS camera output",
            "stage path contains a NUL byte",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_encoded_bytes()).map_err(|_| {
        error(
            "publish macOS camera output",
            "destination path contains a NUL byte",
        )
    })?;
    let code =
        unsafe { sxc_macos_camera_publish_no_replace(source.as_ptr(), destination.as_ptr()) };
    if code == 0 {
        Ok(())
    } else {
        Err(error(
            "publish macOS camera output",
            &format!("renameatx_np failed with errno {code}"),
        ))
    }
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|cause| error("sync macOS camera directory", &cause.to_string()))
}

fn error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
}

unsafe extern "C" {
    fn sxc_macos_camera_publish_no_replace(
        source: *const std::ffi::c_char,
        destination: *const std::ffi::c_char,
    ) -> i32;
}
