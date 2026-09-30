//! Deterministic project dubbing outputs. Producers only write owned stages.

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

pub(super) struct Outputs {
    project: PathBuf,
    wav: PathBuf,
    receipt: PathBuf,
}

fn invalid(cause: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "unsafe dubbing output",
        cause.to_string(),
    )
}

impl Outputs {
    pub(super) fn new(project: &Path, asset: &str, lang: &str) -> Result<Self, CutError> {
        // Language names and BCP-47 tags remain valid; only filename syntax is
        // excluded here. Translation keeps its existing language contract.
        for token in [asset, lang] {
            if token.is_empty()
                || token
                    .chars()
                    .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
                || matches!(token, "." | "..")
            {
                return Err(invalid(
                    "asset and language must be safe filename components",
                ));
            }
        }
        let outputs = Self {
            project: project.to_owned(),
            wav: project.join("dub").join(format!("{asset}.{lang}.wav")),
            receipt: project
                .join("receipts")
                .join(format!("{asset}.{lang}.dub.json")),
        };
        outputs.check(&outputs.wav, "dub")?;
        outputs.check(&outputs.receipt, "receipts")?;
        Ok(outputs)
    }

    fn check(&self, path: &Path, relative: &str) -> Result<(), CutError> {
        let parent = crate::output_paths::ensure_plain_project_relative_dir(
            &self.project,
            Path::new(relative),
        )?;
        if path.parent() != Some(parent.as_path()) {
            return Err(invalid("destination is outside its project directory"));
        }
        cut_core::matte_cache::plain_file_exists(path)?;
        Ok(())
    }

    pub(super) fn wav(&self) -> &Path {
        &self.wav
    }

    pub(super) fn stage_wav(&self) -> Result<StagedWav<'_>, CutError> {
        self.check(&self.wav, "dub")?;
        let directory = tempfile::Builder::new()
            .prefix(".cut-dub-")
            .tempdir_in(self.wav.parent().unwrap())?;
        let path = directory.path().join("output.wav");
        Ok(StagedWav {
            outputs: self,
            directory,
            path,
        })
    }

    pub(super) fn write_receipt(&self, bytes: &[u8]) -> Result<(), CutError> {
        self.check(&self.receipt, "receipts")?;
        crate::output_paths::write_output_atomic(&self.receipt, bytes)
    }
}

pub(super) struct StagedWav<'a> {
    outputs: &'a Outputs,
    directory: tempfile::TempDir,
    path: PathBuf,
}

impl StagedWav<'_> {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn publish(self) -> Result<(), CutError> {
        self.outputs.check(&self.outputs.wav, "dub")?;
        cut_core::matte_cache::require_plain_directory(self.directory.path())?;
        if !cut_core::matte_cache::plain_file_exists(&self.path)?
            || std::fs::metadata(&self.path)?.len() == 0
        {
            return Err(invalid(
                "dubbing worker did not produce a nonempty plain WAV",
            ));
        }
        crate::output_paths::publish_output_atomic(&self.path, &self.outputs.wav)
    }
}

#[cfg(test)]
mod tests;
