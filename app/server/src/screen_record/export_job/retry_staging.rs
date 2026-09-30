//! Private, immutable retry inputs for recorder export workers.
//!
//! A retry must not merely hash a live path and then let a queued worker reopen
//! it. Each relevant source is copied while it is hashed into an owned private
//! stage; FFmpeg receives only those staged paths.

use super::ExportFormat;
#[path = "retry_input_io.rs"]
mod input_io;
use crate::jobs::{ScreenRecordExportRetryDescriptor, ScreenRecordExportRetryFormat};
use crate::screen_record::{screen_record_cache_dir, CaptureExportAudio};
use cut_core::CutError;
use input_io::*;
use record_recovery::PrivateStaging;
use std::path::{Path, PathBuf};

/// Private retry copies retained by `PreparedExport` until the queued renderer
/// has completed. The fields point only at files controlled by `_stages`.
pub(super) struct RetryInputStaging {
    pub(super) source: PathBuf,
    #[cfg(test)]
    pub(super) plan: PathBuf,
    pub(super) render_plan: record_core::EditPlan,
    pub(super) capture_audio: CaptureExportAudio,
    descriptor: ScreenRecordExportRetryDescriptor,
    _stages: Vec<PrivateStaging>,
}

impl RetryInputStaging {
    pub(super) fn matches_descriptor(&self, expected: &ScreenRecordExportRetryDescriptor) -> bool {
        self.descriptor == *expected
    }

    #[cfg(test)]
    pub(super) fn paths_for_test(&self) -> Vec<PathBuf> {
        self._stages
            .iter()
            .map(|stage| stage.path().to_path_buf())
            .collect()
    }
}
/// Run immutable retry staging on Tokio's blocking pool. Dropped callers leave
/// its queued or returned `PrivateStaging` owned by task output for RAII cleanup.
pub(super) async fn stage_retry_inputs_async(
    dir: PathBuf,
    revision: String,
    source: PathBuf,
    plan: PathBuf,
    capture_audio: CaptureExportAudio,
    render_plan: record_core::EditPlan,
    format: ExportFormat,
) -> Result<RetryInputStaging, CutError> {
    tokio::task::spawn_blocking(move || {
        stage_retry_inputs(
            &dir,
            revision,
            &source,
            &plan,
            &capture_audio,
            render_plan,
            &format,
        )
    })
    .await
    .map_err(retry_staging_join_error)?
}
/// Copy every actual renderer input into a random private stage and calculate
/// the comparison descriptor from exactly those copied bytes. This is called
/// after ordinary project fencing and before output admission; any mismatch
/// drops every stage without reserving an export destination.
pub(super) fn stage_retry_inputs(
    dir: &Path,
    revision: String,
    source: &Path,
    plan: &Path,
    capture_audio: &CaptureExportAudio,
    mut render_plan: record_core::EditPlan,
    format: &ExportFormat,
) -> Result<RetryInputStaging, CutError> {
    let stage_parent = screen_record_cache_dir(dir)?;
    let (source, source_input, source_stage) = copy_input(
        dir,
        &stage_parent,
        "recording_source",
        source,
        stage_leaf("recording_source", source),
    )?;
    let (_plan, plan_input, plan_stage) = copy_input(
        dir,
        &stage_parent,
        "edit_plan",
        plan,
        "edit_plan.json".into(),
    )?;
    let descriptor_source = source_input.path.clone();
    let descriptor_plan = plan_input.path.clone();
    let mut inputs = vec![source_input, plan_input];
    let mut stages = vec![source_stage, plan_stage];
    let mut staged_mic = None;
    let mut staged_system = None;

    for (role, path) in capture_audio.retry_inputs() {
        let (staged, input, stage) = copy_input(
            dir,
            &stage_parent,
            role,
            path,
            match role {
                "capture_mic" => "capture_mic.wav".into(),
                "capture_system" => "capture_system.wav".into(),
                _ => unreachable!("capture retry inputs have fixed roles"),
            },
        )?;
        match role {
            "capture_mic" => staged_mic = Some(staged),
            "capture_system" => staged_system = Some(staged),
            _ => unreachable!("capture retry inputs have fixed roles"),
        }
        inputs.push(input);
        stages.push(stage);
    }
    if let Some(camera) = render_plan.webcam.as_mut() {
        let path = Path::new(&camera.source);
        let (staged, input, stage) = copy_input(
            dir,
            &stage_parent,
            "plan_webcam",
            path,
            stage_leaf("plan_webcam", path),
        )?;
        camera.source = staged.to_string_lossy().into_owned();
        inputs.push(input);
        stages.push(stage);
    }
    inputs.sort_by(|left, right| left.role.cmp(&right.role).then(left.path.cmp(&right.path)));

    Ok(RetryInputStaging {
        source,
        #[cfg(test)]
        plan: _plan,
        render_plan,
        capture_audio: capture_audio.with_staged_retry_inputs(staged_mic, staged_system),
        descriptor: ScreenRecordExportRetryDescriptor {
            project_revision: revision,
            source: descriptor_source,
            plan: descriptor_plan,
            format: retry_format(format),
            inputs,
            system_audio_offset_ms: capture_audio.system_audio_offset_ms(),
        },
        _stages: stages,
    })
}

/// Construct the ordinary first-export descriptor without changing its renderer
/// inputs. Unlike retry staging, that request preserves its existing paths.
pub(super) fn retry_descriptor(
    dir: &Path,
    revision: String,
    source: &Path,
    plan: &Path,
    capture_audio: &CaptureExportAudio,
    render_plan: &record_core::EditPlan,
    format: &ExportFormat,
) -> Result<ScreenRecordExportRetryDescriptor, CutError> {
    let mut inputs = vec![
        fingerprint_input(dir, "recording_source", source)?,
        fingerprint_input(dir, "edit_plan", plan)?,
    ];
    for (role, path) in capture_audio.retry_inputs() {
        inputs.push(fingerprint_input(dir, role, path)?);
    }
    if let Some(camera) = crate::screen_record::plan_inputs::webcam_path(render_plan) {
        inputs.push(fingerprint_input(dir, "plan_webcam", &camera)?);
    }
    inputs.sort_by(|left, right| left.role.cmp(&right.role).then(left.path.cmp(&right.path)));
    Ok(ScreenRecordExportRetryDescriptor {
        project_revision: revision,
        source: project_relative_path(dir, source)?,
        plan: project_relative_path(dir, plan)?,
        format: retry_format(format),
        inputs,
        system_audio_offset_ms: capture_audio.system_audio_offset_ms(),
    })
}

fn retry_format(format: &ExportFormat) -> ScreenRecordExportRetryFormat {
    match format {
        ExportFormat::Mp4 => ScreenRecordExportRetryFormat::Mp4,
        ExportFormat::Gif { fps, width } => ScreenRecordExportRetryFormat::Gif {
            fps: *fps,
            width: *width,
        },
    }
}
