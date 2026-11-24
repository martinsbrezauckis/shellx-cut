//! Windows parent-directory pinning and no-reparse traversal via
//! `NtOpenFile(RootDirectory, ...)` with `FILE_OPEN_REPARSE_POINT` plus
//! handle-based `GetFinalPathNameByHandleW`.

use cut_core::{error_codes, CutError};
use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileDispositionInfo, GetFileInformationByHandle,
    GetFileInformationByHandleEx, GetFinalPathNameByHandleW, MoveFileExW,
    SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, CREATE_NEW, DELETE,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING, SYNCHRONIZE,
};

fn output_folder_error(folder: &Path, error: &std::io::Error) -> CutError {
    crate::folder_access::folder_error("output folder", folder, error)
}

const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_OPEN_REPARSE_POINT_OPT: u32 = 0x0020_0000;
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

#[repr(C)]
struct NtUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
struct NtObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *const NtUnicodeString,
    attributes: u32,
    security_descriptor: *const c_void,
    security_quality_of_service: *const c_void,
}

#[repr(C)]
#[derive(Default)]
struct NtIoStatusBlock {
    status_or_ptr: usize,
    information: usize,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtOpenFile(
        file_handle: *mut HANDLE,
        desired_access: u32,
        object_attributes: *const NtObjectAttributes,
        io_status_block: *mut NtIoStatusBlock,
        share_access: u32,
        open_options: u32,
    ) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DirIdentity {
    volume: u32,
    index: u64,
}

pub(super) struct WindowsPinnedDir {
    handle: OwnedHandle,
    identity: DirIdentity,
    chain: Vec<(String, DirIdentity)>,
    verified_path: PathBuf,
}

fn validate_component(name: &OsStr) -> Result<String, CutError> {
    let s = name.to_string_lossy();
    if s.is_empty() || s == "." || s == ".." || s.contains(['\\', '/', ':', '\0']) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid Windows path component: {s}"),
            "expected a single safe directory entry name",
        ));
    }
    Ok(s.into_owned())
}

fn handle_dir_identity(handle: HANDLE, sofar: &Path) -> Result<DirIdentity, CutError> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(output_folder_error(sofar, &io::Error::last_os_error()));
    }
    if (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        let mut tag: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        } == 0
        {
            return Err(output_folder_error(sofar, &io::Error::last_os_error()));
        }
        if cut_core::reparse::is_name_surrogate_tag(tag.ReparseTag) {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "output directory is not a local plain directory: {}",
                    sofar.display()
                ),
                "export publication refuses Windows symlinks and junctions",
            ));
        }
    }
    if (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) == 0 {
        return Err(CutError::new(
            error_codes::IO,
            format!(
                "output directory is not a local plain directory: {}",
                sofar.display()
            ),
            "expected a directory",
        ));
    }
    Ok(DirIdentity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

fn open_root_prefix(prefix: &Path) -> Result<OwnedHandle, CutError> {
    let wide: Vec<u16> = prefix.as_os_str().encode_wide().chain([0]).collect();
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            SHARE_ALL,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(output_folder_error(prefix, &io::Error::last_os_error()));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(raw.cast()) })
}

fn nt_open_child(
    parent: &OwnedHandle,
    name: &OsStr,
    access: u32,
    opts: u32,
) -> io::Result<OwnedHandle> {
    let text = name.to_string_lossy();
    if text.is_empty() || text == "." || text == ".." || text.contains(['\\', '/', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Windows entry name",
        ));
    }
    let utf16: Vec<u16> = name.encode_wide().collect();
    let byte_len = u16::try_from(utf16.len().saturating_mul(2))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name too long"))?;
    let unicode = NtUnicodeString {
        length: byte_len,
        maximum_length: byte_len,
        buffer: utf16.as_ptr(),
    };
    let attrs = NtObjectAttributes {
        length: std::mem::size_of::<NtObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle() as HANDLE,
        object_name: &unicode,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: null(),
        security_quality_of_service: null(),
    };
    let mut iosb = NtIoStatusBlock::default();
    let mut raw: HANDLE = null_mut();
    let status = unsafe {
        NtOpenFile(
            &mut raw,
            access,
            &attrs,
            &mut iosb,
            SHARE_ALL,
            opts | FILE_OPEN_REPARSE_POINT_OPT | FILE_SYNCHRONOUS_IO_NONALERT,
        )
    };
    if status < 0 {
        if status == 0xC000_0034_u32 as i32 || status == 0xC000_003A_u32 as i32 {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if status == 0xC000_0103_u32 as i32 {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        }
        return Err(io::Error::other(format!(
            "NtOpenFile status 0x{:08x}",
            status as u32
        )));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(raw.cast()) })
}

