//! Read project files and disposable caches without following project-supplied
//! links.

use super::BoundedCacheRead;
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::path::Path;

/// A regular file that does not name another entry. On Windows a reparse
/// point that is not a name surrogate (for example a cloud-file placeholder)
/// is the file itself; the link count is checked on the opened handle there.
fn plain_file(metadata: &Metadata) -> bool {
    if !metadata.is_file() || crate::reparse::names_another_entry(metadata) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return false;
        }
    }
    true
}

fn admitted_handle(file: &File, before: &Metadata, opened: &Metadata) -> bool {
    if !plain_file(opened) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        before.dev() == opened.dev() && before.ino() == opened.ino()
    }
    #[cfg(windows)]
    {
        // The handle came from `open_windows`, which already refused a
        // name-surrogate reparse point and confirmed the file identity. A
        // tag that cannot be read refuses the file.
        let _ = before;
        let single_link =
            crate::reparse::handle_identity(file).is_ok_and(|(_, _, links)| links == 1);
        let not_surrogate = crate::reparse::handle_reparse_tag(file)
            .is_ok_and(|tag| !tag.is_some_and(crate::reparse::is_name_surrogate_tag));
        single_link && not_surrogate
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, before);
        false
    }
}

/// Whether a project path is absent, an ordinary single-link file, or anything
/// else (a link, a Windows reparse point, a file shared with another path, a
/// directory, a pipe or a device).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainFileState {
    /// Nothing exists at the path.
    Missing,
    /// An ordinary file with exactly one name, opened without following a link.
    Plain,
    /// Something exists at the path but it is not an ordinary single-link file.
    NotPlain,
}

/// Open `path` only if it is an ordinary single-link file, without following
/// a link at the final component, and confirm the opened handle is that file.
fn open_plain(path: &Path) -> Result<(File, Metadata), PlainFileState> {
    let before = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PlainFileState::Missing)
        }
        Err(_) => return Err(PlainFileState::NotPlain),
    };
    if !plain_file(&before) {
        return Err(PlainFileState::NotPlain);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "linux")]
        const O_NOFOLLOW: i32 = 0o400000;
        #[cfg(target_os = "macos")]
        const O_NOFOLLOW: i32 = 0x100;
        options.custom_flags(O_NOFOLLOW);
    }
    #[cfg(windows)]
    let file = open_windows(path, &mut options)?;
    #[cfg(not(windows))]
    let file = options.open(path).map_err(|_| PlainFileState::NotPlain)?;
    let metadata = file.metadata().map_err(|_| PlainFileState::NotPlain)?;
    if !admitted_handle(&file, &before, &metadata) {
        return Err(PlainFileState::NotPlain);
    }
    Ok((file, metadata))
}

/// Open the entry itself first (`FILE_FLAG_OPEN_REPARSE_POINT`) and read its
/// reparse tag. A name surrogate (symbolic link, junction) or an unreadable
/// tag is refused. A file without a reparse point is used as opened. A file
/// with another kind of reparse point (cloud placeholder, compression) is
/// opened again normally so its owner can supply the content, and must be the
/// same file (volume and file index) as the entry inspected first.
#[cfg(windows)]
fn open_windows(path: &Path, options: &mut OpenOptions) -> Result<File, PlainFileState> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    let entry = options
        .clone()
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| PlainFileState::NotPlain)?;
    match crate::reparse::handle_reparse_tag(&entry) {
        Err(_) => Err(PlainFileState::NotPlain),
        Ok(None) => Ok(entry),
        Ok(Some(tag)) if crate::reparse::is_name_surrogate_tag(tag) => {
            Err(PlainFileState::NotPlain)
        }
        Ok(Some(_)) => {
            let content = options.open(path).map_err(|_| PlainFileState::NotPlain)?;
            let first = crate::reparse::handle_identity(&entry);
            let second = crate::reparse::handle_identity(&content);
            match (first, second) {
                (Ok((volume, index, _)), Ok((volume2, index2, _)))
                    if volume == volume2 && index == index2 =>
                {
                    Ok(content)
                }
                _ => Err(PlainFileState::NotPlain),
            }
        }
    }
}

/// Open a project file for streaming only when it is an ordinary single-link
/// file, without following a link at its name. Large inputs (a partial audio
/// file) use this where [`read_bounded_cache`] would hold everything in memory.
pub fn open_plain_file(path: &Path) -> Result<File, PlainFileState> {
    open_plain(path).map(|(file, _)| file)
}

/// Classify a project path with the same rules [`read_bounded_cache`] uses.
/// The caller must already have admitted the parent directory chain.
pub fn plain_file_state(path: &Path) -> PlainFileState {
    match open_plain(path) {
        Ok(_) => PlainFileState::Plain,
        Err(state) => state,
    }
}

/// Read a project file only when it is an ordinary single-link file, without
/// following project-supplied links, and stop at `limit` bytes.
///
/// Project-generated parent directories are admitted by ProjectStore::open,
/// the snapshot reader, or the calling verb. The selected project root may
/// intentionally be a symlink; its leaves may not alias unrelated files.
/// Rejected caches are unavailable so the caller keeps its ordinary journal
/// replay fallback.
pub fn read_bounded_cache(path: &Path, limit: u64) -> BoundedCacheRead {
    let Ok((file, metadata)) = open_plain(path) else {
        return BoundedCacheRead::Unavailable;
    };
    if metadata.len() > limit {
        return BoundedCacheRead::Oversized;
    }
    let mut bytes = Vec::new();
    if file.take(limit + 1).read_to_end(&mut bytes).is_err() {
        return BoundedCacheRead::Unavailable;
    }
    if bytes.len() as u64 > limit {
        return BoundedCacheRead::Oversized;
    }
    BoundedCacheRead::Bytes(bytes)
}

static REPLACE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write `bytes` to a new private file beside `path`, then rename it onto
/// `path`. A link or shared (hard-linked) file already at `path` is replaced
/// as a name; its target is never opened or written. The caller must already
/// have admitted the parent directory chain.
pub fn replace_project_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project-file");
    let sequence = REPLACE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = parent.join(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
    // create_new never follows or reuses an existing entry at the temporary name.
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(first) = std::fs::rename(&temporary, path) {
        // A platform that refuses to rename over an existing name: remove only
        // a file or link entry (never its target), then retry once.
        let replaceable = std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.is_file() || metadata.file_type().is_symlink());
        let retried = if replaceable {
            std::fs::remove_file(path).and_then(|()| std::fs::rename(&temporary, path))
        } else {
            Err(first)
        };
        if let Err(error) = retried {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    }
    Ok(())
}
