use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use cut_core::{error_codes, CutError};
use record_recovery::{
    is_plain_dir, is_plain_regular_file, FrameGridStitchPlan, PrivateStaging,
    RecordingSessionJournal, RecordingStream, RunAwareStitchPlan,
};
use sha2::{Digest, Sha256};

use super::types::{
    source_spans, PauseProjectionArtifactVerifier, ProjectionMediaFacts,
    StagedPauseProjectionSource, VerifiedPauseProjectionSource,
};

pub(super) fn verify_sources<V>(
    capture_dir: &Path,
    journal: &RecordingSessionJournal,
    plan: &RunAwareStitchPlan,
    verifier: &V,
) -> Result<Vec<VerifiedPauseProjectionSource>, CutError>
where
    V: PauseProjectionArtifactVerifier,
{
    if !is_plain_dir(capture_dir).map_err(source_error)? {
        return Err(invalid("the capture root is not a local plain directory"));
    }
    source_spans(plan)
        .map(
            |(
                run_sequence,
                checkpoint_sequence,
                artifact,
                expected_sha256,
                expected_duration_ms,
            )| {
                let fragment = journal
                    .sealed_runs()
                    .iter()
                    .find(|run| run.sequence == run_sequence)
                    .and_then(|run| {
                        run.fragments.iter().find(|fragment| {
                            fragment.stream == RecordingStream::ScreenVideo
                                && fragment.checkpoint_sequence == Some(checkpoint_sequence)
                                && fragment.artifact == artifact
                        })
                    })
                    .ok_or_else(|| {
                        invalid("source stitch plan is not backed by a sealed screen artifact")
                    })?;
                let path = contained_artifact(capture_dir, artifact)?;
                let metadata = open_read_nofollow(&path)
                    .and_then(|file| file.metadata().map_err(source_error))?;
                if fragment.sha256 != expected_sha256
                    || fragment.facts.media_duration_ms != expected_duration_ms
                    || metadata.len() != fragment.bytes
                    || sha256(&path)? != fragment.sha256
                {
                    return Err(invalid(
                        "sealed screen artifact bytes or SHA-256 do not match journal facts",
                    ));
                }
                let actual = verifier.verify(&path)?;
                match_media_facts(expected_duration_ms, &fragment.facts, actual)?;
                Ok(VerifiedPauseProjectionSource {
                    run_sequence,
                    checkpoint_sequence,
                    path,
                    bytes: fragment.bytes,
                    sha256: fragment.sha256.clone(),
                    duration_ms: fragment.facts.media_duration_ms,
                    decoded_video_frames: actual.decoded_video_frames,
                    avg_frame_rate: actual.avg_frame_rate,
                    r_frame_rate: actual.r_frame_rate,
                })
            },
        )
        .collect()
}

pub(super) fn validate_staged_source(
    path: &Path,
    staged: StagedPauseProjectionSource,
    actual: ProjectionMediaFacts,
    expected_duration_ms: u64,
) -> Result<(), CutError> {
    if staged.duration_ms != expected_duration_ms
        || actual.duration_ms != expected_duration_ms
        || actual.has_audio
    {
        return Err(invalid(
            "staged source duration differs from the compact sealed timeline",
        ));
    }
    let metadata =
        open_read_nofollow(path).and_then(|file| file.metadata().map_err(source_error))?;
    if !is_plain_regular_file(path).map_err(source_error)? || metadata.len() == 0 {
        return Err(invalid(
            "staged source is not a non-empty local regular file",
        ));
    }
    Ok(())
}

/// Copy every just-verified source into a fresh private local stage before the
/// writer opens it. The copy's hash and byte count are checked while the source
/// descriptor is held, so a path replacement after the first verification
/// cannot change the media FFmpeg assembles.
pub(super) fn snapshot_sources(
    capture_dir: &Path,
    sources: &[VerifiedPauseProjectionSource],
) -> Result<Vec<PrivateStaging>, CutError> {
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| snapshot_source(capture_dir, index, source))
        .collect()
}

pub(super) fn validate_frame_grid_staged_source(
    path: &Path,
    actual: ProjectionMediaFacts,
    grid: &FrameGridStitchPlan,
) -> Result<(), CutError> {
    let expected_duration_ms = rounded_duration_ms(grid)?;
    if actual.has_audio
        || actual.decoded_video_frames != grid.output_frame_count
        || actual.avg_frame_rate != Some(grid.output_frame_rate)
        || actual.r_frame_rate != Some(grid.output_frame_rate)
        || actual.duration_ms != expected_duration_ms
    {
        return Err(invalid(
            "staged source does not prove the exact qualified frame-grid policy",
        ));
    }
    let metadata =
        open_read_nofollow(path).and_then(|file| file.metadata().map_err(source_error))?;
    if !is_plain_regular_file(path).map_err(source_error)? || metadata.len() == 0 {
        return Err(invalid(
            "staged source is not a non-empty local regular file",
        ));
    }
    Ok(())
}

