//! One checkpoint turned into a constant-frame-rate part of the stitched take.
//!
//! The native file holds only the frames the system delivered (a still screen
//! gives a few per second). The part repeats the last frame on the admitted
//! frame grid, as every screen recorder's output does, and covers exactly the
//! segment's planned capture-clock time.
//!
//! A part can be made while the recording still runs, as soon as its plan is
//! known (`prepare_cfr_segment`). The final stitch then reuses it instead of
//! encoding the whole take at Stop, but only when this engine made it here:
//! its part files and record have origin records outside the capture folder
//! (`cut_media::cache_trust`). A part, record or unit count that came with a
//! capture folder from elsewhere is never reused; the part is made again.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::frames::{frames_ms, part_frames_with_unit, part_graph, PartFrames};
use super::plan::{plan_segment, SegmentPlan};
use crate::manifest::{
    checkpoint_path, is_local_checkpoint_file, is_plain_regular_file, read_manifest, sha256,
};
use crate::media::{bounded_status_within, probe_own_output};
use cache::{cache_dir, part_name, record_name, unit_name, CacheRecord, CACHE_SCHEMA};

mod cache;
pub(crate) use cache::cached;
pub(crate) use cache::cached_with_unit;

use crate::{Checkpoint, ManifestError, MediaFacts};

/// The frame grid a part is made on, from the capture's admitted cadence.
#[derive(Debug, Clone)]
pub(crate) struct Grid {
    pub(crate) rate: String,
    pub(crate) num: u64,
    pub(crate) den: u64,
    pub(crate) admitted: Option<record_core::FrameRate>,
    pub(crate) tolerance_ms: u64,
}

/// One made part: its main file, then `units` copies of its hold unit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Made {
    pub(crate) main: PathBuf,
    pub(crate) unit: Option<PathBuf>,
    pub(crate) units: u64,
    /// Facts of the whole part (main plus every unit copy).
    pub(crate) facts: MediaFacts,
}

