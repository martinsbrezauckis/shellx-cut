//! Test-only finalization fault seams; production always uses real durability.

use std::fs::File;

use record_core::Result;

use super::{
    finalize_closed, hooks::FinalizationHooks, CameraCaptureDirectory, CameraMediaProbe,
    CameraMediaSeal, StagedCameraMedia,
};
use crate::camera_finalization_anchored::AnchoredDirectory;
use crate::camera_finalization_durability::{sync_publication_directory, sync_published_file};

#[allow(
    clippy::too_many_arguments,
    reason = "test-only durability barriers remain independently injectable"
)]
pub(crate) fn finalize_after_close_for_test(
    probe: &impl CameraMediaProbe,
    capture: &CameraCaptureDirectory,
    staged: &StagedCameraMedia,
    closed_stage: File,
    after_paths_anchored: impl FnOnce() -> Result<()>,
    after_staged_hash: impl FnOnce() -> Result<()>,
    after_link: impl FnOnce() -> Result<()>,
    after_publication_verify: impl FnOnce() -> Result<()>,
) -> Result<CameraMediaSeal> {
    finalize_closed(
        probe,
        capture,
        staged,
        closed_stage,
        FinalizationHooks {
            after_paths_anchored,
            after_staged_hash,
            after_link,
            after_publication_verify,
            sync_file: sync_published_file,
            after_file_sync: || Ok(()),
            sync_directory: sync_publication_directory,
            after_directory_sync: || Ok(()),
        },
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "test-only durability barriers remain independently injectable"
)]
pub(crate) fn finalize_after_close_with_sync_for_test(
    probe: &impl CameraMediaProbe,
    capture: &CameraCaptureDirectory,
    staged: &StagedCameraMedia,
    closed_stage: File,
    after_paths_anchored: impl FnOnce() -> Result<()>,
    after_staged_hash: impl FnOnce() -> Result<()>,
    after_link: impl FnOnce() -> Result<()>,
    after_publication_verify: impl FnOnce() -> Result<()>,
    sync_file: impl FnOnce(&File) -> Result<()>,
    sync_directory: impl FnOnce(&AnchoredDirectory) -> Result<()>,
) -> Result<CameraMediaSeal> {
    finalize_closed(
        probe,
        capture,
        staged,
        closed_stage,
        FinalizationHooks {
            after_paths_anchored,
            after_staged_hash,
            after_link,
            after_publication_verify,
            sync_file,
            after_file_sync: || Ok(()),
            sync_directory,
            after_directory_sync: || Ok(()),
        },
    )
}

pub(crate) fn finalize_after_close_with_post_sync_for_test(
    probe: &impl CameraMediaProbe,
    capture: &CameraCaptureDirectory,
    staged: &StagedCameraMedia,
    closed_stage: File,
    after_file_sync: impl FnOnce() -> Result<()>,
    after_directory_sync: impl FnOnce() -> Result<()>,
) -> Result<CameraMediaSeal> {
    finalize_closed(
        probe,
        capture,
        staged,
        closed_stage,
        FinalizationHooks {
            after_paths_anchored: || Ok(()),
            after_staged_hash: || Ok(()),
            after_link: || Ok(()),
            after_publication_verify: || Ok(()),
            sync_file: sync_published_file,
            after_file_sync,
            sync_directory: sync_publication_directory,
            after_directory_sync,
        },
    )
}
