//! Exact-input admission for one failed default-output recorder export.

use super::*;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;

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
        None => {
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
    let current = retry_descriptor(&dir, revision, &source, &plan, &capture_audio, &format)?;
    if current.inputs != descriptor.inputs
        || current.system_audio_offset_ms != descriptor.system_audio_offset_ms
        || current.format != descriptor.format
    {
        return Err(retry_conflict(
            "recording source, edit plan, or capture-audio inputs changed since the failed export",
        ));
    }
    let prepared = allocate_export(&dir, source, plan, capture_audio, format, None)?;
    let output_path = prepared.output_path.clone();
    let format_name = prepared.format.name();
    let child = state.jobs.admit_retry(source_job_id)?;
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

pub(super) fn retry_descriptor(
    dir: &Path,
    revision: String,
    source: &Path,
    plan: &Path,
    capture_audio: &crate::screen_record::CaptureExportAudio,
    format: &ExportFormat,
) -> Result<crate::jobs::ScreenRecordExportRetryDescriptor, CutError> {
    let mut inputs = vec![
        fingerprint_input(dir, "recording_source", source)?,
        fingerprint_input(dir, "edit_plan", plan)?,
    ];
    for (role, path) in capture_audio.retry_inputs() {
        inputs.push(fingerprint_input(dir, role, path)?);
    }
    inputs.sort_by(|a, b| a.role.cmp(&b.role).then(a.path.cmp(&b.path)));
    Ok(crate::jobs::ScreenRecordExportRetryDescriptor {
        project_revision: revision,
        source: project_relative_path(dir, source)?,
        plan: project_relative_path(dir, plan)?,
        format: match format {
            ExportFormat::Mp4 => crate::jobs::ScreenRecordExportRetryFormat::Mp4,
            ExportFormat::Gif { fps, width } => crate::jobs::ScreenRecordExportRetryFormat::Gif {
                fps: *fps,
                width: *width,
            },
        },
        inputs,
        system_audio_offset_ms: capture_audio.system_audio_offset_ms(),
    })
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

fn fingerprint_input(
    dir: &Path,
    role: &str,
    path: &Path,
) -> Result<crate::jobs::JobInputFingerprint, CutError> {
    let mut file = File::open(path).map_err(|error| retry_io_error(path, error))?;
    let bytes = file
        .metadata()
        .map_err(|error| retry_io_error(path, error))?
        .len();
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| retry_io_error(path, error))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(crate::jobs::JobInputFingerprint {
        role: role.to_string(),
        path: project_relative_path(dir, path)?,
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn project_relative_path(dir: &Path, path: &Path) -> Result<String, CutError> {
    let dir = dir
        .canonicalize()
        .map_err(|error| retry_io_error(dir, error))?;
    let path = path
        .canonicalize()
        .map_err(|error| retry_io_error(path, error))?;
    path.strip_prefix(dir)
        .map_err(|_| retry_conflict("retry inputs must remain inside the active project"))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn retry_io_error(path: &Path, error: std::io::Error) -> CutError {
    CutError::new(
        error_codes::IO,
        format!(
            "could not fingerprint retry input {}: {error}",
            path.display()
        ),
        "restore the recorder export input or start a new export",
    )
}

fn retry_conflict(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "screen-record export retry is not safe to admit",
        cause.into(),
    )
}
