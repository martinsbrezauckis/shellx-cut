//! Capture-owned destination containment through anchored directory handles.

use std::ffi::{CStr, CString};
use std::path::{Component, Path};

use record_core::Result;

use crate::camera_finalization_anchored::{
    open_child_dir, same_directory, AnchoredDirectory, DirectoryIdentity,
};
use crate::camera_finalization_error::finalization_error;
use crate::camera_finalization_owner::CameraCaptureDirectory;

#[derive(Debug)]
struct CapturedDirectory {
    name: CString,
    identity: DirectoryIdentity,
}

#[derive(Debug)]
pub(crate) struct CameraFinalizationPaths<'owner> {
    owner: &'owner CameraCaptureDirectory,
    capture: AnchoredDirectory,
    route: Vec<CapturedDirectory>,
    destination_name: CString,
}

/// Resolve a declared relative video only through file-descriptor anchored
/// directories. Once this returns, future opens and links never re-resolve the
/// caller's capture path or a string-built parent path.
pub(crate) fn resolve_capture_owned_paths<'owner>(
    owner: &'owner CameraCaptureDirectory,
    video: &str,
) -> Result<CameraFinalizationPaths<'owner>> {
    let relative = clean_relative_path(video, "camera publication video path")?;
    let destination_name = component_name(relative.file_name(), "camera publication video name")?;
    let capture = owner.capture_dir()?;
    let mut current = capture.try_clone()?;
    let mut route = Vec::new();
    for component in relative.parent().into_iter().flat_map(Path::components) {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = component_name(Some(name), "camera publication parent directory")?;
        let next = open_child_dir(&current, &name, "camera publication parent directory")?;
        route.push(CapturedDirectory {
            name,
            identity: next.identity().clone(),
        });
        current = next;
    }
    Ok(CameraFinalizationPaths {
        owner,
        capture,
        route,
        destination_name,
    })
}

impl CameraFinalizationPaths<'_> {
    pub(crate) fn destination_name(&self) -> &CStr {
        &self.destination_name
    }

    /// Reopen the declared route beneath the original capture directory
    /// handle. Every component must still be the directory originally opened;
    /// a rename/symlink/junction swap fails before publication or sealing.
    pub(crate) fn destination_parent(&self) -> Result<AnchoredDirectory> {
        let mut current = self.revalidate_owner()?;
        for captured in &self.route {
            let next = open_child_dir(
                &current,
                &captured.name,
                "camera publication parent directory",
            )?;
            if !same_directory(&next, &captured.identity) {
                return Err(finalization_error(
                    "camera publication parent identity changed",
                    "the capture-contained directory route was swapped after anchoring",
                ));
            }
            current = next;
        }
        Ok(current)
    }

    pub(crate) fn capture_filesystem(&self) -> u64 {
        self.capture.device()
    }

    /// Reopen the reserved capture root through its owner rather than relying
    /// only on the stale descriptor captured at initial path anchoring.
    pub(crate) fn revalidate_owner(&self) -> Result<AnchoredDirectory> {
        let capture = self.owner.capture_dir()?;
        if !same_directory(&capture, self.capture.identity()) {
            return Err(finalization_error(
                "camera capture directory identity changed",
                "the reserved capture directory changed after finalization anchoring",
            ));
        }
        Ok(capture)
    }
}

fn component_name(value: Option<&std::ffi::OsStr>, label: &str) -> Result<CString> {
    let value = value.and_then(std::ffi::OsStr::to_str).ok_or_else(|| {
        finalization_error(
            &format!("{label} is not a Unicode path component"),
            "camera private contracts use validated UTF-8 relative video names",
        )
    })?;
    CString::new(value).map_err(|_| {
        finalization_error(
            &format!("{label} contains a NUL byte"),
            "camera media paths must be one native component",
        )
    })
}

fn clean_relative_path<'value>(value: &'value str, label: &str) -> Result<&'value Path> {
    if value.is_empty() || value.trim() != value || value.contains(['\\', ':', '\0']) {
        return Err(finalization_error(
            &format!("{label} must be a clean relative path"),
            "camera media paths are capture-contained",
        ));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::CurDir
                    | Component::ParentDir
            )
        })
    {
        return Err(finalization_error(
            &format!("{label} escapes its capture directory"),
            "camera media paths are capture-contained",
        ));
    }
    Ok(path)
}
