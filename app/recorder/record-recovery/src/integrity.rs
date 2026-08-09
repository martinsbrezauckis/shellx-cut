//! Local checkpoint evidence classification before recovery may mutate anything.

use std::path::Path;

use crate::manifest::{
    checkpoint_name, checkpoint_path, io, is_local_checkpoint_file, is_plain_dir,
    is_plain_regular_file, sha256,
};
use crate::media::{matches_expected, verify_checkpoint_media};
use crate::{Checkpoint, ManifestError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefixIssue {
    /// The journal names a checkpoint that is absent, a link/reparse point, or
    /// another non-local file. It is not proof of corruption and cannot be moved.
    MissingEvidence,
    /// A local regular checkpoint was successfully verified and contradicted its
    /// immutable journal facts, so it may be quarantined.
    Corrupt(u64),
    /// A prior attempt atomically moved the proven-corrupt checkpoint but crashed
    /// before its receipt append. Preserve that durable fact on the retry.
    AlreadyQuarantined(u64),
}

pub(crate) fn verified_prefix(
    root: &Path,
    segments: &[Checkpoint],
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<(Vec<Checkpoint>, Option<PrefixIssue>), ManifestError> {
    let mut usable = Vec::new();
    for segment in segments {
        if segment.file != checkpoint_name(segment.sequence) {
            return Err(ManifestError::Invalid("checkpoint path mismatch".into()));
        }
        let path = checkpoint_path(root, segment.sequence);
        if !is_local_checkpoint_file(root, segment.sequence)? {
            if is_missing(&path)? && is_quarantined_checkpoint(root, segment.sequence)? {
                return Ok((
                    usable,
                    Some(PrefixIssue::AlreadyQuarantined(segment.sequence)),
                ));
            }
            return Ok((usable, Some(PrefixIssue::MissingEvidence)));
        }
        // Tool failures are deliberately propagated. No hash, rename, output, or
        // receipt mutation is safe until playback verification is available.
        let media = verify_checkpoint_media(ffmpeg, ffprobe, &path)?;
        let media_matches = media.is_some_and(|actual| {
            segment
                .media
                .as_ref()
                .map(|expected| matches_expected(expected, &actual))
                .unwrap_or(true)
        });
        if !media_matches || sha256(&path)? != segment.sha256 {
            return Ok((usable, Some(PrefixIssue::Corrupt(segment.sequence))));
        }
        usable.push(segment.clone());
    }
    Ok((usable, None))
}

pub(crate) fn quarantine(root: &Path, sequence: u64) -> Result<std::path::PathBuf, ManifestError> {
    if !is_local_checkpoint_file(root, sequence)? {
        return Err(ManifestError::Invalid(
            "unsafe checkpoint for quarantine".into(),
        ));
    }
    let bad = checkpoint_path(root, sequence);
    let dir = root.join("quarantine");
    std::fs::create_dir_all(&dir).map_err(|source| crate::manifest::io(&dir, source))?;
    if !is_plain_dir(&dir)? {
        return Err(ManifestError::Invalid("unsafe quarantine directory".into()));
    }
    let target = quarantine_path(root, sequence);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(ManifestError::Invalid(
                "quarantine target already exists".into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(crate::manifest::io(&target, source)),
    }
    std::fs::rename(&bad, &target).map_err(|source| crate::manifest::io(&target, source))?;
    Ok(target)
}

pub(crate) fn quarantine_path(root: &Path, sequence: u64) -> std::path::PathBuf {
    root.join("quarantine")
        .join(format!("segment-{sequence:06}.mp4.corrupt"))
}

fn is_quarantined_checkpoint(root: &Path, sequence: u64) -> Result<bool, ManifestError> {
    let dir = root.join("quarantine");
    Ok(is_plain_dir(&dir)? && is_plain_regular_file(&quarantine_path(root, sequence))?)
}

fn is_missing(path: &Path) -> Result<bool, ManifestError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Ok(_) => Ok(false),
        Err(source) => Err(io(path, source)),
    }
}
