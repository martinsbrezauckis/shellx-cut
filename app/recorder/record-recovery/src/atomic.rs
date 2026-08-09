//! Small cross-platform durable replacement primitive for manifest projections.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Durably replace one local projection. Callers use this only after building a
/// complete new value; observers see either the old file or the fully-synced new
/// file, never a truncate-in-place project/receipt hybrid.
pub fn replace_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let part = path.with_extension(format!("replace-{}-{nonce}.part", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    replace(&part, path)?;
    File::open(path)?.sync_all()?;
    sync_parent(path)
}

/// Publish an already-synced new file only if its final name does not exist. This
/// closes the exists-then-rename race for recovery/source output publication.
pub fn publish_new_synced(part: &Path, path: &Path) -> io::Result<()> {
    ensure_plain_dir(part.parent())?;
    ensure_plain_dir(path.parent())?;
    let staged = open_regular_nofollow(part).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!("validate staged publication file: {source}"),
        )
    })?;
    staged.sync_all().map_err(|source| {
        io::Error::new(
            source.kind(),
            format!("sync staged publication file: {source}"),
        )
    })?;
    drop(staged);
    publish_new(part, path).map_err(|source| {
        io::Error::new(source.kind(), format!("publish finalized output: {source}"))
    })?;
    open_regular_nofollow(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| {
            io::Error::new(source.kind(), format!("sync finalized output: {source}"))
        })?;
    sync_parent(path)
}

fn ensure_plain_dir(path: Option<&Path>) -> io::Result<()> {
    let Some(path) = path else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication path has no parent directory",
        ));
    };
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() && !is_reparse(&metadata)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication parent is not a local directory",
        ))
    }
}

fn open_regular_nofollow(path: &Path) -> io::Result<File> {
    let initial = fs::symlink_metadata(path)?;
    if !is_plain_regular(&initial) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication source is not a local regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        // FlushFileBuffers requires a write-capable handle on Windows. A
        // read-only validation reopen reports ERROR_ACCESS_DENIED after the
        // capture writer has correctly finalized its staging file.
        options.write(true);
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    if is_plain_regular(&file.metadata()?) {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened publication file is not a local regular file",
        ))
    }
}

fn is_plain_regular(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn replace(part: &Path, path: &Path) -> io::Result<()> {
    std::fs::rename(part, path)
}

#[cfg(unix)]
fn publish_new(part: &Path, path: &Path) -> io::Result<()> {
    // Linking a same-volume temp file creates the final name atomically only
    // when absent. Removing the old link leaves the finalized inode in place.
    std::fs::hard_link(part, path)?;
    std::fs::remove_file(part)
}

#[cfg(windows)]
fn publish_new(part: &Path, path: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let from = extended_absolute_wide(part)?;
    let to = extended_absolute_wide(path)?;
    // Deliberately omit MOVEFILE_REPLACE_EXISTING: Windows must fail instead
    // of replacing a racing finalized capture. WRITE_THROUGH keeps the
    // existing durable-publication contract without any delete-before-rename
    // sequence in product code.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn publish_new(part: &Path, path: &Path) -> io::Result<()> {
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "final output exists",
        ));
    }
    std::fs::rename(part, path)
}

#[cfg(windows)]
fn replace(part: &Path, path: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from = extended_absolute_wide(part)?;
    let to = extended_absolute_wide(path)?;
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `MoveFileExW` does not add an extended-length prefix itself. Rebuild a
/// validated file path from its canonical local parent and normal final
/// component, then pass a verbatim absolute form to the raw Win32 call. The
/// caller has already checked the source/destination parents and source file;
/// this conversion only avoids the legacy path parser, it does not weaken the
/// no-replace or reparse-point checks around publication.
#[cfg(windows)]
fn extended_absolute_wide(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication path has no parent directory",
        )
    })?;
    let file_name = path.file_name().filter(|name| {
        let candidate = Path::new(name);
        matches!(
            candidate.components().next(),
            Some(std::path::Component::Normal(_))
        ) && candidate.components().count() == 1
    });
    let Some(file_name) = file_name else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication path has no normal final file name",
        ));
    };
    let absolute = fs::canonicalize(parent)?.join(file_name);
    let raw = absolute.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut prefixed = if raw.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16])
    {
        raw
    } else if raw.starts_with(&[b'\\' as u16, b'\\' as u16]) {
        let mut value = r"\\?\UNC\".encode_utf16().collect::<Vec<_>>();
        value.extend_from_slice(&raw[2..]);
        value
    } else {
        let mut value = r"\\?\".encode_utf16().collect::<Vec<_>>();
        value.extend_from_slice(&raw);
        value
    };
    prefixed.push(0);
    Ok(prefixed)
}

#[cfg(not(any(unix, windows)))]
fn replace(part: &Path, path: &Path) -> io::Result<()> {
    std::fs::rename(part, path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
}
#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}
