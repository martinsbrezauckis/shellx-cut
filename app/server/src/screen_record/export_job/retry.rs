//! Exact-input admission for one failed default-output recorder export.

pub(super) use super::retry_staging::retry_descriptor;
use super::retry_staging::stage_retry_inputs_async;
use super::*;

/// Admit a fresh child only after the revision and every renderer input still
/// match the failed job's persisted descriptor.
pub(crate) async fn retry_screen_record_export(
    state: &AppState,
    source_job_id: &str,
) -> Result<VerbResult, CutError> {
    let record = state.jobs.get(source_job_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("no job '{source_job_id}'"),
            "select a job returned by jobs.list",
        )
    })?;
    if record.kind != "screen_record_export" {
        return Err(retry_conflict(
            "only screen_record.export jobs have a retry implementation",
        ));
    }
    let descriptor = match record.retry.and_then(|retry| retry.descriptor) {
        Some(crate::jobs::JobRetryDescriptor::ScreenRecordExport(descriptor)) => descriptor,
        _ => {
            return Err(retry_conflict(
                "this export used an explicit Save As path or predates retry support",
            ))
        }
    };
    let (_project, _edl, dir, revision) = snapshot(state).await?;
    if revision != descriptor.project_revision {
        return Err(retry_conflict(
            "project revision changed since the failed export; start a new export instead",
        ));
    }
    let format = export_format_from_descriptor(&descriptor.format);
    let (source, plan, capture_audio) =
        resolve_export_inputs(&dir, &descriptor.source, &descriptor.plan)?;
    let staged = stage_retry_inputs_async(
        dir.clone(),
        revision.clone(),
        source,
        plan,
        capture_audio,
        format.clone(),
    )
    .await?;
    if !staged.matches_descriptor(&descriptor) {
        return Err(retry_conflict(
            "recording source, edit plan, or capture-audio inputs changed since the failed export",
        ));
    }
    let mut prepared = allocate_export(
        &dir,
        staged.source.clone(),
        staged.plan.clone(),
        staged.capture_audio.clone(),
        format,
        None,
    )?;
    prepared._retry_input_staging = Some(staged);
    let output_path = prepared.output_path.clone();
    let format_name = prepared.format.name();
    let child = admit_retry_at_revision(state, source_job_id, &dir, &revision)?;
    let job_id = child.job_id.clone();
    let retry = child.retry.as_ref().expect("retry child has projection");
    let attempt = retry.attempt;
    let root_job_id = retry.root_job_id.clone();
    spawn_export_job(state, &job_id, prepared);
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "retry_of": source_job_id,
        "root_job_id": root_job_id,
        "attempt": attempt,
        "path": output_path,
        "format": format_name,
        "status": "queued",
    })))
}

/// Hold the project read lock while binding durable admission to the exact log
/// head that was fingerprinted. `jobs.retry` also holds `project_transition`,
/// so a project replacement cannot interleave this final check and spawn.
fn admit_retry_at_revision(
    state: &AppState,
    source_job_id: &str,
    project_dir: &Path,
    expected_revision: &str,
) -> Result<crate::jobs::JobRecord, CutError> {
    let guard = state.project.try_read().map_err(|_| {
        retry_conflict("project changed while retry inputs were being revalidated; try again")
    })?;
    let store = guard
        .as_ref()
        .ok_or_else(|| retry_conflict("no project is open for this retry"))?;
    let operations = store.log.read_all()?;
    let current_revision = operations
        .last()
        .map(|operation| operation.op_id.as_str())
        .unwrap_or("op_000000");
    if store.dir != project_dir || current_revision != expected_revision {
        return Err(retry_conflict(
            "project changed while retry inputs were being revalidated; start a new export instead",
        ));
    }
    state.jobs.admit_retry(source_job_id)
}

fn export_format_from_descriptor(
    format: &crate::jobs::ScreenRecordExportRetryFormat,
) -> ExportFormat {
    match format {
        crate::jobs::ScreenRecordExportRetryFormat::Mp4 => ExportFormat::Mp4,
        crate::jobs::ScreenRecordExportRetryFormat::Gif { fps, width } => ExportFormat::Gif {
            fps: *fps,
            width: *width,
        },
    }
}

fn retry_conflict(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "screen-record export retry is not safe to admit",
        cause.into(),
    )
}
