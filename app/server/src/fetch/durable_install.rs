//! durable_install.rs — put verified programs into the tools folder so that
//! no half-written program can ever be started under its final name, and so
//! that a power loss right after the install cannot leave an empty program.
//!
//! - Every program is flushed to disk before it is renamed into place, and
//!   the folder that received the rename is flushed after it (Unix; Windows
//!   commits a rename with the file system's own journal).
//! - The cross-volume fallback copies each program under a temporary name in
//!   the destination folder, flushes and closes it, and only then renames it
//!   to its final name.

use std::io;
use std::path::Path;

/// Flush a closed file's contents to disk. On Unix a read-only descriptor is
/// enough (and opening the program for writing would itself make it busy for
/// a concurrent start); Windows needs write access to flush.
pub(super) fn sync_file(path: &Path) -> io::Result<()> {
    let file = if cfg!(windows) {
        std::fs::OpenOptions::new().write(true).open(path)?
    } else {
        std::fs::File::open(path)?
    };
    file.sync_all()
}

/// Flush a folder's entries (a rename into it) to disk. Unix only: Windows
/// cannot open a folder this way and journals the rename itself.
pub(super) fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Copy `from` to `to` through a temporary file beside `to`: written, made
/// executable, flushed and closed before the rename, so `to` is either absent
/// or the complete program.
pub(super) fn copy_into_place(from: &Path, to: &Path) -> io::Result<()> {
    let dir = to
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no destination folder"))?;
    let mut source = std::fs::File::open(from)?;
    let mut partial = tempfile::Builder::new()
        .prefix(".install-")
        .tempfile_in(dir)?;
    io::copy(&mut source, partial.as_file_mut())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        partial
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }
    partial.as_file().sync_all()?;
    // Close the write descriptor before the program gets its final name.
    let partial = partial.into_temp_path();
    partial.persist(to).map_err(|error| error.error)?;
    sync_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copied_program_appears_only_complete_and_executable() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("source");
        std::fs::write(&from, b"program bytes").unwrap();
        let dest = dir.path().join("bin");
        std::fs::create_dir(&dest).unwrap();
        let to = dest.join("ffmpeg");
        copy_into_place(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"program bytes");
        let names: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["ffmpeg"], "no temporary file is left behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&to).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        // Replacing an existing program is one rename, never a truncation.
        std::fs::write(&from, b"newer").unwrap();
        copy_into_place(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"newer");
        sync_file(&to).unwrap();
        sync_dir(&dest).unwrap();
    }

    #[test]
    fn a_failed_copy_leaves_no_program_under_the_final_name() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("ffmpeg");
        assert!(copy_into_place(&dir.path().join("missing"), &to).is_err());
        assert!(!to.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
