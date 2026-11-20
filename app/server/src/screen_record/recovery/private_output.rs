//! Files recovery writes inside a recording folder.
//!
//! A recording folder lives in a project, and a project can be received from
//! someone else, so any name in it may already hold a planted link. Recovery
//! therefore never opens or lets a tool open an output path in the recording
//! folder. Each output is produced in a new private folder that this process
//! creates itself (creation fails if the name exists), and is then moved to
//! its final name by rename. A rename replaces a link at the destination name;
//! it never writes through it. A destination that is a folder, a link, or a
//! file with several names is refused instead, so nothing outside the project
//! changes and the person gets a plain reason.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cut_core::store::{plain_file_state, PlainFileState};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// A private folder owned by one recovery run. Removed when dropped.
pub(crate) struct RunDir {
    path: PathBuf,
}

impl RunDir {
    /// Create a new, empty folder inside `parent` (the admitted recording
    /// folder). Fails rather than reuse anything already at the name.
    pub(crate) fn create(parent: &Path) -> std::io::Result<Self> {
        if !record_recovery::is_plain_dir(parent).unwrap_or(false) {
            return Err(std::io::Error::other(
                "the recording folder is not an ordinary folder",
            ));
        }
        let millis = super::super::active_capture::unix_ms_now();
        for _ in 0..64 {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".recover-{millis}-{}-{sequence}",
                std::process::id()
            ));
            // Only Unix sets a mode on the builder.
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "no unused recovery folder name was available",
        ))
    }

    /// A file name inside the private folder.
    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Why an output could not be put at its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused(pub String);

/// Move `staged` (a file inside a [`RunDir`]) to `dest`. `dest` must be
/// absent or an ordinary single-name file, which is replaced.
pub(crate) fn publish(staged: &Path, dest: &Path) -> Result<(), Refused> {
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if plain_file_state(dest) == PlainFileState::NotPlain {
        return Err(Refused(format!(
            "{name} in the recording folder is a link or not an ordinary file; it was left untouched"
        )));
    }
    std::fs::rename(staged, dest)
        .map_err(|error| Refused(format!("{name} could not be saved: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_folder_is_new_private_and_removed_afterwards() {
        let parent = tempfile::tempdir().unwrap();
        let path = {
            let run = RunDir::create(parent.path()).unwrap();
            std::fs::write(run.file("x"), b"staged").unwrap();
            let dest = parent.path().join("x.out");
            publish(&run.file("x"), &dest).unwrap();
            assert_eq!(std::fs::read(dest).unwrap(), b"staged");
            run.path.clone()
        };
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_destination_is_refused_and_its_target_unchanged() {
        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("sentinel");
        std::fs::write(&sentinel, b"outside").unwrap();
        let dest = parent.path().join("take.mp4");
        std::os::unix::fs::symlink(&sentinel, &dest).unwrap();
        let run = RunDir::create(parent.path()).unwrap();
        std::fs::write(run.file("take.mp4"), b"recovered").unwrap();
        let refused = publish(&run.file("take.mp4"), &dest).unwrap_err();
        assert!(
            refused.0.contains("link or not an ordinary file"),
            "{}",
            refused.0
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside");
        let hard = parent.path().join("hard.mp4");
        std::fs::hard_link(&sentinel, &hard).unwrap();
        let refused = publish(&run.file("take.mp4"), &hard).unwrap_err();
        assert!(refused.0.contains("left untouched"));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside");
    }
}
