//! Project-local containment for the screen-record capture tree.
//!
//! This deliberately validates stable, pre-existing path components before every
//! operation. It is not a handle-relative sandbox and does not claim to resist a
//! concurrent same-user path swap; Cut's personal workstation is trusted for that
//! case. It does reject static links and Windows reparse points before they can
//! redirect capture data outside `<project>/cache/screen_record`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contract::ManifestError;
use crate::manifest::{io, is_plain_dir, is_plain_regular_file, valid_capture_id};

const CACHE_COMPONENT: &str = "cache";
const SCREEN_RECORD_COMPONENT: &str = "screen_record";

/// A validated project-local screen-record cache root.
#[derive(Debug, Clone)]
pub struct CaptureRoot {
    project_dir: PathBuf,
    cache_dir: PathBuf,
}

impl CaptureRoot {
    /// Open the capture root, creating only its two literal descendants.
    pub fn for_project(project_dir: &Path) -> Result<Self, ManifestError> {
        require_plain_dir(project_dir, "project directory")?;
        let cache = ensure_plain_child(project_dir, CACHE_COMPONENT)?;
        let cache_dir = ensure_plain_child(&cache, SCREEN_RECORD_COMPONENT)?;
        Ok(Self {
            project_dir: project_dir.to_path_buf(),
            cache_dir,
        })
    }

    /// Open an already-created capture root without changing the project tree.
    pub fn open_existing(project_dir: &Path) -> Result<Option<Self>, ManifestError> {
        require_plain_dir(project_dir, "project directory")?;
        let cache = project_dir.join(CACHE_COMPONENT);
        if !existing_plain_dir(&cache, "project cache directory")? {
            return Ok(None);
        }
        let cache_dir = cache.join(SCREEN_RECORD_COMPONENT);
        if !existing_plain_dir(&cache_dir, "screen-record cache directory")? {
            return Ok(None);
        }
        Ok(Some(Self {
            project_dir: project_dir.to_path_buf(),
            cache_dir,
        }))
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Create one new, literal capture-id directory. Existing capture roots are
    /// rejected so a stale directory cannot supply pre-existing artifact leaves.
    pub fn create_capture_dir(&self, capture_id: &str) -> Result<PathBuf, ManifestError> {
        validate_capture_component(capture_id)?;
        self.revalidate_root()?;
        let capture_dir = self.cache_dir.join(capture_id);
        match fs::symlink_metadata(&capture_dir) {
            Ok(_) => {
                require_plain_dir(&capture_dir, "capture directory")?;
                Err(ManifestError::Invalid(format!(
                    "capture directory already exists: {}",
                    capture_dir.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                require_plain_dir(&self.cache_dir, "screen-record cache directory")?;
                fs::create_dir(&capture_dir).map_err(|source| io(&capture_dir, source))?;
                require_plain_dir(&capture_dir, "capture directory")?;
                Ok(capture_dir)
            }
            Err(source) => Err(io(&capture_dir, source)),
        }
    }

    /// Return an existing local capture directory without creating it.
    pub fn existing_capture_dir(&self, capture_id: &str) -> Result<Option<PathBuf>, ManifestError> {
        validate_capture_component(capture_id)?;
        self.revalidate_root()?;
        let capture_dir = self.cache_dir.join(capture_id);
        match fs::symlink_metadata(&capture_dir) {
            Ok(_) => {
                require_plain_dir(&capture_dir, "capture directory")?;
                Ok(Some(capture_dir))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(io(&capture_dir, source)),
        }
    }

    /// Derive a literal file leaf after revalidating every directory component.
    pub fn capture_file(
        &self,
        capture_id: &str,
        file_name: &str,
    ) -> Result<PathBuf, ManifestError> {
        validate_file_component(file_name)?;
        let capture_dir = self.existing_capture_dir(capture_id)?.ok_or_else(|| {
            ManifestError::Invalid(format!("capture directory is missing: {capture_id}"))
        })?;
        Ok(capture_dir.join(file_name))
    }

    /// Atomically publish a new local file without replacing an existing leaf.
    /// The marker reader observes either no file or the complete synced payload.
    pub fn publish_new_capture_file(
        &self,
        capture_id: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, ManifestError> {
        let path = self.capture_file(capture_id, file_name)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(ManifestError::Invalid(format!(
                    "refusing to replace existing capture file: {}",
                    path.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io(&path, source)),
        }
        // Revalidate immediately before creating a sibling staging file.
        let capture_dir = self
            .existing_capture_dir(capture_id)?
            .expect("capture_file confirmed the capture directory exists");
        let part = write_new_part(&capture_dir, file_name, bytes)?;
        if let Err(source) = crate::atomic::publish_new_synced(&part, &path) {
            if is_plain_regular_file(&part).unwrap_or(false) {
                let _ = fs::remove_file(&part);
            }
            return Err(io(&path, source));
        }
        is_plain_regular_file(&path)?
            .then_some(path)
            .ok_or_else(|| ManifestError::Invalid("published capture file is not local".into()))
    }

    fn revalidate_root(&self) -> Result<(), ManifestError> {
        require_plain_dir(&self.project_dir, "project directory")?;
        let cache = self.project_dir.join(CACHE_COMPONENT);
        require_plain_dir(&cache, "project cache directory")?;
        require_plain_dir(&self.cache_dir, "screen-record cache directory")
    }
}

fn ensure_plain_child(parent: &Path, component: &str) -> Result<PathBuf, ManifestError> {
    let path = parent.join(component);
    match fs::symlink_metadata(&path) {
        Ok(_) => require_plain_dir(&path, "capture-root component")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            require_plain_dir(parent, "capture-root parent")?;
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(io(&path, source)),
            }
            require_plain_dir(&path, "capture-root component")?;
        }
        Err(source) => return Err(io(&path, source)),
    }
    Ok(path)
}

fn require_plain_dir(path: &Path, role: &str) -> Result<(), ManifestError> {
    is_plain_dir(path)?
        .then_some(())
        .ok_or_else(|| ManifestError::Invalid(format!("{role} is not a local plain directory")))
}

fn existing_plain_dir(path: &Path, role: &str) -> Result<bool, ManifestError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io(path, source)),
        Ok(_) => {
            require_plain_dir(path, role)?;
            Ok(true)
        }
    }
}

fn validate_capture_component(capture_id: &str) -> Result<(), ManifestError> {
    valid_capture_id(capture_id)
        .then_some(())
        .ok_or_else(|| ManifestError::Invalid("capture id is not a literal path component".into()))
}

fn validate_file_component(file_name: &str) -> Result<(), ManifestError> {
    (!file_name.is_empty()
        && file_name != "."
        && file_name != ".."
        && !file_name.contains(['/', '\\', ':']))
    .then_some(())
    .ok_or_else(|| {
        ManifestError::Invalid("capture file name is not a literal path component".into())
    })
}

fn write_new_part(
    capture_dir: &Path,
    file_name: &str,
    bytes: &[u8],
) -> Result<PathBuf, ManifestError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    for attempt in 0..32 {
        let part = capture_dir.join(format!(
            ".{file_name}-{}-{nonce}-{attempt}.part",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        match options.open(&part) {
            Ok(mut file) => {
                file.write_all(bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|source| io(&part, source))?;
                return Ok(part);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io(&part, source)),
        }
    }
    Err(ManifestError::Invalid(
        "could not reserve a local capture staging file".into(),
    ))
}

#[cfg(test)]
mod tests;
