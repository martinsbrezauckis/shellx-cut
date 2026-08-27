//! Private, restart-safe publication of a completed pause-session projection.
//!
//! This executor deliberately has no server or worker caller. A future native
//! owner supplies the source stager only after it has sealed every selected run.

mod artifacts;
mod output;
mod stager;
mod types;

use cut_core::{error_codes, CutError};
use record_recovery::{
    CaptureRoot, PrivateStaging, RecordingSessionJournal, RecordingSessionJournalFile,
};

use super::pause_projection::{plan_legacy_root_projection, SealedLegacyProjectionRun};
use super::pause_projection_frame_grid::plan_frame_grid_legacy_root_projection;
// This is the private integration seam for the eventual native owner. The
// current server has no caller, so the re-exports intentionally remain unused.
#[allow(unused_imports)]
pub(crate) use stager::{
    FfmpegFrameGridPauseProjectionSourceStager, FrameGridPauseProjectionSourceStager,
};
#[allow(unused_imports)]
pub(crate) use types::{
    FfmpegPauseProjectionArtifactVerifier, PauseProjectionArtifactVerifier,
    PauseProjectionExecution, PauseProjectionResult, PauseProjectionSourceStager,
    ProjectionMediaFacts, StagedPauseProjectionSource, VerifiedPauseProjectionSource,
};

pub(crate) fn execute_pause_projection<V, S>(
    root: &CaptureRoot,
    capture_id: &str,
    journal: &RecordingSessionJournal,
    event_runs: &[SealedLegacyProjectionRun],
    verifier: &V,
    stager: &mut S,
) -> Result<PauseProjectionResult, CutError>
where
    V: PauseProjectionArtifactVerifier,
    S: PauseProjectionSourceStager,
{
    let durable = RecordingSessionJournalFile::replay(root, capture_id).map_err(journal_error)?;
    if durable != *journal {
        return Err(invalid(
            "the supplied session journal differs from durable replay evidence",
        ));
    }
    let plan = plan_legacy_root_projection(journal, event_runs)?;
    let paths = output::OutputPaths::new(root, capture_id)?;
    let payloads = output::ProjectionPayloads::from_plan(&plan)?;
    let sources =
        artifacts::verify_sources(paths.capture_dir(), journal, plan.source_stitch(), verifier)?;
    if let Some(result) =
        output::completed(&paths, journal, &payloads, plan.source_stitch().duration_ms)?
    {
        return Ok(result);
    }
    let staging = PrivateStaging::create(paths.capture_dir(), "pause-projection", "source.mp4")
        .map_err(|error| invalid(format!("reserve private pause projection stage: {error}")))?;
    let staged = stager.stage(&sources, plan.source_stitch(), staging.path())?;
    let staged_media = verifier.verify(staging.path())?;
    artifacts::validate_staged_source(
        staging.path(),
        staged,
        staged_media,
        plan.source_stitch().duration_ms,
    )?;
    output::publish(
        &paths,
        journal,
        &payloads,
        staging.path(),
        plan.source_stitch().duration_ms,
    )
}

/// Execute the strictly qualified private source path for a completed pause
/// session. Unlike the compatibility executor above, this path requires a
/// frame-grid plan and proves the staged output's exact rate/frame count before
/// no-replace publication. It remains deliberately unwired to a public verb or
/// native capture lifecycle.
pub(crate) fn execute_frame_grid_pause_projection<V, S>(
    root: &CaptureRoot,
    capture_id: &str,
    journal: &RecordingSessionJournal,
    event_runs: &[SealedLegacyProjectionRun],
    verifier: &V,
    stager: &mut S,
) -> Result<PauseProjectionResult, CutError>
where
    V: PauseProjectionArtifactVerifier,
    S: FrameGridPauseProjectionSourceStager,
{
    let durable = RecordingSessionJournalFile::replay(root, capture_id).map_err(journal_error)?;
    if durable != *journal {
        return Err(invalid(
            "the supplied session journal differs from durable replay evidence",
        ));
    }
    let qualified = plan_frame_grid_legacy_root_projection(journal, event_runs)?;
    let paths = output::OutputPaths::new(root, capture_id)?;
    let payloads = output::ProjectionPayloads::from_plan(qualified.legacy())?;
    let sources = artifacts::verify_sources(
        paths.capture_dir(),
        journal,
        qualified.legacy().source_stitch(),
        verifier,
    )?;
    let logical_duration_ms = qualified.legacy().source_stitch().duration_ms;
    if let Some(result) = output::completed(&paths, journal, &payloads, logical_duration_ms)? {
        let media = verifier.verify(paths.source())?;
        artifacts::validate_frame_grid_staged_source(
            paths.source(),
            media,
            qualified.source_grid(),
        )?;
        return Ok(result);
    }
    let staging = PrivateStaging::create(paths.capture_dir(), "pause-projection", "source.mp4")
        .map_err(|error| invalid(format!("reserve private pause projection stage: {error}")))?;
    stager.stage_frame_grid(
        paths.capture_dir(),
        &sources,
        qualified.source_grid(),
        staging.path(),
    )?;
    let staged_media = verifier.verify(staging.path())?;
    artifacts::validate_frame_grid_staged_source(
        staging.path(),
        staged_media,
        qualified.source_grid(),
    )?;
    output::publish(
        &paths,
        journal,
        &payloads,
        staging.path(),
        logical_duration_ms,
    )
}

fn journal_error(error: record_recovery::SessionJournalError) -> CutError {
    invalid(format!(
        "durable pause-session journal cannot be replayed: {error}"
    ))
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot execute pause-session legacy projection",
        detail.into(),
    )
    .with_suggested_action(
        "retain the sealed session evidence and retry only with exact completed run inputs",
    )
}

#[cfg(test)]
#[path = "pause_projection_executor_exact_tests.rs"]
mod exact_tests;
#[cfg(test)]
#[path = "pause_projection_executor_tests.rs"]
mod tests;
