//! Owned external render phases.
//!
//! The legacy dispatch router deliberately stays policy-only. This leaf binds
//! all Python sidecars and ffmpeg render work to the current server job before
//! returning a result to that router.

use super::process::run_owned_render;
use crate::jobs::JobCancellation;
use cut_core::CutError;
use std::path::PathBuf;

pub(super) fn run_render<T>(
    cancellation: JobCancellation,
    render: impl FnOnce() -> Result<T, CutError>,
) -> Result<T, CutError> {
    run_owned_render(cancellation, render)
}

pub(super) fn output_facts(
    cancellation: JobCancellation,
    output: PathBuf,
    receipts: PathBuf,
    render_id: String,
    hash: String,
    duration_ms: u64,
) -> Result<cut_perception::RenderFacts, CutError> {
    with_sidecar(cancellation, |control| {
        let report = cut_perception::run_instruments_owned(
            &output,
            &receipts,
            &format!("{render_id}.output"),
            &hash,
            cut_perception::InstrumentSet::RenderChecks,
            None,
            control,
        )?;
        Ok(cut_perception::RenderFacts {
            duration_ms,
            loudness: report.loudness.clone(),
            output_report: Some(report),
        })
    })
}

pub(super) fn contact_sheet(
    cancellation: JobCancellation,
    media: PathBuf,
    out_dir: PathBuf,
    preset: String,
) -> Result<serde_json::Value, CutError> {
    with_sidecar(cancellation, |control| {
        cut_perception::build_contact_sheet_owned(&media, &out_dir, &preset, control)
    })
}

pub(super) fn qc_sheet(
    cancellation: JobCancellation,
    media: PathBuf,
    out_dir: PathBuf,
    preset: String,
) -> Result<serde_json::Value, CutError> {
    with_sidecar(cancellation, |control| {
        cut_perception::build_qc_sheet_owned(&media, &out_dir, &preset, control)
    })
}

pub(super) fn subject_track(
    cancellation: JobCancellation,
    media: PathBuf,
    hash: String,
    preset: String,
    direction: Option<serde_json::Value>,
) -> Result<cut_perception::types::SubjectTrack, CutError> {
    with_sidecar(cancellation, |control| {
        cut_perception::run_subject_owned(&media, &hash, &preset, direction, control)
    })
}

fn with_sidecar<T>(
    cancellation: JobCancellation,
    run: impl FnOnce(&cut_media::ffmpeg::OwnedProcessControl) -> Result<T, CutError>,
) -> Result<T, CutError> {
    let control = crate::dispatch::owned_job_process_control(cancellation);
    run(&control)
}
