//! Linux directory-handle operations for race-resistant camera finalization.

use std::ffi::CStr;
use std::path::Path;

use record_core::Result;

use crate::camera_finalization_error::finalization_error;
#[cfg(target_os = "linux")]
use crate::camera_finalization_identity::from_opened_regular;
use crate::camera_finalization_identity::VerifiedRegularFile;
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectoryIdentity;

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub(crate) struct AnchoredDirectory {
    file: File,
    identity: DirectoryIdentity,
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub(crate) struct AnchoredDirectory;

impl AnchoredDirectory {
    pub(crate) fn identity(&self) -> &DirectoryIdentity {
        #[cfg(target_os = "linux")]
        {
            &self.identity
        }
        #[cfg(not(target_os = "linux"))]
        {
            static IDENTITY: DirectoryIdentity = DirectoryIdentity;
            &IDENTITY
        }
    }

    pub(crate) fn try_clone(&self) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let file = self.file.try_clone().map_err(|error| {
                finalization_error(
                    "camera directory handle cannot be duplicated",
                    &error.to_string(),
                )
            })?;
            Ok(Self {
                file,
                identity: self.identity.clone(),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self;
            Err(unsupported())
        }
    }

    pub(crate) fn device(&self) -> u64 {
        #[cfg(target_os = "linux")]
        {
            self.identity.device
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self;
            0
        }
    }

    pub(crate) fn sync_directory(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.file.sync_all().map_err(|error| {
                finalization_error(
                    "camera publication directory cannot be synchronized",
                    &error.to_string(),
                )
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self;
            Err(unsupported())
        }
    }
}

#[cfg_attr(
    not(all(test, target_os = "linux")),
    allow(
        dead_code,
        reason = "Linux-only anchored finalization awaits a reviewed camera adapter"
    )
)]
pub(crate) fn open_capture_dir(path: &Path) -> Result<AnchoredDirectory> {
    #[cfg(target_os = "linux")]
    {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            finalization_error(
                "camera capture directory contains a NUL byte",
                "capture-owned directory handles require one native path",
            )
        })?;
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        file_from_fd(fd, "camera capture directory")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(unsupported())
    }
}

/// Make an owned finalization descriptor read-only at both the descriptor and
/// inode-mode layers. A retained writable descriptor is an explicit violation
/// of `CameraNativeCloser`'s unsafe trusted-closer contract.
pub(crate) fn enforce_non_writable_mode(file: &File) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        require_read_only_descriptor(file)?;
        let metadata = file.metadata().map_err(|error| {
            finalization_error("camera media mode cannot be inspected", &error.to_string())
        })?;
        let mode = metadata.mode() & 0o7777 & !0o222;
        let changed = unsafe { libc::fchmod(file.as_raw_fd(), mode) };
        if changed != 0 {
            return Err(finalization_error(
                "camera media cannot be made non-writable",
                &std::io::Error::last_os_error().to_string(),
            ));
        }
        ensure_non_writable_mode(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        Err(unsupported())
    }
}

pub(crate) fn ensure_non_writable_mode(file: &File) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let metadata = file.metadata().map_err(|error| {
            finalization_error("camera media mode cannot be rechecked", &error.to_string())
        })?;
        if metadata.mode() & 0o222 != 0 {
            return Err(finalization_error(
                "camera published media remains writable",
                "sealed media must have no writable inode-mode bits",
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        Err(unsupported())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn require_read_only_descriptor(file: &File) -> Result<()> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(finalization_error(
            "camera stage descriptor flags cannot be read",
            &std::io::Error::last_os_error().to_string(),
        ));
    }
    if flags & libc::O_ACCMODE != libc::O_RDONLY {
        return Err(finalization_error(
            "camera closed stage descriptor is writable",
            "native close must return a read-only descriptor after retiring every writer",
        ));
    }
    Ok(())
}

pub(crate) fn open_child_dir(
    parent: &AnchoredDirectory,
    name: &CStr,
    label: &str,
) -> Result<AnchoredDirectory> {
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe {
            libc::openat(
                parent.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let child = file_from_fd(fd, label)?;
        if child.device() != parent.device() {
            return Err(finalization_error(
                "camera directory changed filesystems",
                "capture-owned publication requires one anchored filesystem",
            ));
        }
        Ok(child)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (parent, name, label);
        Err(unsupported())
    }
}

pub(crate) fn open_regular_at(
    parent: &AnchoredDirectory,
    name: &CStr,
) -> Result<VerifiedRegularFile> {
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe {
            libc::openat(
                parent.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let file = regular_file_from_fd(fd)?;
        from_opened_regular(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (parent, name);
        Err(unsupported())
    }
}

/// Link the exact nameless file descriptor, never a stage pathname. `/proc`
/// resolves this process's descriptor to the inode itself, while `newdirfd`
/// keeps the publication leaf anchored under the captured directory handle.
pub(crate) fn link_nameless_no_replace(
    source: &VerifiedRegularFile,
    destination_parent: &AnchoredDirectory,
    destination_name: &CStr,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let source = CString::new(format!("/proc/self/fd/{}", source.raw_fd())).map_err(|_| {
            finalization_error(
                "camera staged descriptor cannot be published",
                "the descriptor path must not contain a NUL byte",
            )
        })?;
        let linked = unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                source.as_ptr(),
                destination_parent.file.as_raw_fd(),
                destination_name.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if linked == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        let message = if error.kind() == std::io::ErrorKind::AlreadyExists {
            "camera publication destination already exists"
        } else {
            "camera staged media could not be published from its exact descriptor"
        };
        Err(finalization_error(message, &error.to_string()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source, destination_parent, destination_name);
        Err(unsupported())
    }
}

#[cfg(target_os = "linux")]
fn file_from_fd(fd: i32, label: &str) -> Result<AnchoredDirectory> {
    if fd < 0 {
        return Err(finalization_error(
            &format!("{label} cannot be opened without following links"),
            &std::io::Error::last_os_error().to_string(),
        ));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|error| {
        finalization_error(&format!("{label} cannot be inspected"), &error.to_string())
    })?;
    if !metadata.is_dir() {
        return Err(finalization_error(
            &format!("{label} is not a directory"),
            "anchored camera containment requires direct directory handles",
        ));
    }
    Ok(AnchoredDirectory {
        file,
        identity: DirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        },
    })
}

#[cfg(target_os = "linux")]
fn regular_file_from_fd(fd: i32) -> Result<File> {
    if fd < 0 {
        return Err(finalization_error(
            "camera publication destination cannot be opened without following links",
            &std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub(crate) fn same_directory(left: &AnchoredDirectory, right: &DirectoryIdentity) -> bool {
    left.identity() == right
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> record_core::RecordError {
    finalization_error(
        "camera anchored finalization is unavailable on this host",
        "finalization fails closed without Linux directory-handle and descriptor-link primitives",
    )
}
