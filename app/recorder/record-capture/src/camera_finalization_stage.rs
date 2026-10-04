//! Linux camera stage creation under an already verified directory handle.

use std::ffi::CStr;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};

use record_core::Result;

use crate::camera_finalization_anchored::{open_child_dir, AnchoredDirectory};
use crate::camera_finalization_error::finalization_error;

#[cfg(any(test, feature = "capture-linux"))]
impl AnchoredDirectory {
    pub(crate) fn ensure_plain_child(&self, name: &CStr) -> Result<Self> {
        let created = unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) };
        if created != 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(finalization_error(
                "camera child directory cannot be reserved",
                &std::io::Error::last_os_error().to_string(),
            ));
        }
        open_child_dir(self, name, "camera output directory")
    }

    pub(crate) fn nameless_writable_stage(&self) -> Result<File> {
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c".".as_ptr(),
                libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(finalization_error(
                "camera nameless stage cannot be reserved",
                &std::io::Error::last_os_error().to_string(),
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