/// Make the CFR part of `segments[index]` ahead of Stop. `next_start_ms` is the
/// capture-clock time of the next segment's first frame. Best effort: the final
/// stitch makes any part that is missing or does not match.
pub fn prepare_cfr_segment(
    root: &Path,
    segments: &[Checkpoint],
    index: usize,
    next_start_ms: Option<u64>,
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<(), ManifestError> {
    let segment = segments
        .get(index)
        .ok_or_else(|| ManifestError::Invalid("no checkpoint to prepare".into()))?;
    let media = segment
        .media
        .as_ref()
        .ok_or_else(|| ManifestError::Invalid("checkpoint has no verified media facts".into()))?;
    let grid = super::grid(
        read_manifest(root)?.start.output_cadence,
        std::slice::from_ref(media),
    );
    let plan = plan_segment(index, segment, media, next_start_ms)?;
    if cached(root, segment, &plan, &grid).is_some() {
        return Ok(());
    }
    let dir = cache_dir(root)?;
    let stage = |kind: &str| {
        dir.join(format!(
            ".segment-{:06}.{}.{kind}.part.mp4",
            segment.sequence,
            std::process::id()
        ))
    };
    let (main_part, unit_part) = (stage("main"), stage("unit"));
    let _ = std::fs::remove_file(&main_part);
    let _ = std::fs::remove_file(&unit_part);
    let made = transcode(
        ffmpeg, ffprobe, root, segment, media, &plan, &grid, &main_part, &unit_part, 1,
    );
    let made = match made {
        Ok(made) => made,
        Err(error) => {
            let _ = std::fs::remove_file(&main_part);
            let _ = std::fs::remove_file(&unit_part);
            return Err(error);
        }
    };
    let install = |part: &Path, output: &Path| -> Result<String, ManifestError> {
        let io = |source| ManifestError::Io {
            path: output.to_path_buf(),
            source,
        };
        if is_plain_regular_file(output)? {
            crate::atomic::replace_file_synced(part, output).map_err(io)?;
        } else {
            crate::atomic::publish_new_synced(part, output).map_err(io)?;
        }
        sha256(output)
    };
    let main_path = dir.join(part_name(segment.sequence));
    let output_sha256 = install(&main_part, &main_path)?;
    cut_media::cache_trust::remember_hash(&main_path, &output_sha256);
    let unit_sha256 = match made.unit.as_ref() {
        Some(_) => {
            let unit_path = dir.join(unit_name(segment.sequence));
            let hash = install(&unit_part, &unit_path)?;
            cut_media::cache_trust::remember_hash(&unit_path, &hash);
            Some(hash)
        }
        None => {
            let _ = std::fs::remove_file(&unit_part);
            None
        }
    };
    let record = CacheRecord {
        schema: CACHE_SCHEMA.into(),
        sequence: segment.sequence,
        source_sha256: segment.sha256.clone(),
        lead_ms: plan.lead_ms,
        span_ms: plan.span_ms,
        hold_ms: plan.hold_ms,
        rate: grid.rate.clone(),
        output_sha256,
        media: made.facts,
        unit_sha256,
        units: made.units,
        unit_frames: Some(part_frames_with_unit(&plan, grid.num, grid.den, 1).unit),
    };
    let record_path = dir.join(record_name(segment.sequence));
    crate::atomic::replace_synced(&record_path, &serde_json::to_vec(&record)?).map_err(
        |source| ManifestError::Io {
            path: record_path.clone(),
            source,
        },
    )?;
    cut_media::cache_trust::remember(&record_path);
    Ok(())
}

/// Encode one part with the first encoder that works (see
/// `encoder::encoder_choices`); a failed or mismatching attempt tries the next one.
/// `unit_output` receives the hold unit when the part has one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn transcode(
    ffmpeg: &str,
    ffprobe: &str,
    root: &Path,
    segment: &Checkpoint,
    media: &MediaFacts,
    plan: &SegmentPlan,
    grid: &Grid,
    output: &Path,
    unit_output: &Path,
    unit_seconds: u64,
) -> Result<Made, ManifestError> {
    if !is_local_checkpoint_file(root, segment.sequence)? {
        return Err(ManifestError::Invalid("checkpoint became unsafe".into()));
    }
    let source = checkpoint_path(root, segment.sequence);
    let frames = part_frames_with_unit(plan, grid.num, grid.den, unit_seconds);
    let fps = if grid.admitted.is_some() {
        format!("fps={}:eof_action=pass", grid.rate)
    } else {
        format!("fps={}", grid.rate)
    };
    let graph = part_graph(plan, &frames, &fps, grid.num, grid.den);
    // Generous: about five times the encoded time plus the fixed bound.
    let encoded_ms = frames_ms(frames.main + frames.unit, grid.num, grid.den);
    let timeout = Duration::from_millis(encoded_ms.saturating_mul(5));
    let mut last = None;
    for encoder in super::encoder::encoder_choices(media) {
        let mut command = Command::new(ffmpeg);
        command
            .args(["-v", "error", "-y", "-protocol_whitelist"])
            .arg(cut_media::ffmpeg::LOCAL_INPUT_PROTOCOLS)
            .args(["-format_whitelist", "mov", "-f", "mov", "-i"])
            .arg(&source)
            .args(["-filter_complex", &graph, "-map", "[pa]", "-an"])
            .args(&encoder)
            .arg(output);
        if frames.units > 0 {
            command
                .args(["-map", "[pu]", "-an"])
                .args(&encoder)
                .arg(unit_output);
        }
        let made = bounded_status_within(&mut command, "make constant-rate screen part", timeout)
            .and_then(|status| {
                if status.success() {
                    probe_made(ffprobe, output, unit_output, &frames, grid)
                } else {
                    Err(ManifestError::Invalid(
                        "ffmpeg could not make the constant-rate screen part".into(),
                    ))
                }
            });
        match made {
            Ok(made) => return Ok(made),
            Err(error) => {
                last = Some(ManifestError::Invalid(format!(
                    "the screen part from segment {}: {error}",
                    segment.sequence
                )))
            }
        }
    }
    Err(last.unwrap_or_else(|| ManifestError::Invalid("no video encoder was available".into())))
}

/// Check the written files hold exactly the planned frames on the grid.
fn probe_made(
    ffprobe: &str,
    output: &Path,
    unit_output: &Path,
    frames: &PartFrames,
    grid: &Grid,
) -> Result<Made, ManifestError> {
    let main = probe_own_output(ffprobe, output)?;
    // The frame count is exact with ffmpeg's frame trims; a tool that
    // reports packets differently is held to the duration within one frame.
    let exact = |facts: &MediaFacts, count: u64| {
        !facts.has_audio
            && (facts.decoded_video_frames == count
                || facts
                    .duration_ms
                    .abs_diff(frames_ms(count, grid.num, grid.den))
                    <= grid.tolerance_ms)
            && super::matches_output_cadence(facts, grid.admitted)
    };
    if !exact(&main, frames.main) {
        return Err(ManifestError::Invalid(format!(
            "it holds {} frames at {:?}, planned {} at {}",
            main.decoded_video_frames, main.avg_frame_rate, frames.main, grid.rate
        )));
    }
    let unit = if frames.units > 0 {
        let unit = probe_own_output(ffprobe, unit_output)?;
        if !exact(&unit, frames.unit) {
            return Err(ManifestError::Invalid(format!(
                "its hold unit holds {} frames, planned {}",
                unit.decoded_video_frames, frames.unit
            )));
        }
        Some(unit_output.to_path_buf())
    } else {
        None
    };
    let mut facts = main;
    facts.decoded_video_frames = frames.total;
    facts.duration_ms = frames_ms(frames.total, grid.num, grid.den);
    Ok(Made {
        main: output.to_path_buf(),
        unit,
        units: frames.units,
        facts,
    })
}
