//! The one way the media engine admits and writes its internal files.
//!
//! A project folder can come from someone else. Any name under it may already
//! be a symbolic link, a junction, or a hard link (a second name for a file
//! elsewhere on the same disk). Writing to such a name in place writes into
//! that other file (item 140). So:
//!
//! - lists and other scratch files FFmpeg reads are written as fresh files
//!   with random names in the engine's private work folder, never in the
//!   project ([`private_list_file`]);
//! - files kept in the project (cache segments, proxies, previews) are written
//!   to a fresh temporary beside the name and renamed over it, which replaces
//!   the name and never opens what was there (`crate::atomic_output`);
//! - an existing internal file is reused only when it is an ordinary file with
//!   exactly one name ([`plain_cache_file_exists`]); on Windows the link count
//!   is read from the file handle.

use cut_core::{error_codes, CutError};
use std::path::Path;

/// Whether an internal media file exists as an ordinary single-name file.
/// `Ok(false)` when nothing is there. A link, junction, second name (hard
/// link) or non-file is refused with a plain reason and the way out.
pub(crate) fn plain_cache_file_exists(path: &Path) -> Result<bool, CutError> {
    cut_core::matte_cache::plain_file_exists(path).map_err(|e| {
        if e.code == error_codes::INVALID_ARGS {
            return CutError::new(
                error_codes::IO,
                format!("unsafe media cache file: {}", path.display()),
                "internal output files must be ordinary files with one name: this one is a link, \
                 a junction or a second name of another file, so Cut does not read or write it",
            )
            .with_suggested_action(
                "remove the project's cache folders (.cache, cache, proxies/preview-cache) or the named file, \
                 then try again; Cut rebuilds them",
            );
        }
        e
    })
}

/// Write `contents` to a fresh file with a random name in the engine's
/// private work folder and return its path. The file is removed when the
/// returned value is dropped, so keep it alive while FFmpeg reads it.
pub(crate) fn private_list_file(
    prefix: &str,
    suffix: &str,
    contents: &str,
) -> Result<tempfile::TempPath, CutError> {
    use std::io::Write;
    let dir = crate::private_work::session_dir()?;
    let mut file = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile_in(&dir)?;
    file.write_all(contents.as_bytes())?;
    file.as_file().sync_all()?;
    Ok(file.into_temp_path())
}

#[cfg(test)]
mod tests;
