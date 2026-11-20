//! Whether an opened file has exactly one name (item 140 sweep).
//!
//! A capture folder can come inside a received project. A file there may be a
//! hard link: a second name of a file elsewhere on the same disk. Appending to
//! or truncating such a name changes that other file, so every journal or
//! manifest this crate reopens for writing must have a single name. The count
//! is read from the open handle, so it is the file actually opened.

use std::fs::File;

/// `true` when the open file has exactly one name.
pub fn has_single_name(file: &File) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(file.metadata()?.nlink() == 1)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: file owns a live handle and info has the required writable ABI.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(info.nNumberOfLinks == 1)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Ok(true)
    }
}
