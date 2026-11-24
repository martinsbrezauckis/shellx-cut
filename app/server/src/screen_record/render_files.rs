//! Directory-anchored, no-follow access for recording-render retirement.
//! Never use a project-provided path as a deletion root.
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs::File;
use std::{
    io,
    path::{Component, Path},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Identity {
    pub bytes: u64,
    stamp: Vec<u64>,
}

#[cfg(unix)]
fn identity(file: &File) -> io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    if !m.is_file() || m.nlink() != 1 {
        return Err(io::Error::other("not a regular render file"));
    }
    Ok(Identity {
        bytes: m.len(),
        stamp: vec![
            m.dev(),
            m.ino(),
            m.mtime() as u64,
            m.mtime_nsec() as u64,
            m.ctime() as u64,
            m.ctime_nsec() as u64,
        ],
    })
}

#[cfg(unix)]
fn directory(path: &Path) -> io::Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    if !path.is_absolute() {
        return Err(io::Error::other("render root is not absolute"));
    }
    let mut current = File::open("/")?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            if component == Component::RootDir {
                continue;
            }
            return Err(io::Error::other("non-normal render root"));
        };
        let name = CString::new(name.as_bytes())?;
        // SAFETY: name is terminated; current remains alive through openat.
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a newly owned descriptor.
        current = unsafe { File::from_raw_fd(fd) };
    }
    Ok(current)
}

fn leaf(path: &Path) -> io::Result<&std::ffi::OsStr> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing render name"))?;
    if Path::new(name).components().count() != 1 {
        return Err(io::Error::other("invalid render name"));
    }
    Ok(name)
}

/// Inspect or unlink one exact engine-origin file. Every parent is opened with
/// no-follow; Unix unlink is relative to the pinned directory, never the path.
/// The expected identity comes from the local origin record, not the project.
#[cfg(unix)]
pub(super) fn access(
    path: &Path,
    expected: Option<&Identity>,
    remove: bool,
) -> io::Result<Identity> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let parent = directory(
        path.parent()
            .ok_or_else(|| io::Error::other("missing render folder"))?,
    )?;
    let name = CString::new(leaf(path)?.as_bytes())?;
    // O_NONBLOCK prevents a planted FIFO from hanging inventory.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let current = identity(&file)?;
    if expected.is_some_and(|expected| *expected != current) {
        return Err(io::Error::other("render changed since it was made"));
    }
    if remove {
        if expected.is_none() {
            return Err(io::Error::other("render ownership is missing"));
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(current)
}

/// On Windows pin every ancestor against rename/deletion and reject name-surrogate
/// reparse points on the opened handles. Delete the exact opened leaf, not its path.
#[cfg(windows)]
pub(super) fn access(
    path: &Path,
    expected: Option<&Identity>,
    remove: bool,
) -> io::Result<Identity> {
    use std::{
        fs::OpenOptions,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::PathBuf,
    };
    use windows_sys::Win32::Storage::FileSystem::*;
    if !path.is_absolute() {
        return Err(io::Error::other("render path is not absolute"));
    }
    let mut pinned = Vec::new();
    let mut prefix = PathBuf::new();
    for component in path
        .parent()
        .ok_or_else(|| io::Error::other("missing render folder"))?
        .components()
    {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                prefix.push(component);
                continue;
            }
            Component::Normal(_) => prefix.push(component),
            _ => return Err(io::Error::other("non-normal render path")),
        }
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&prefix)?;
        let metadata = file.metadata()?;
        let not_surrogate = cut_core::reparse::handle_reparse_tag(&file)
            .is_ok_and(|tag| !tag.is_some_and(cut_core::reparse::is_name_surrogate_tag));
        if !metadata.is_dir() || !not_surrogate {
            return Err(io::Error::other("linked render parent"));
        }
        pinned.push(file);
    }
    let _ = leaf(path)?;
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | if remove { 0x00010000 } else { 0 })
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let not_surrogate = cut_core::reparse::handle_reparse_tag(&file)
        .is_ok_and(|tag| !tag.is_some_and(cut_core::reparse::is_name_surrogate_tag));
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if !not_surrogate
        || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
        || info.nNumberOfLinks != 1
    {
        return Err(io::Error::other("not a plain render file"));
    }
    let current = Identity {
        bytes: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        stamp: vec![
            info.dwVolumeSerialNumber as u64,
            info.nFileIndexHigh as u64,
            info.nFileIndexLow as u64,
            info.ftLastWriteTime.dwHighDateTime as u64,
            info.ftLastWriteTime.dwLowDateTime as u64,
        ],
    };
    if expected.is_some_and(|expected| *expected != current) {
        return Err(io::Error::other("render changed since it was made"));
    }
    if remove {
        if expected.is_none() {
            return Err(io::Error::other("render ownership is missing"));
        }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle() as _,
                FileDispositionInfo,
                &disposition as *const _ as _,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(current)
}
