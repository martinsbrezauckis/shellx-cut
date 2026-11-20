//! Tell a name that stands for another entry (a link) from an ordinary file
//! or folder.
//!
//! On Windows a path entry may carry a reparse point. Only a reparse point
//! whose tag has the name-surrogate bit represents another named entity
//! (symbolic links and junctions/mount points); other reparse points, such as
//! cloud-file placeholders kept by a sync client, compressed (WOF) files or
//! deduplicated files, are the file or folder itself. Project checks refuse
//! the first kind and treat the second kind as ordinary entries.

/// Name-surrogate bit of a Windows reparse tag (`IsReparseTagNameSurrogate`).
pub const NAME_SURROGATE_BIT: u32 = 0x2000_0000;

/// Whether a reparse tag stands for another named entity.
pub const fn is_name_surrogate_tag(tag: u32) -> bool {
    tag & NAME_SURROGATE_BIT != 0
}

/// Whether a path entry, as reported by `std::fs::symlink_metadata`, names
/// another entry. On Unix this is a symbolic link. On Windows the standard
/// library reads the reparse tag (`FileAttributeTagInfo`) of a reparse point,
/// fails the metadata call when it cannot, and reports `is_symlink` exactly
/// for name-surrogate tags, so a cloud placeholder or compressed file is not
/// reported here.
pub fn names_another_entry(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// The reparse tag of an open handle, or `None` when the file has no reparse
/// point. An error means the tag could not be read; callers refuse the file.
#[cfg(windows)]
pub fn handle_reparse_tag(file: &std::fs::File) -> std::io::Result<Option<u32>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_ATTRIBUTE_TAG_INFO,
    };
    let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: file owns the live handle and info is a writable buffer of the
    // size passed for the FileAttributeTagInfo class.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileAttributeTagInfo,
            std::ptr::addr_of_mut!(info).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0).then_some(info.ReparseTag))
}

/// Volume serial number, file index and link count of an open handle.
#[cfg(windows)]
pub fn handle_identity(file: &std::fs::File) -> std::io::Result<(u32, u64, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: file owns the live handle and info has the required writable ABI.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((info.dwVolumeSerialNumber, index, info.nNumberOfLinks))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tag values from [MS-FSCC] 2.1.2.1 "Reparse Tags".
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
    const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
    const IO_REPARSE_TAG_LX_SYMLINK: u32 = 0xA000_001D;
    const IO_REPARSE_TAG_DEDUP: u32 = 0x8000_0013;
    const IO_REPARSE_TAG_WOF: u32 = 0x8000_0017;
    const IO_REPARSE_TAG_CLOUD: u32 = 0x9000_001A;
    const IO_REPARSE_TAG_CLOUD_F: u32 = 0x9000_F01A;
    const IO_REPARSE_TAG_APPEXECLINK: u32 = 0x8000_001B;

    #[test]
    fn links_and_junctions_are_name_surrogates() {
        for tag in [
            IO_REPARSE_TAG_SYMLINK,
            IO_REPARSE_TAG_MOUNT_POINT,
            IO_REPARSE_TAG_LX_SYMLINK,
        ] {
            assert!(is_name_surrogate_tag(tag), "{tag:#x}");
        }
    }

    #[test]
    fn cloud_compressed_and_deduplicated_files_are_not_name_surrogates() {
        for tag in [
            IO_REPARSE_TAG_CLOUD,
            IO_REPARSE_TAG_CLOUD_F,
            IO_REPARSE_TAG_WOF,
            IO_REPARSE_TAG_DEDUP,
            IO_REPARSE_TAG_APPEXECLINK,
        ] {
            assert!(!is_name_surrogate_tag(tag), "{tag:#x}");
        }
    }
}
