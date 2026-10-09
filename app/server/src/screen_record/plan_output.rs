//! Immutable default plan publication for an auto-edit request.
//! The caller finishes scene and Studio projections before publishing.

use cut_core::{error_codes, CutError};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) struct PlanStage {
    path: PathBuf,
    cache: PathBuf,
}

impl PlanStage {
    pub(super) fn new(cache: &Path) -> Result<Self, CutError> {
        let temporary = tempfile::Builder::new()
            .prefix(".autoedit-")
            .suffix(".plan.json")
            .tempfile_in(cache)?;
        let (file, path) = temporary.keep().map_err(|error| error.error)?;
        drop(file);
        Ok(Self {
            path,
            cache: cache.to_owned(),
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn publish(&self, track: &Path) -> Result<PathBuf, CutError> {
        self.publish_after_check(track, || {})
    }

    fn publish_after_check(
        &self,
        track: &Path,
        before_persist: impl FnOnce(),
    ) -> Result<PathBuf, CutError> {
        let bytes = std::fs::read(&self.path)?;
        let mut digest = Sha256::new();
        let identity = track.as_os_str().as_encoded_bytes();
        digest.update((identity.len() as u64).to_le_bytes());
        digest.update(identity);
        digest.update(&bytes);
        let path = self
            .cache
            .join(format!("plan-{:x}.json", digest.finalize()));
        // Apply the project-cache and plain-leaf guard before publication. The
        // exclusive persist below remains authoritative if a rival appears.
        let _ = super::cache_output::cache_hit(&path)?;
        before_persist();
        let staged = tempfile::TempPath::try_from_path(self.path.clone())?;
        match staged.persist_noclobber(&path) {
            Ok(()) => Ok(path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A concurrent producer or imported entry won. Only a guarded,
                // byte-identical regular file may satisfy this request.
                let _ = super::cache_output::cache_hit(&path)?;
                if std::fs::read(&path)? == bytes {
                    Ok(path)
                } else {
                    Err(CutError::new(
                        error_codes::IO,
                        "recorder plan cache identity is occupied",
                        format!("existing plan bytes differ at {}", path.display()),
                    ))
                }
            }
            Err(error) => Err(CutError::new(
                error_codes::IO,
                "could not publish recorder plan",
                error.error.to_string(),
            )),
        }
    }
}

impl Drop for PlanStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn simultaneous_identical_plans_publish_one_retained_leaf() {
        let project = tempfile::tempdir().unwrap();
        let cache = super::super::screen_record_cache_dir(project.path()).unwrap();
        let track = project.path().join("capture/events.json");
        let gate = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let stage = PlanStage::new(&cache).unwrap();
                super::super::cache_output::write(stage.path(), b"finished-plan").unwrap();
                let gate = gate.clone();
                let track = track.clone();
                std::thread::spawn(move || {
                    stage.publish_after_check(&track, || {
                        gate.wait();
                    })
                })
            })
            .collect();
        let first = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>();
        assert!(first.iter().all(Result::is_ok), "{first:?}");
        let paths: Vec<_> = first.into_iter().map(Result::unwrap).collect();
        assert_eq!(paths[0], paths[1]);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"finished-plan");
        assert_eq!(std::fs::read_dir(cache).unwrap().count(), 1);
    }
}
