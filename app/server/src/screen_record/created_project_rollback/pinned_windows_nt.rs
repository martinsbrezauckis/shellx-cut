//! Low-level NT relative handle primitives (`NtOpenFile`, `FileIdBothDirectoryInfo`,
//! `FileDispositionInfo`) for no-follow Windows handle operations.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_NO_MORE_FILES, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, FileDispositionInfo, FileIdBothDirectoryInfo,
    FileIdBothDirectoryRestartInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_INFO,
    FILE_ID_BOTH_DIR_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const FILE_OPEN_REPARSE_POINT_OPT: u32 = 0x0020_0000;
pub(super) const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

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
extern "system" {
    fn NtOpenFile(
        file_handle: *mut HANDLE,
        desired_access: u32,
        object_attributes: *const NtObjectAttributes,
        io_status_block: *mut NtIoStatusBlock,
        share_access: u32,
        open_options: u32,
    ) -> i32;
}

pub(super) fn validate_single_component_utf16(name: &str) -> io::Result<Vec<u16>> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '/', ':', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a single safe Windows entry name",
        ));
    }
    Ok(name.encode_utf16().collect())
}

pub(super) fn handle_info(handle: HANDLE) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    // SAFETY: `BY_HANDLE_FILE_INFORMATION` is plain zero-initializable data.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: valid handle and initialized output struct.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: valid handle and owned `FILE_ATTRIBUTE_TAG_INFO`.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || (tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || tag.ReparseTag != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "handle refers to a Windows reparse point",
        ));
    }
    Ok(info)
}

pub(super) fn dir_identity_from_handle(handle: HANDLE) -> io::Result<super::DirIdentity> {
    let info = handle_info(handle)?;
    if (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "handle is not a directory",
        ));
    }
    Ok(super::DirIdentity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

pub(super) fn file_identity_from_handle(
    handle: HANDLE,
    require_single_link: bool,
) -> io::Result<super::FileIdentity> {
    let info = handle_info(handle)?;
    if (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "handle is a directory, not a regular file",
        ));
    }
    if require_single_link && info.nNumberOfLinks != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file has unexpected hard links",
        ));
    }
    Ok(super::FileIdentity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

pub(super) fn nt_open_child(
    parent_handle: &OwnedHandle,
    name: &str,
    access: u32,
    open_options: u32,
) -> io::Result<OwnedHandle> {
    let utf16 = validate_single_component_utf16(name)?;
    let byte_len = u16::try_from(utf16.len().saturating_mul(2))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name too long"))?;
    let unicode = NtUnicodeString {
        length: byte_len,
        maximum_length: byte_len,
        buffer: utf16.as_ptr(),
    };
    let attrs = NtObjectAttributes {
        length: std::mem::size_of::<NtObjectAttributes>() as u32,
        root_directory: parent_handle.as_raw_handle() as HANDLE,
        object_name: &unicode,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: null(),
        security_quality_of_service: null(),
    };
    let mut iosb = NtIoStatusBlock::default();
    let mut raw: HANDLE = null_mut();
    // SAFETY: relative open anchored to `parent_handle` with single component and `FILE_OPEN_REPARSE_POINT`.
    let status = unsafe {
        NtOpenFile(
            &mut raw,
            access,
            &attrs,
            &mut iosb,
            SHARE_ALL,
            open_options | FILE_OPEN_REPARSE_POINT_OPT | FILE_SYNCHRONOUS_IO_NONALERT,
        )
    };
    if status < 0 {
        return Err(io::Error::other(format!(
            "NtOpenFile refused relative open: 0x{:08x}",
            status as u32
        )));
    }
    // SAFETY: `NtOpenFile` succeeded and returned an owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw as *mut _) })
}

pub(super) fn mark_delete_on_close(handle: HANDLE) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: valid owned handle and initialized `FILE_DISPOSITION_INFO`.
    if unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn list_child_names_from_handle(dir_handle: HANDLE) -> io::Result<Vec<String>> {
    let mut buf = vec![0u64; 8192];
    let buf_bytes = buf.len() * std::mem::size_of::<u64>();
    let mut names = Vec::new();
    let mut class = FileIdBothDirectoryRestartInfo;
    loop {
        // SAFETY: `buf` is 8-byte aligned and sized cleanly for `FILE_ID_BOTH_DIR_INFO` batches.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                dir_handle,
                class,
                buf.as_mut_ptr().cast(),
                buf_bytes as u32,
            )
        };
        if ok == 0 {
            // SAFETY: thread-local Win32 error code immediately after failing call.
            let code = unsafe { GetLastError() };
            if code == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        class = FileIdBothDirectoryInfo;
        let base = buf.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        loop {
            if offset + std::mem::size_of::<FILE_ID_BOTH_DIR_INFO>() > buf_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "truncated directory listing buffer",
                ));
            }
            // SAFETY: `offset` comes from kernel `NextEntryOffset` within `buf_bytes` and is aligned.
            let entry = unsafe { &*(base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>()) };
            let name_bytes = entry.FileNameLength as usize;
            // SAFETY: `FileName` lies inside `entry` within `buf_bytes`.
            let name_offset =
                unsafe { entry.FileName.as_ptr().cast::<u8>().offset_from(base) as usize };
            if !name_bytes.is_multiple_of(2) || name_offset + name_bytes > buf_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "malformed Windows directory entry length",
                ));
            }
            // SAFETY: bounds checked above against `buf_bytes`.
            let utf16 =
                unsafe { std::slice::from_raw_parts(entry.FileName.as_ptr(), name_bytes / 2) };
            let name = String::from_utf16(utf16).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-16 entry name")
            })?;
            if name != "." && name != ".." {
                if (entry.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "reparse point found during directory enumeration",
                    ));
                }
                names.push(name);
            }
            if entry.NextEntryOffset == 0 {
                break;
            }
            offset = offset
                .checked_add(entry.NextEntryOffset as usize)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "offset overflow"))?;
        }
    }
    Ok(names)
}
