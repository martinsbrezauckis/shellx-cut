//! The record of a part prepared during the recording, and when the final
//! stitch may reuse it: only a part this engine made here (part files and
//! record have origin records outside the capture folder, see
//! `cut_media::cache_trust`) whose repeat count and frames are exactly the
//! checked plan's. Anything else is made again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::super::frames::part_frames_with_unit;
use super::super::plan::SegmentPlan;
use super::{Grid, Made};
use crate::manifest::{is_plain_dir, is_plain_regular_file, sha256};
use crate::{Checkpoint, ManifestError, MediaFacts};

const CACHE_DIR: &str = "stitch-cache";
pub(super) const CACHE_SCHEMA: &str = "shellx-cut/stitch-cfr-part/2";
const MAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct CacheRecord {
    pub(super) schema: String,
    pub(super) sequence: u64,
    pub(super) source_sha256: String,
    pub(super) lead_ms: u64,
    pub(super) span_ms: u64,
    pub(super) hold_ms: u64,
    pub(super) rate: String,
    pub(super) output_sha256: String,
    pub(super) media: MediaFacts,
    /// The hold unit and how often it repeats (none when `units` is 0).
    pub(super) unit_sha256: Option<String>,
    pub(super) units: u64,
    /// Earlier trusted records omitted this because every unit was one second.
    #[serde(default)]
    pub(super) unit_frames: Option<u64>,
}

/// A prepared part that matches this exact plan, source and grid.
pub(crate) fn cached(
    root: &Path,
    segment: &Checkpoint,
    plan: &SegmentPlan,
    grid: &Grid,
) -> Option<Made> {
    cached_with_unit(root, segment, plan, grid, 1)
}

pub(crate) fn cached_with_unit(
    root: &Path,
    segment: &Checkpoint,
    plan: &SegmentPlan,
    grid: &Grid,
    unit_seconds: u64,
) -> Option<Made> {
    let dir = root.join(CACHE_DIR);
    if !is_plain_dir(&dir).ok()? {
        return None;
    }
    let output = dir.join(part_name(segment.sequence));
    let record_path = dir.join(record_name(segment.sequence));
    if !is_plain_regular_file(&output).ok()? || !is_plain_regular_file(&record_path).ok()? {
        return None;
    }
    // Only a record this engine wrote here, unchanged since (its origin record
    // lives outside the capture folder), says anything about the part.
    if std::fs::metadata(&record_path).ok()?.len() > MAX_RECORD_BYTES
        || !cut_media::cache_trust::trusted(&record_path)
    {
        return None;
    }
    let record: CacheRecord = serde_json::from_slice(&std::fs::read(&record_path).ok()?).ok()?;
    // The repeat count and frame total must be exactly this plan's.
    let frames = part_frames_with_unit(plan, grid.num, grid.den, unit_seconds);
    let matches = record.units == frames.units
        && record
            .unit_frames
            .map_or(unit_seconds == 1, |count| count == frames.unit)
        && record.media.decoded_video_frames == frames.total
        && record.unit_sha256.is_some() == (frames.units > 0)
        && record.schema == CACHE_SCHEMA
        && record.sequence == segment.sequence
        && record.source_sha256 == segment.sha256
        && (record.lead_ms, record.span_ms, record.hold_ms)
            == (plan.lead_ms, plan.span_ms, plan.hold_ms)
        && record.rate == grid.rate
        && record.media.duration_ms.abs_diff(plan.target_ms()) <= grid.tolerance_ms;
    let made_here = |path: &Path, expected: &str| {
        sha256(path).ok().is_some_and(|hash| {
            hash == expected && cut_media::cache_trust::trusted_hash(path, &hash)
        })
    };
    if !matches || !made_here(&output, &record.output_sha256) {
        return None;
    }
    let unit = match record.unit_sha256.as_ref() {
        None if record.units == 0 => None,
        Some(expected) if record.units > 0 => {
            let unit = dir.join(unit_name(segment.sequence));
            (is_plain_regular_file(&unit).ok()? && made_here(&unit, expected))
                .then_some(unit)?
                .into()
        }
        _ => return None,
    };
    Some(Made {
        main: output,
        unit,
        units: record.units,
        facts: record.media,
    })
}

pub(super) fn cache_dir(root: &Path) -> Result<PathBuf, ManifestError> {
    let dir = root.join(CACHE_DIR);
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => return Err(ManifestError::Io { path: dir, source }),
    }
    if !is_plain_dir(&dir)? {
        return Err(ManifestError::Invalid(
            "stitch cache is not a local directory".into(),
        ));
    }
    Ok(dir)
}

pub(super) fn part_name(sequence: u64) -> String {
    format!("segment-{sequence:06}.cfr.mp4")
}

pub(super) fn unit_name(sequence: u64) -> String {
    format!("segment-{sequence:06}.unit.mp4")
}

pub(super) fn record_name(sequence: u64) -> String {
    format!("segment-{sequence:06}.cfr.json")
}
