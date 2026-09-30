//! Plain project-owned directories for job recovery and quarantine.

use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

pub(super) fn validate_jobs_dir(jobs_dir: &Path) -> Result<(), CutError> {
    let project_dir = jobs_dir.parent().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "job directory has no project parent",
            "an attached project directory is required",
        )
    })?;
    cut_core::store::validate_job_output_roots(project_dir)
}

pub(super) fn prepare_jobs_dir(jobs_dir: &Path) -> Result<(), CutError> {
    validate_jobs_dir(jobs_dir)?;
    create_child(jobs_dir)?;
    validate_jobs_dir(jobs_dir)
}

pub(super) fn prepare_quarantine_dir(jobs_dir: &Path) -> Result<PathBuf, CutError> {
    // Check both entries before creating anything, including a quarantine root
    // that is already linked even when all recovered records would be valid.
    validate_jobs_dir(jobs_dir)?;
    let quarantine_dir = jobs_dir.join("quarantine");
    create_child(&quarantine_dir)?;
    validate_jobs_dir(jobs_dir)?;
    Ok(quarantine_dir)
}

fn create_child(path: &Path) -> Result<(), CutError> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}
