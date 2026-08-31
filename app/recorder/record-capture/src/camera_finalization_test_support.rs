#[cfg(target_os = "linux")]
use std::cell::Cell;
#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::{Seek, SeekFrom, Write};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
use record_core::{RecordError, Result};

#[cfg(target_os = "linux")]
use crate::camera_finalization::{
    CameraMediaProbe, CameraNativeCloser, DeclaredCameraMediaFacts, MeasuredCameraMediaFacts,
    StagedCameraMedia,
};
#[cfg(target_os = "linux")]
use crate::camera_finalization_owner::CameraCaptureDirectory;

#[cfg(target_os = "linux")]
pub(crate) struct FixtureCloser {
    pub(crate) closed: bool,
    pub(crate) fail_close: bool,
    stage: Option<File>,
}

#[cfg(target_os = "linux")]
impl FixtureCloser {
    pub(crate) fn with_stage(root: &Path) -> Self {
        let (stage, writer) = nameless_stage(root);
        drop(writer);
        Self {
            closed: false,
            fail_close: false,
            stage: Some(stage),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            closed: false,
            fail_close: true,
            stage: None,
        }
    }
}

#[cfg(target_os = "linux")]
unsafe impl CameraNativeCloser for FixtureCloser {
    fn close_media(&mut self) -> Result<File> {
        self.closed = true;
        if self.fail_close {
            return Err(RecordError::new(
                "capture",
                "native camera writer did not close",
                "fixture close failure",
            ));
        }
        self.stage.take().ok_or_else(|| {
            RecordError::new(
                "capture",
                "native camera writer did not return a stage",
                "fixture stage was already consumed",
            )
        })
    }
}

#[cfg(target_os = "linux")]
pub(crate) struct FixtureProbe {
    facts: MeasuredCameraMediaFacts,
    pub(crate) calls: Cell<usize>,
}

#[cfg(target_os = "linux")]
impl FixtureProbe {
    pub(crate) fn new(facts: MeasuredCameraMediaFacts) -> Self {
        Self {
            facts,
            calls: Cell::new(0),
        }
    }
}

#[cfg(target_os = "linux")]
impl CameraMediaProbe for FixtureProbe {
    fn measure(&self, _closed_file: &File) -> Result<MeasuredCameraMediaFacts> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.facts.clone())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn facts() -> MeasuredCameraMediaFacts {
    MeasuredCameraMediaFacts {
        width: 640,
        height: 480,
        fps_num: 30,
        fps_den: 1,
        frame_count: 30,
        duration_ms: 1_000,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn declaration() -> DeclaredCameraMediaFacts {
    DeclaredCameraMediaFacts {
        width: 640,
        height: 480,
        fps_num: 30,
        fps_den: 1,
        frame_count: 30,
        duration_ms: 1_000,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn staged(root: &Path) -> StagedCameraMedia {
    std::fs::create_dir(root.join("camera")).unwrap();
    std::fs::create_dir(root.join(".camera-staging")).unwrap();
    StagedCameraMedia {
        artifact_id: "camera_01".into(),
        video: "camera/camera.mp4".into(),
        declared: declaration(),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn owner(root: &Path) -> CameraCaptureDirectory {
    CameraCaptureDirectory::reserve(root).unwrap()
}

#[cfg(target_os = "linux")]
pub(crate) fn nameless_stage(root: &Path) -> (File, File) {
    let staging_dir = CString::new(root.join(".camera-staging").as_os_str().as_bytes()).unwrap();
    let fd = unsafe {
        libc::open(
            staging_dir.as_ptr(),
            libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
            0o600,
        )
    };
    assert!(
        fd >= 0,
        "O_TMPFILE fixture: {}",
        std::io::Error::last_os_error()
    );
    let mut writer = unsafe { File::from_raw_fd(fd) };
    writer.write_all(b"closed camera media").unwrap();
    writer.sync_all().unwrap();
    writer.seek(SeekFrom::Start(0)).unwrap();
    let read_only = File::open(format!("/proc/self/fd/{}", writer.as_raw_fd())).unwrap();
    (read_only, writer)
}

#[cfg(target_os = "linux")]
pub(crate) fn rewrite_stage(writer: &mut File) {
    writer.set_len(0).unwrap();
    writer.seek(SeekFrom::Start(0)).unwrap();
    writer.write_all(b"mutated camera data").unwrap();
    writer.sync_all().unwrap();
}
