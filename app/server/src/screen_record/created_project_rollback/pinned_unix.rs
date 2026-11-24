//! Unix (`linux` and `macos`) pinned directory-handle operations with
//! `O_NOFOLLOW` on every path component and `openat`/`unlinkat` execution.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirIdentity {
    pub(super) dev: u64,
    pub(super) ino: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileIdentity {
    pub(super) dev: u64,
    pub(super) ino: u64,
}

pub(super) struct PinnedDir {
    fd: OwnedFd,
    pub(super) identity: DirIdentity,
}

pub(super) struct PinnedFile {
    file: File,
    pub(super) identity: FileIdentity,
}

struct DirStream(*mut libc::DIR);

impl Drop for DirStream {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` came from a successful `fdopendir`.
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
}

fn c_path_bytes(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

pub(super) fn validate_single_component(name: &str) -> io::Result<CString> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a single safe directory entry name",
        ));
    }
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name contains NUL"))
}

fn fstat_fd(fd: libc::c_int) -> io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: valid open descriptor and owned output stat struct.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fstat` succeeded and populated `stat`.
    Ok(unsafe { stat.assume_init() })
}

#[allow(clippy::unnecessary_cast)]
fn dir_identity_from_stat(stat: &libc::stat) -> io::Result<DirIdentity> {
    if (stat.st_mode as u32 & libc::S_IFMT as u32) != (libc::S_IFDIR as u32) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "descriptor is not a directory",
        ));
    }
    Ok(DirIdentity {
        dev: stat.st_dev as u64,
        ino: stat.st_ino as u64,
    })
}

#[allow(clippy::unnecessary_cast)]
fn file_identity_from_stat(
    stat: &libc::stat,
    require_single_link: bool,
) -> io::Result<FileIdentity> {
    if (stat.st_mode as u32 & libc::S_IFMT as u32) != (libc::S_IFREG as u32) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "descriptor is not a regular file",
        ));
    }
    if require_single_link && (stat.st_nlink as u64) != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "regular file has unexpected hard links",
        ));
    }
    Ok(FileIdentity {
        dev: stat.st_dev as u64,
        ino: stat.st_ino as u64,
    })
}

pub(super) fn open_root_dir_nofollow(path: &Path) -> io::Result<PinnedDir> {
    let c_path = c_path_bytes(path)?;
    // SAFETY: CString is NUL-terminated; O_DIRECTORY | O_NOFOLLOW forbids symlinks on the root dir itself.
    let raw = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a newly opened owned descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let identity = dir_identity_from_stat(&fstat_fd(fd.as_raw_fd())?)?;
    Ok(PinnedDir { fd, identity })
}

pub(super) fn open_child_dir_nofollow(parent: &PinnedDir, name: &str) -> io::Result<PinnedDir> {
    let c_name = validate_single_component(name)?;
    // SAFETY: valid parent directory fd and NUL-terminated single component; no links are followed.
    let raw = unsafe {
        libc::openat(
            parent.fd.as_raw_fd(),
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a newly opened owned descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let identity = dir_identity_from_stat(&fstat_fd(fd.as_raw_fd())?)?;
    Ok(PinnedDir { fd, identity })
}

pub(super) fn open_child_file_nofollow(parent: &PinnedDir, name: &str) -> io::Result<PinnedFile> {
    let c_name = validate_single_component(name)?;
    // SAFETY: valid parent directory fd and NUL-terminated single component; O_NOFOLLOW forbids symlinks.
    let raw = unsafe {
        libc::openat(
            parent.fd.as_raw_fd(),
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a newly opened owned file descriptor.
    let file = unsafe { File::from_raw_fd(raw) };
    let identity = file_identity_from_stat(&fstat_fd(file.as_raw_fd())?, true)?;
    Ok(PinnedFile { file, identity })
}

pub(super) fn verify_dir_handle(dir: &PinnedDir) -> io::Result<()> {
    let current = dir_identity_from_stat(&fstat_fd(dir.fd.as_raw_fd())?)?;
    if current != dir.identity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory handle identity mismatch",
        ));
    }
    Ok(())
}

pub(super) fn open_verified_child_file_nofollow(
    parent: &PinnedDir,
    name: &str,
    expected: FileIdentity,
) -> io::Result<PinnedFile> {
    let file = open_child_file_nofollow(parent, name)?;
    if file.identity != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "child file identity mismatch",
        ));
    }
    Ok(file)
}

