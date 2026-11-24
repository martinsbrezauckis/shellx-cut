//! Windows pinned directory-handle operations built on `NtOpenFile(RootDirectory, ...)`
//! and `FILE_OPEN_REPARSE_POINT` plus `SetFileInformationByHandle(FileDispositionInfo)`.

#[path = "pinned_windows_nt.rs"]
mod nt;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, OPEN_EXISTING, SYNCHRONIZE,
};

const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirIdentity {
    pub(super) volume: u32,
    pub(super) index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileIdentity {
    pub(super) volume: u32,
    pub(super) index: u64,
}

pub(super) struct PinnedDir {
    handle: OwnedHandle,
    pub(super) identity: DirIdentity,
}

pub(super) struct PinnedFile {
    file: File,
    pub(super) identity: FileIdentity,
}

pub(super) fn open_root_dir_nofollow(path: &Path) -> io::Result<PinnedDir> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: NUL-terminated wide path; `FILE_FLAG_OPEN_REPARSE_POINT` avoids following links on root.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            nt::SHARE_ALL,
            null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a newly opened owned handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw as *mut _) };
    let identity = nt::dir_identity_from_handle(handle.as_raw_handle() as HANDLE)?;
    Ok(PinnedDir { handle, identity })
}

pub(super) fn open_child_dir_nofollow(parent: &PinnedDir, name: &str) -> io::Result<PinnedDir> {
    let handle = nt::nt_open_child(
        &parent.handle,
        name,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_DIRECTORY_FILE,
    )?;
    let identity = nt::dir_identity_from_handle(handle.as_raw_handle() as HANDLE)?;
    Ok(PinnedDir { handle, identity })
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

pub(super) fn open_child_file_nofollow(parent: &PinnedDir, name: &str) -> io::Result<PinnedFile> {
    let handle = nt::nt_open_child(
        &parent.handle,
        name,
        FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_NON_DIRECTORY_FILE,
    )?;
    let identity = nt::file_identity_from_handle(handle.as_raw_handle() as HANDLE, true)?;
    let file = File::from(handle);
    Ok(PinnedFile { file, identity })
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

pub(super) fn verify_dir_handle(dir: &PinnedDir) -> io::Result<()> {
    let current = nt::dir_identity_from_handle(dir.handle.as_raw_handle() as HANDLE)?;
    if current != dir.identity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory handle identity mismatch",
        ));
    }
    Ok(())
}

pub(super) fn list_child_names(dir: &PinnedDir) -> io::Result<Vec<String>> {
    nt::list_child_names_from_handle(dir.handle.as_raw_handle() as HANDLE)
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
    let handle = nt::nt_open_child(
        &parent.handle,
        name,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_NON_DIRECTORY_FILE,
    )?;
    let id = nt::file_identity_from_handle(handle.as_raw_handle() as HANDLE, true)?;
    if let Some(expected_id) = expected {
        if id != expected_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "child file identity mismatch before unlink",
            ));
        }
    }
    nt::mark_delete_on_close(handle.as_raw_handle() as HANDLE)?;
    drop(handle);
    Ok(())
}

pub(super) fn unlink_verified_child_dir_at(
    parent: &PinnedDir,
    name: &str,
    expected: Option<DirIdentity>,
) -> io::Result<()> {
    let handle = nt::nt_open_child(
        &parent.handle,
        name,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_DIRECTORY_FILE,
    )?;
    let id = nt::dir_identity_from_handle(handle.as_raw_handle() as HANDLE)?;
    if let Some(expected_id) = expected {
        if id != expected_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "child directory identity mismatch before rmdir",
            ));
        }
    }
    nt::mark_delete_on_close(handle.as_raw_handle() as HANDLE)?;
    drop(handle);
    Ok(())
}

pub(super) fn unlink_root_project_dir(
    root: &PinnedDir,
    project_dir: PinnedDir,
    project_dir_name: &str,
) -> io::Result<()> {
    let handle = nt::nt_open_child(
        &root.handle,
        project_dir_name,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_DIRECTORY_FILE,
    )?;
    let id = nt::dir_identity_from_handle(handle.as_raw_handle() as HANDLE)?;
    if id != project_dir.identity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "project directory identity mismatch before rmdir",
        ));
    }
    drop(project_dir);
    nt::mark_delete_on_close(handle.as_raw_handle() as HANDLE)?;
    drop(handle);
    Ok(())
}