fn contained_artifact(capture_dir: &Path, artifact: &str) -> Result<PathBuf, CutError> {
    let mut path = capture_dir.to_path_buf();
    let components = Path::new(artifact).components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(
            "sealed artifact path is not a literal relative path",
        ));
    }
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            unreachable!()
        };
        path.push(component);
        let local = if index + 1 == components.len() {
            is_plain_regular_file(&path).map_err(source_error)?
        } else {
            is_plain_dir(&path).map_err(source_error)?
        };
        if !local {
            return Err(invalid(
                "sealed artifact path is missing, linked, or not local",
            ));
        }
    }
    Ok(path)
}

fn snapshot_source(
    capture_dir: &Path,
    index: usize,
    source: &VerifiedPauseProjectionSource,
) -> Result<PrivateStaging, CutError> {
    if source.path.parent() != Some(capture_dir) && !source.path.starts_with(capture_dir) {
        return Err(invalid("verified source escaped its capture directory"));
    }
    let stage = PrivateStaging::create(capture_dir, "pause-source-input", "input.mp4").map_err(
        |error| {
            invalid(format!(
                "reserve private pause source input {index}: {error}"
            ))
        },
    )?;
    let mut input = open_read_nofollow(source.path())?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage.path())
        .map_err(source_error)?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(source_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        output.write_all(&buffer[..read]).map_err(source_error)?;
        bytes = bytes
            .checked_add(u64::try_from(read).map_err(|_| invalid("source byte count overflow"))?)
            .ok_or_else(|| invalid("source byte count overflow"))?;
    }
    output.sync_all().map_err(source_error)?;
    if bytes != source.bytes || format!("{:x}", digest.finalize()) != source.sha256 {
        return Err(invalid(
            "sealed source path or bytes drifted before private staging",
        ));
    }
    Ok(stage)
}

fn match_media_facts(
    expected_duration_ms: u64,
    expected: &record_recovery::StreamFragmentFacts,
    actual: ProjectionMediaFacts,
) -> Result<(), CutError> {
    if actual.has_audio
        || actual.duration_ms != expected_duration_ms
        || expected.decoded_video_frames != Some(actual.decoded_video_frames)
        || expected
            .avg_frame_rate
            .is_some_and(|rate| actual.avg_frame_rate != Some(rate))
        || expected
            .r_frame_rate
            .is_some_and(|rate| actual.r_frame_rate != Some(rate))
    {
        return Err(invalid(
            "sealed screen artifact media facts differ from journal evidence",
        ));
    }
    Ok(())
}

fn rounded_duration_ms(grid: &FrameGridStitchPlan) -> Result<u64, CutError> {
    let numerator = u128::from(grid.expected_duration_ms.numerator);
    let denominator = u128::from(grid.expected_duration_ms.denominator);
    let rounded = numerator
        .checked_add(denominator / 2)
        .ok_or_else(|| invalid("frame-grid duration arithmetic overflowed"))?
        / denominator;
    u64::try_from(rounded).map_err(|_| invalid("frame-grid duration arithmetic overflowed"))
}

pub(super) fn sha256(path: &Path) -> Result<String, CutError> {
    let mut file = open_read_nofollow(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(source_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn read_nofollow(path: &Path) -> Result<Vec<u8>, CutError> {
    let mut file = open_read_nofollow(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(source_error)?;
    Ok(bytes)
}

fn open_read_nofollow(path: &Path) -> Result<File, CutError> {
    if !is_plain_regular_file(path).map_err(source_error)? {
        return Err(invalid(
            "projection file is missing, linked, or not a local regular file",
        ));
    }
    open_checked_nofollow(path)
}

/// Every projection reader comes through this opened-handle validation. The
/// preflight above narrows the literal leaf, while this check proves that the
/// handle obtained after a concurrent swap is still a plain file (and not a
/// Windows reparse point) before any bytes are hashed, copied, or decoded.
fn open_checked_nofollow(path: &Path) -> Result<File, CutError> {
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
    let file = options.open(path).map_err(source_error)?;
    let metadata = file.metadata().map_err(source_error)?;
    if is_open_plain_regular(&metadata) {
        Ok(file)
    } else {
        Err(invalid(
            "projection file changed while opening without following links",
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

fn source_error(error: impl std::fmt::Display) -> CutError {
    invalid(format!("inspect sealed pause projection artifact: {error}"))
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "cannot execute pause-session legacy projection",
        detail.into(),
    )
}

#[cfg(test)]
#[path = "artifacts_tests.rs"]
mod tests;