pub(super) fn open_verified_child_dir_nofollow(
    parent: &PinnedDir,
    name: &str,
    expected: DirIdentity,
) -> io::Result<PinnedDir> {
    let dir = open_child_dir_nofollow(parent, name)?;
    if dir.identity != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "child directory identity mismatch",
        ));
    }
    Ok(dir)
}

fn clear_errno() {
    #[cfg(target_os = "linux")]
    // SAFETY: thread-local errno pointer on Linux.
    unsafe {
        *libc::__errno_location() = 0;
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    // SAFETY: thread-local errno pointer on macOS/BSD.
    unsafe {
        *libc::__error() = 0;
    }
}

pub(super) fn list_child_names(dir: &PinnedDir) -> io::Result<Vec<String>> {
    // SAFETY: dup the pinned directory descriptor so `fdopendir`/`closedir` owns only the dup.
    let dup_fd = unsafe { libc::fcntl(dir.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if dup_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fdopendir` takes ownership of `dup_fd` on success.
    let raw_dir = unsafe { libc::fdopendir(dup_fd) };
    if raw_dir.is_null() {
        let err = io::Error::last_os_error();
        // SAFETY: `fdopendir` failed so `dup_fd` is still owned here.
        unsafe { libc::close(dup_fd) };
        return Err(err);
    }
    let stream = DirStream(raw_dir);
    // SAFETY: `stream.0` is an open `DIR*`.
    unsafe { libc::rewinddir(stream.0) };
    let mut names = Vec::new();
    loop {
        clear_errno();
        // SAFETY: `stream.0` is valid and non-null.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let err = io::Error::last_os_error();
            if err.raw_os_error().unwrap_or(0) != 0 {
                return Err(err);
            }
            break;
        }
        // SAFETY: `d_name` in `dirent` is NUL-terminated.
        let c_name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let name = c_name
            .to_str()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 entry name"))?;
        if name != "." && name != ".." {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

pub(super) fn read_pinned_file_bounded(
    file: &mut PinnedFile,
    max_bytes: usize,
) -> io::Result<Vec<u8>> {
    file.file.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    (&file.file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut buf)?;
    if buf.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeded bounded verification limit",
        ));
    }
    Ok(buf)
}

pub(super) fn unlink_verified_child_file_at(
    parent: &PinnedDir,
    name: &str,
    expected: Option<FileIdentity>,
) -> io::Result<()> {
    let current = open_child_file_nofollow(parent, name)?;
    if let Some(id) = expected {
        if current.identity != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "child file identity mismatch before unlink",
            ));
        }
    }
    drop(current);
    let c_name = validate_single_component(name)?;
    // SAFETY: valid parent directory fd and NUL-terminated component; flags=0 unlinks only non-dir entries.
    if unsafe { libc::unlinkat(parent.fd.as_raw_fd(), c_name.as_ptr(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn unlink_verified_child_dir_at(
    parent: &PinnedDir,
    name: &str,
    expected: Option<DirIdentity>,
) -> io::Result<()> {
    let current = open_child_dir_nofollow(parent, name)?;
    if let Some(id) = expected {
        if current.identity != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "child directory identity mismatch before rmdir",
            ));
        }
    }
    drop(current);
    let c_name = validate_single_component(name)?;
    // SAFETY: AT_REMOVEDIR removes only an empty direct-child directory and never traverses symlinks.
    if unsafe { libc::unlinkat(parent.fd.as_raw_fd(), c_name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn unlink_root_project_dir(
    root: &PinnedDir,
    project_dir: PinnedDir,
    project_dir_name: &str,
) -> io::Result<()> {
    let expected = project_dir.identity;
    let _current = open_verified_child_dir_nofollow(root, project_dir_name, expected)?;
    verify_dir_handle(&project_dir)?;
    drop(_current);
    drop(project_dir);
    let c_name = validate_single_component(project_dir_name)?;
    // SAFETY: AT_REMOVEDIR removes only the verified empty direct-child directory and never follows symlinks.
    if unsafe { libc::unlinkat(root.fd.as_raw_fd(), c_name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
