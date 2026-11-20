//! Lock-file ownership for output reservations (item 98).
//!
//! A reservation holds an advisory lock on `.<name>.shellx-cut-reservation.lock`
//! beside the output. The owner now deletes that file when it releases the
//! reservation, so exports folders no longer collect zero-byte lock files.
//!
//! Deleting a lock file is safe only with the usual rule: whoever takes the
//! lock checks afterwards that the path still names the very file it locked.
//! A waiter that opened the old file before the owner deleted it gets the lock
//! on a file nobody can see any more; the check fails, it lets go and opens
//! the name again (a new file, or the next owner's). The owner removes the
//! name while it still holds the lock, so no one can lock that file after the
//! name is gone and believe it is current.

use std::fs::File;
use std::path::Path;

/// True when `path` (not followed if it is a link) names the same file as
/// `file`.
/// A platform that cannot tell file identities answers true (the old,
/// keep-the-file behaviour is then what protects it).
pub(super) fn still_named(file: &File, path: &Path) -> bool {
    match identity_of_file(file) {
        None => true,
        Some(mine) => identity_of_path(path) == Some(mine),
    }
}

#[cfg(unix)]
fn identity_of_file(file: &File) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().ok()?;
    Some((m.dev(), m.ino()))
}

#[cfg(unix)]
fn identity_of_path(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path).ok()?;
    m.file_type().is_file().then(|| (m.dev(), m.ino()))
}

/// Volume serial number and file index (GetFileInformationByHandle).
#[cfg(windows)]
fn identity_of_file(file: &File) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    // SAFETY: the handle is owned by `file` and stays open for the call; the
    // out-struct is plain data we own.
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) == 0 {
            return None;
        }
        Some((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
}

#[cfg(windows)]
fn identity_of_path(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_FLAG_OPEN_REPARSE_POINT: look at a link itself, never its target.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    identity_of_file(&file)
}

#[cfg(not(any(unix, windows)))]
fn identity_of_file(_file: &File) -> Option<(u64, u64)> {
    None
}

#[cfg(not(any(unix, windows)))]
fn identity_of_path(_path: &Path) -> Option<(u64, u64)> {
    None
}
