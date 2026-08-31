//! Opaque capture-directory authority retained from reservation to sealing.

use std::ffi::CString;
use std::path::Path;

use record_core::Result;

use crate::camera_finalization_anchored::{
    open_capture_dir, open_child_dir, same_directory, AnchoredDirectory, DirectoryIdentity,
};
use crate::camera_finalization_error::finalization_error;

/// A capture owner is acquired while the capture directory is reserved, not
/// reconstructed from a caller path during finalization. Its parent descriptor
/// and child identity make later root replacement a closed failure.
#[derive(Debug)]
pub(crate) struct CameraCaptureDirectory {
    parent: AnchoredDirectory,
    name: CString,
    identity: DirectoryIdentity,
}

impl CameraCaptureDirectory {
    #[cfg_attr(
        not(all(test, target_os = "linux")),
        allow(
            dead_code,
            reason = "the Linux anchored finalizer is exercised by its native fault harness until a reviewed camera adapter owns reservation"
        )
    )]
    pub(crate) fn reserve(capture_dir: &Path) -> Result<Self> {
        let parent_path = capture_dir.parent().ok_or_else(|| {
            finalization_error(
                "camera capture directory has no parent",
                "capture reservation requires one owned parent directory",
            )
        })?;
        let name = capture_dir
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or_else(|| {
                finalization_error(
                    "camera capture directory has no Unicode name",
                    "capture reservation requires one literal native directory component",
                )
            })?;
        let name = CString::new(name).map_err(|_| {
            finalization_error(
                "camera capture directory name contains a NUL byte",
                "capture reservation requires one literal native directory component",
            )
        })?;
        let parent = open_capture_dir(parent_path)?;
        let capture = open_child_dir(&parent, &name, "camera capture directory")?;
        Ok(Self {
            parent,
            name,
            identity: capture.identity().clone(),
        })
    }

    pub(crate) fn capture_dir(&self) -> Result<AnchoredDirectory> {
        let capture = open_child_dir(&self.parent, &self.name, "camera capture directory")?;
        if !same_directory(&capture, &self.identity) {
            return Err(finalization_error(
                "camera capture directory identity changed",
                "the reserved capture directory was replaced before finalization",
            ));
        }
        Ok(capture)
    }
}
