//! Strict, private frame-grid qualification for a future pause source writer.
//!
//! Legacy v1 projection remains intentionally available for journals without
//! cadence/probe evidence. This companion plan is opt-in and fail-closed: it
//! turns the backend request into an output policy only after every sealed
//! screen fragment proves the same exact frame rate and frame count.

use cut_core::{error_codes, CutError};
use record_core::{CaptureCadence, FrameRate, Settings};
use record_recovery::{
    quantize_run_aware_stitch, FrameGridStitchPlan, RecordingSessionJournal, RunAwareStitchSpan,
};

use super::pause_projection::{
    plan_legacy_root_projection, LegacyRootProjectionPlan, SealedLegacyProjectionRun,
};

/// An expected CFR source policy paired with the legacy output plan. It is not
/// media verification: a future writer must still prove output frame count/rate
/// before publishing any artifact.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrameGridLegacyRootProjectionPlan {
    legacy: LegacyRootProjectionPlan,
    source_grid: FrameGridStitchPlan,
}

impl FrameGridLegacyRootProjectionPlan {
    pub(crate) fn legacy(&self) -> &LegacyRootProjectionPlan {
        &self.legacy
    }

    pub(crate) fn source_grid(&self) -> &FrameGridStitchPlan {
        &self.source_grid
    }
}

/// Build a strict source-grid plan only from exact cadence and per-fragment
/// probe facts. Missing v1 facts deliberately fail here rather than being
/// reconstructed from legacy `Settings.fps`.
pub(crate) fn plan_frame_grid_legacy_root_projection(
    journal: &RecordingSessionJournal,
    event_runs: &[SealedLegacyProjectionRun],
) -> Result<FrameGridLegacyRootProjectionPlan, CutError> {
    let replayed = RecordingSessionJournal::replay(journal.entries()).map_err(replay_error)?;
    let legacy = plan_legacy_root_projection(&replayed, event_runs)?;
    let output_policy =
        strict_output_policy(
            replayed.intent().capture_cadence.as_ref().ok_or_else(|| {
                invalid("frame-grid projection requires immutable capture cadence")
            })?,
        )?;
    validate_settings(legacy.merged_events().settings, output_policy)?;
    let source_grid = quantize_run_aware_stitch(legacy.source_stitch(), output_policy)
        .map_err(|error| invalid(format!("frame-grid source policy is invalid: {error}")))?;
    validate_fragment_evidence(&replayed, &source_grid)?;
    Ok(FrameGridLegacyRootProjectionPlan {
        legacy,
        source_grid,
    })
}

fn strict_output_policy(cadence: &CaptureCadence) -> Result<FrameRate, CutError> {
    let rate = cadence.backend_requested;
    if rate.den != 1 || !(1..=240).contains(&rate.num) {
        return Err(invalid(
            "frame-grid projection requires an admitted integer backend frame policy",
        ));
    }
    Ok(rate)
}

fn validate_settings(settings: Settings, output_policy: FrameRate) -> Result<(), CutError> {
    if settings.fps != output_policy.num as f32 {
        return Err(invalid(
            "legacy projection settings drift from the exact backend frame policy",
        ));
    }
    Ok(())
}

fn validate_fragment_evidence(
    journal: &RecordingSessionJournal,
    source_grid: &FrameGridStitchPlan,
) -> Result<(), CutError> {
    for grid_span in &source_grid.spans {
        let RunAwareStitchSpan::Source {
            run_sequence,
            checkpoint_sequence,
            artifact,
            sha256,
            ..
        } = &grid_span.span
        else {
            continue;
        };
        let fragment = journal
            .sealed_runs()
            .iter()
            .find(|run| run.sequence == *run_sequence)
            .and_then(|run| {
                run.fragments.iter().find(|fragment| {
                    fragment.stream == record_recovery::RecordingStream::ScreenVideo
                        && fragment.checkpoint_sequence == Some(*checkpoint_sequence)
                        && fragment.artifact == *artifact
                        && fragment.sha256 == *sha256
                })
            })
            .ok_or_else(|| {
                invalid("frame-grid source span is not backed by a sealed screen fragment")
            })?;
        if fragment.facts.avg_frame_rate != Some(source_grid.output_frame_rate)
            || fragment.facts.r_frame_rate != Some(source_grid.output_frame_rate)
            || fragment.facts.decoded_video_frames != Some(grid_span.frame_count())
        {
            return Err(invalid(
                "sealed screen fragment probe cadence or decoded frame count drifts from frame-grid policy",
            ));
        }
    }
    Ok(())
}

fn replay_error(error: record_recovery::SessionJournalError) -> CutError {
    invalid(format!("frame-grid journal replay failed: {error}"))
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot qualify pause projection on an exact frame grid",
        detail.into(),
    )
    .with_suggested_action(
        "retain sealed fragments and require matching verified cadence before source assembly",
    )
}

#[cfg(test)]
#[path = "pause_projection_frame_grid_tests.rs"]
mod tests;
