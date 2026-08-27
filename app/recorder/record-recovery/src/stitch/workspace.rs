//! Private, no-follow workspace ownership for one stitch operation.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::{is_plain_dir, is_plain_regular_file};
use crate::ManifestError;

static WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) struct StitchWorkspace {
    root: PathBuf,
    files: Vec<PathBuf>,
}

impl StitchWorkspace {
    pub(super) fn new(capture_root: &Path) -> Result<Self, ManifestError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        for attempt in 0..32 {
            let counter = WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = capture_root.join(format!(
                ".stitch-work-{}-{nonce}-{counter}-{attempt}",
                std::process::id()
            ));
            match create_private_dir(&root) {
                Ok(()) => {
                    if is_plain_dir(&root)? {
                        return Ok(Self {
                            root,
                            files: Vec::new(),
                        });
                    }
                    return Err(ManifestError::Invalid("unsafe stitch workspace".into()));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(ManifestError::Io { path: root, source }),
            }
        }
        Err(ManifestError::Invalid(
            "could not reserve stitch workspace".into(),
        ))
    }

    pub(super) fn reserve(&mut self, name: &str) -> Result<PathBuf, ManifestError> {
        self.create(name, None)
    }

    pub(super) fn write(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf, ManifestError> {
        self.create(name, Some(bytes))
    }

    fn create(&mut self, name: &str, bytes: Option<&[u8]>) -> Result<PathBuf, ManifestError> {
        let path = self.root.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| ManifestError::Io {
                path: path.clone(),
                source,
            })?;
        if let Some(bytes) = bytes {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|source| ManifestError::Io {
                    path: path.clone(),
                    source,
                })?;
        }
        drop(file);
        if !is_plain_regular_file(&path)? {
            return Err(ManifestError::Invalid("unsafe stitch temporary".into()));
        }
        self.files.push(path.clone());
        Ok(path)
    }
}

impl Drop for StitchWorkspace {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = fs::remove_file(file);
        }
        let _ = fs::remove_dir(&self.root);
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}
