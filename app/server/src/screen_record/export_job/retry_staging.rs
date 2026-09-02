//! Private, immutable retry inputs for recorder export workers.
//!
//! A retry must not merely hash a live path and then let a queued worker reopen
//! it. Each relevant source is copied while it is hashed into an owned private
//! stage; FFmpeg receives only those staged paths.

use super::ExportFormat;
use crate::jobs::{
    JobInputFingerprint, ScreenRecordExportRetryDescriptor, ScreenRecordExportRetryFormat,
};
use crate::screen_record::{screen_record_cache_dir, CaptureExportAudio};
use cut_core::{error_codes, CutError};
use record_recovery::{is_plain_regular_file, PrivateStaging};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Private retry copies retained by `PreparedExport` until the queued renderer
/// has completed. The fields point only at files controlled by `_stages`.
pub(super) struct RetryInputStaging {
    pub(super) source: PathBuf,
    pub(super) plan: PathBuf,
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
    format: ExportFormat,
) -> Result<RetryInputStaging, CutError> {
    tokio::task::spawn_blocking(move || {
        stage_retry_inputs(&dir, revision, &source, &plan, &capture_audio, &format)
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
    let (plan, plan_input, plan_stage) = copy_input(
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
    inputs.sort_by(|left, right| left.role.cmp(&right.role).then(left.path.cmp(&right.path)));

    Ok(RetryInputStaging {
        source,
        plan,
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
    format: &ExportFormat,
) -> Result<ScreenRecordExportRetryDescriptor, CutError> {
    let mut inputs = vec![
        fingerprint_input(dir, "recording_source", source)?,
        fingerprint_input(dir, "edit_plan", plan)?,
    ];
    for (role, path) in capture_audio.retry_inputs() {
        inputs.push(fingerprint_input(dir, role, path)?);
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

fn copy_input(
    project_dir: &Path,
    stage_parent: &Path,
    role: &str,
    source: &Path,
    stage_leaf: String,
) -> Result<(PathBuf, JobInputFingerprint, PrivateStaging), CutError> {
    if !is_plain_regular_file(source).map_err(|error| retry_io_error(source, error))? {
        return Err(retry_conflict(
            "retry inputs must remain local regular files",
        ));
    }
    let logical_path = project_relative_path(project_dir, source)?;
    let stage = PrivateStaging::create(stage_parent, "recorder-retry-input", &stage_leaf)
        .map_err(|error| retry_io_error(stage_parent, error))?;
    let mut input = open_regular_nofollow(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.path())
        .map_err(|error| retry_io_error(stage.path(), error))?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| retry_io_error(source, error))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|error| retry_io_error(stage.path(), error))?;
        bytes = bytes
            .checked_add(u64::try_from(count).expect("read count fits u64"))
            .ok_or_else(|| retry_conflict("retry input byte count overflowed"))?;
    }
    output
        .sync_all()
        .map_err(|error| retry_io_error(stage.path(), error))?;
    Ok((
        stage.path().to_path_buf(),
        JobInputFingerprint {
            role: role.to_string(),
            path: logical_path,
            bytes,
            sha256: format!("{:x}", hash.finalize()),
        },
        stage,
    ))
}

fn fingerprint_input(dir: &Path, role: &str, path: &Path) -> Result<JobInputFingerprint, CutError> {
    // First export retains its established renderer-path behavior. Only the
    // retry admission path below binds inputs to private no-follow snapshots.
    let mut file = File::open(path).map_err(|error| retry_io_error(path, error))?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| retry_io_error(path, error))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes = bytes
            .checked_add(u64::try_from(count).expect("read count fits u64"))
            .ok_or_else(|| retry_conflict("retry input byte count overflowed"))?;
    }
    Ok(JobInputFingerprint {
        role: role.to_string(),
        path: project_relative_path(dir, path)?,
        bytes,
        sha256: format!("{:x}", hash.finalize()),
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

fn stage_leaf(role: &str, path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .unwrap_or("media");
    format!("{role}.{extension}")
}

/// Validate the literal input before opening it, then prove the opened handle
/// is still a local regular file. Unix uses O_NOFOLLOW; Windows asks for the
/// reparse point itself and rejects it from the opened metadata.
fn open_regular_nofollow(path: &Path) -> Result<File, CutError> {
    if !is_plain_regular_file(path).map_err(|error| retry_io_error(path, error))? {
        return Err(retry_conflict(
            "retry inputs must remain local regular files",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|error| retry_io_error(path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| retry_io_error(path, error))?;
    if is_open_plain_regular(&metadata) {
        Ok(file)
    } else {
        Err(retry_conflict(
            "retry input changed while opening without following links",
        ))
    }
}

fn is_open_plain_regular(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
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

fn retry_io_error(path: &Path, error: impl std::fmt::Display) -> CutError {
    CutError::new(
        error_codes::IO,
        format!("could not stage retry input {}: {error}", path.display()),
        "restore the recorder export input or start a new export",
    )
}
fn retry_staging_join_error(error: tokio::task::JoinError) -> CutError {
    CutError::new(
        error_codes::JOB_FAILED,
        if error.is_panic() {
            "recorder retry input staging worker panicked"
        } else {
            "recorder retry input staging worker was cancelled"
        },
        error.to_string(),
    )
    .with_suggested_action("retry the export after project storage is available")
}

fn retry_conflict(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "screen-record export retry is not safe to admit",
        cause.into(),
    )
}