fn final_path_from_handle(handle: HANDLE) -> Result<PathBuf, CutError> {
    let needed = unsafe { GetFinalPathNameByHandleW(handle, null_mut(), 0, FILE_NAME_NORMALIZED) };
    if needed == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut buf = vec![0u16; needed as usize + 1];
    let len = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            buf.as_mut_ptr(),
            buf.len() as u32,
            FILE_NAME_NORMALIZED,
        )
    };
    if len == 0 || len as usize >= buf.len() {
        return Err(io::Error::last_os_error().into());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buf[..len as usize],
    )))
}

fn walk_no_follow(normalized_path: &Path) -> Result<WindowsPinnedDir, CutError> {
    let mut comps = normalized_path.components().peekable();
    let mut prefix_path = PathBuf::new();
    while let Some(c) = comps.peek() {
        match c {
            Component::Prefix(p) => {
                prefix_path.push(p.as_os_str());
                comps.next();
            }
            Component::RootDir => {
                prefix_path.push(std::path::MAIN_SEPARATOR_STR);
                comps.next();
                break;
            }
            _ => break,
        }
    }
    let mut cur = open_root_prefix(&prefix_path)?;
    let mut cur_id = handle_dir_identity(cur.as_raw_handle() as HANDLE, &prefix_path)?;
    let mut sofar = prefix_path;
    let mut chain = Vec::new();
    for c in comps {
        if let Component::Normal(part) = c {
            let name = validate_component(part)?;
            sofar.push(&name);
            cur = nt_open_child(
                &cur,
                part,
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_DIRECTORY_FILE,
            )
            .map_err(|e| output_folder_error(&sofar, &e))?;
            cur_id = handle_dir_identity(cur.as_raw_handle() as HANDLE, &sofar)?;
            chain.push((name, cur_id));
        }
    }
    let verified_path = final_path_from_handle(cur.as_raw_handle() as HANDLE)?;
    Ok(WindowsPinnedDir {
        handle: cur,
        identity: cur_id,
        chain,
        verified_path,
    })
}

impl WindowsPinnedDir {
    pub(super) fn open(normalized_path: &Path, _harden_project: bool) -> Result<Self, CutError> {
        walk_no_follow(normalized_path)
    }

    pub(super) fn verify_still_at_path(&self, normalized_path: &Path) -> Result<(), CutError> {
        let live_id = handle_dir_identity(self.handle.as_raw_handle() as HANDLE, normalized_path)?;
        if live_id != self.identity {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "export directory identity changed while exporting: {}",
                    normalized_path.display()
                ),
                "refusing publication into a replaced directory handle",
            ));
        }
        let current = walk_no_follow(normalized_path)?;
        if current.identity != self.identity || current.chain != self.chain {
            return Err(CutError::new(
                error_codes::IO,
                format!(
                    "export directory parent was moved or replaced while exporting: {}",
                    normalized_path.display()
                ),
                "export publication refuses a parent directory that changed mid-export",
            ));
        }
        Ok(())
    }

    pub(super) fn clear_stale_empty_child(&self, name: &OsStr) -> Result<(), CutError> {
        let _ = validate_component(name)?;
        let Ok(child) = nt_open_child(
            &self.handle,
            name,
            DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_NON_DIRECTORY_FILE,
        ) else {
            return Ok(());
        };
        let file = File::from(child);
        if cut_core::reparse::handle_reparse_tag(&file)
            .ok()
            .flatten()
            .is_some_and(cut_core::reparse::is_name_surrogate_tag)
        {
            return Ok(());
        }
        if file.metadata().is_ok_and(|m| m.is_file() && m.len() == 0) {
            let disp = FILE_DISPOSITION_INFO { DeleteFile: true };
            let _ = unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle() as HANDLE,
                    FileDispositionInfo,
                    (&disp as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
        }
        Ok(())
    }

    pub(super) fn verified_child_path(&self, name: &OsStr) -> Result<PathBuf, CutError> {
        let _ = validate_component(name)?;
        Ok(self.verified_path.join(name))
    }

    pub(super) fn create_exclusive_temp(&self, tmp_name: &OsStr) -> Result<File, CutError> {
        let path = self.verified_child_path(tmp_name)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_WRITE | DELETE,
                0,
                null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_handle(raw.cast()) })
    }

    pub(super) fn unlink_child_best_effort(&self, name: &OsStr) {
        if let Ok(path) = self.verified_child_path(name) {
            let _ = std::fs::remove_file(path);
        }
    }

    pub(super) fn move_file_replace_no_copy(src: &Path, dst: &Path) -> io::Result<()> {
        let from: Vec<u16> = src.as_os_str().encode_wide().chain([0]).collect();
        let to: Vec<u16> = dst.as_os_str().encode_wide().chain([0]).collect();
        let mut attempt = 0u32;
        loop {
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } != 0
            {
                return Ok(());
            }
            let err = unsafe { GetLastError() };
            // Retry briefly on Windows sharing/AV transient locks; never retry EXDEV (17).
            if (err == 5 || err == 32) && attempt < 4 {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(15 * u64::from(attempt)));
                continue;
            }
            return Err(io::Error::from_raw_os_error(err as i32));
        }
    }
}
