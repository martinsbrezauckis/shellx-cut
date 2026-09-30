//! Private matte staging: runners never open a final cache leaf for writing.

use cut_core::{error_codes, CutError};
use std::io::Write;
use std::path::{Path, PathBuf};

fn checked_destination(path: &Path) -> Result<(), CutError> {
    let dir = path
        .parent()
        .ok_or_else(|| super::io_err("cache leaf", "missing parent"))?;
    let project = dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| super::io_err("cache leaf", "missing project"))?;
    if cut_core::matte_cache::cache_dir(project, false)? != dir {
        return Err(super::io_err(
            "cache leaf",
            "destination is outside cache/matte",
        ));
    }
    cut_core::matte_cache::plain_file_exists(path)?;
    Ok(())
}

pub(super) fn cache_hit(path: &Path) -> Result<bool, CutError> {
    checked_destination(path)?;
    Ok(cut_core::matte_cache::plain_file_exists(path)? && std::fs::metadata(path)?.len() > 0)
}

pub(super) struct StagedOutput {
    _directory: tempfile::TempDir,
    path: PathBuf,
    final_path: PathBuf,
}

impl StagedOutput {
    pub(super) fn new(final_path: &Path) -> Result<Self, CutError> {
        checked_destination(final_path)?;
        let directory = tempfile::Builder::new()
            .prefix(".cut-matte-")
            .tempdir_in(final_path.parent().unwrap())?;
        let extension = final_path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| super::io_err("stage extension", "missing output extension"))?;
        let path = directory.path().join(format!("output.{extension}"));
        Ok(Self {
            _directory: directory,
            path,
            final_path: final_path.to_owned(),
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), CutError> {
        cut_core::matte_cache::require_plain_directory(self._directory.path())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    pub(super) fn validate(&self) -> Result<(), CutError> {
        checked_destination(&self.final_path)?;
        cut_core::matte_cache::require_plain_directory(self._directory.path())?;
        if !cut_core::matte_cache::plain_file_exists(&self.path)?
            || std::fs::metadata(&self.path)?.len() == 0
        {
            return Err(CutError::new(
                error_codes::IO,
                "matte runner completed without plain output bytes",
                "the owned matte stage must contain a nonempty regular file",
            ));
        }
        Ok(())
    }

    pub(super) fn publish(self) -> Result<(), CutError> {
        self.validate()?;
        // This replaces a directory entry rather than opening/truncating the
        // old inode. Existing linked leaves are refused even before replacement.
        crate::output_paths::publish_output_atomic(&self.path, &self.final_path)
    }
}

pub(super) fn bake_seed(
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

/// Publish only a completed alpha/receipt pair. Retire the prior plain receipt
/// just before publishing so a failed second rename cannot certify new alpha
/// bytes with a stale receipt. Failed bakes leave both previous finals intact.
pub(super) fn publish_pair(alpha: StagedOutput, receipt: StagedOutput) -> Result<(), CutError> {
    alpha.validate()?;
    receipt.validate()?;
    if cut_core::matte_cache::plain_file_exists(&receipt.final_path)? {
        std::fs::remove_file(&receipt.final_path)?;
    }
    alpha.publish()?;
    receipt.publish()
}
