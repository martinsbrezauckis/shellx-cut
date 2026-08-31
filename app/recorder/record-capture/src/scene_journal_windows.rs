//! Windows anchored, no-reparse journal durability backend.
//!
//! Windows lacks POSIX parent-directory fsync. This backend instead retains a
//! verified non-reparse parent handle, derives a physical fixed leaf path, and
//! acknowledges writes only after WRITE_THROUGH plus FlushFileBuffers.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::scene_journal::{SceneJournalResult, SCENE_JOURNAL_FILE};
use crate::scene_journal_io::{ensure_open_regular, invalid, io_error};

pub(super) fn open_windows_anchored(
    requested: &Path,
    create_new: bool,
    append: bool,
) -> SceneJournalResult<File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
    };

    let parent = WindowsJournalParent::open(requested)?;
    let leaf = parent.leaf_path(requested)?;
    let mut wide = leaf.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    let access = GENERIC_READ | if append { GENERIC_WRITE } else { 0 };
    // The owner handle keeps write access while `ensure_same_leaf` opens a
    // read-only identity witness. Windows validates sharing symmetrically: the
    // witness must share the writer's existing access even though the witness
    // does not request write access itself. Keep writer openings exclusive and
    // admit FILE_SHARE_WRITE only for the read-only witness.
    let share = FILE_SHARE_READ | if append { 0 } else { FILE_SHARE_WRITE };
    let disposition = if create_new {
        CREATE_NEW
    } else {
        OPEN_EXISTING
    };
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            share,
            std::ptr::null(),
            disposition,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io_error(requested, std::io::Error::last_os_error()));
    }
    // SAFETY: CreateFileW returned one owned journal leaf handle.
    let file = unsafe { File::from_raw_handle(handle as *mut _) };
    parent.verify()?;
    ensure_open_regular(&file, requested)?;
    Ok(file)
}

pub(super) fn sync_journal_file(file: &File, path: &Path) -> SceneJournalResult<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::HANDLE, Storage::FileSystem::FlushFileBuffers};

    // FILE_FLAG_WRITE_THROUGH makes the write synchronous; this explicit flush
    // is the final acknowledgement barrier for every committed JSONL line.
    if unsafe { FlushFileBuffers(file.as_raw_handle() as HANDLE) } == 0 {
        return Err(io_error(path, std::io::Error::last_os_error()));
    }
    Ok(())
}

pub(super) fn sync_parent(path: &Path) -> SceneJournalResult<()> {
    WindowsJournalParent::open(path)?.verify()
}

pub(super) fn same_open_file(left: &File, right: &File, path: &Path) -> SceneJournalResult<bool> {
    Ok(identity(left, path)? == identity(right, path)?)
}

pub(super) fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

/// Pin a non-reparse parent and use its physical path while opening one fixed
/// journal leaf. The parent handle is retained for the entire path operation.
struct WindowsJournalParent {
    directory: File,
    identity: WindowsFileIdentity,
    physical_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsFileIdentity {
    volume: u32,
    index_high: u32,
    index_low: u32,
    attributes: u32,
}

impl WindowsJournalParent {
    fn open(path: &Path) -> SceneJournalResult<Self> {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_WRITE_THROUGH, FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ, OPEN_EXISTING,
            },
        };

        let parent = path
            .parent()
            .ok_or_else(|| invalid("scene journal path has no parent directory"))?;
        let mut wide = parent.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL
                    | FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_WRITE_THROUGH,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io_error(path, std::io::Error::last_os_error()));
        }
        // SAFETY: CreateFileW returned one owned directory handle.
        let directory = unsafe { File::from_raw_handle(handle as *mut _) };
        let identity = identity(&directory, path)?;
        ensure_directory(identity, path)?;
        let physical_path = final_path(&directory, path)?;
        Ok(Self {
            directory,
            identity,
            physical_path,
        })
    }

    fn verify(&self) -> SceneJournalResult<()> {
        let current = identity(&self.directory, &self.physical_path)?;
        if current != self.identity {
            return Err(invalid(
                "scene journal parent directory identity changed while owned",
            ));
        }
        ensure_directory(current, &self.physical_path)
    }

    fn leaf_path(&self, requested: &Path) -> SceneJournalResult<PathBuf> {
        self.verify()?;
        let leaf = requested
            .file_name()
            .filter(|leaf| *leaf == std::ffi::OsStr::new(SCENE_JOURNAL_FILE))
            .ok_or_else(|| invalid("scene journal leaf is not its fixed local filename"))?;
        Ok(self.physical_path.join(leaf))
    }
}

fn identity(file: &File, path: &Path) -> SceneJournalResult<WindowsFileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
        return Err(io_error(path, std::io::Error::last_os_error()));
    }
    Ok(WindowsFileIdentity {
        volume: info.dwVolumeSerialNumber,
        index_high: info.nFileIndexHigh,
        index_low: info.nFileIndexLow,
        attributes: info.dwFileAttributes,
    })
}

fn ensure_directory(identity: WindowsFileIdentity, path: &Path) -> SceneJournalResult<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    if identity.attributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || identity.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(invalid(format!(
            "scene journal parent is redirected or not a local directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn final_path(file: &File, path: &Path) -> SceneJournalResult<PathBuf> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED},
    };

    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as HANDLE,
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).expect("bounded Windows path buffer"),
            FILE_NAME_NORMALIZED,
        )
    };
    if length == 0
        || usize::try_from(length)
            .ok()
            .is_none_or(|size| size >= buffer.len())
    {
        return Err(io_error(path, std::io::Error::last_os_error()));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(
        &buffer[..usize::try_from(length).expect("bounded Windows path length")],
    )))
}
