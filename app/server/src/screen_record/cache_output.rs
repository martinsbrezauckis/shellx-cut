//! Private recorder cache publication; final leaves are never opened for writing.

use cut_core::{error_codes, CutError};
use std::io::Write;
use std::path::{Path, PathBuf};

fn checked_destination(path: &Path) -> Result<(), CutError> {
    let dir = path.parent().ok_or_else(|| unsafe_leaf("missing parent"))?;
    let project = dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| unsafe_leaf("missing project"))?;
    if crate::output_paths::ensure_plain_project_relative_dir(
        project,
        Path::new("cache/screen_record"),
    )? != dir
    {
        return Err(unsafe_leaf("destination is outside cache/screen_record"));
    }
    plain_leaf(path)?;
    Ok(())
}

fn unsafe_leaf(cause: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        "unsafe recorder cache entry",
        cause.to_string(),
    )
}

fn plain_leaf(path: &Path) -> Result<bool, CutError> {
    cut_core::matte_cache::plain_file_exists(path).map_err(unsafe_leaf)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), CutError> {
    let stage = StagedOutput::new(path)?;
    stage.write(bytes)?;
    stage.publish()
}

pub(crate) fn cache_hit(path: &Path) -> Result<bool, CutError> {
    checked_destination(path)?;
    Ok(plain_leaf(path)? && std::fs::metadata(path)?.len() > 0)
}

pub(crate) struct StagedOutput {
    _directory: tempfile::TempDir,
    path: PathBuf,
    final_path: PathBuf,
}

impl StagedOutput {
    pub(crate) fn new(final_path: &Path) -> Result<Self, CutError> {
        checked_destination(final_path)?;
        let directory = tempfile::Builder::new()
            .prefix(".cut-recorder-")
            .tempdir_in(final_path.parent().unwrap())?;
        let extension = final_path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| unsafe_leaf("missing output extension"))?;
        let path = directory.path().join(format!("output.{extension}"));
        Ok(Self {
            _directory: directory,
            path,
            final_path: final_path.to_owned(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), CutError> {
        cut_core::matte_cache::require_plain_directory(self._directory.path())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), CutError> {
        checked_destination(&self.final_path)?;
        cut_core::matte_cache::require_plain_directory(self._directory.path())?;
        if !plain_leaf(&self.path)? || std::fs::metadata(&self.path)?.len() == 0 {
            return Err(CutError::new(
                error_codes::IO,
                "recorder renderer completed without plain output bytes",
                "the owned recorder stage must contain a nonempty regular file",
            ));
        }
        Ok(())
    }

    pub(crate) fn publish(self) -> Result<(), CutError> {
        self.validate()?;
        // This replaces a directory entry rather than opening/truncating the
        // old inode. Existing linked leaves are refused even before replacement.
        crate::output_paths::publish_output_atomic(&self.path, &self.final_path)
    }
}

pub(crate) fn bake_seed(
    final_path: &Path,
    bake: impl FnOnce(&Path) -> Result<(), CutError>,
) -> Result<PathBuf, CutError> {
    if cache_hit(final_path)? {
        return Ok(final_path.to_owned());
    }
    let stage = StagedOutput::new(final_path)?;
    bake(stage.path())?;
    stage.publish()?;
    Ok(final_path.to_owned())
}

#[cfg(test)]
#[path = "cache_output_tests.rs"]
mod tests;
