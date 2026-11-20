//! Capture-clock stitch workspace with local no-follow temporary artifacts.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::integrity::verified_prefix;
use crate::manifest::{is_plain_dir, is_plain_regular_file, read_manifest};
use crate::media::{bounded_status, probe_own_output, verify_media};
use crate::{Checkpoint, ManifestError, MediaFacts};

mod cfr;
mod concat;
mod encoder;
mod frames;
mod notes;
mod plan;
#[cfg(all(test, unix))]
mod provenance_tests;
mod workspace;
pub use cfr::prepare_cfr_segment;
use concat::MAX_CONCAT_ENTRIES;
pub use notes::stitch_progress;
use plan::{plan_segments, SegmentPlan};
use workspace::StitchWorkspace;

const CONCAT_TOLERANCE_MS: u64 = 120;
const ELAPSED_TOLERANCE_MS: u64 = 1_120;

/// A final source path coupled to the exact facts already verified before its
/// no-replace publication. Consumers must carry these facts forward instead of
/// launching a second long-running final-source probe.
#[derive(Debug, Clone)]
pub struct StitchedMedia {
    pub path: PathBuf,
    pub media: MediaFacts,
    /// What is wrong with the saved take, in words for the person. Empty when
    /// it matches the recording exactly. A stitch that finds a problem still
    /// saves what was captured and says so here; it never discards the take.
    pub notes: Vec<String>,
}

/// Stitch verified segments after materializing every observed capture-clock gap as
/// cloned video frames. That includes encoder-restart gaps and time in a closed
/// native segment for which no video frame was delivered. All mutable ffmpeg paths
/// live in an atomically-created, private child directory, never at predictable
/// capture-root names.
pub fn stitch_complete(
    root: &Path,
    segments: &[Checkpoint],
    ffmpeg: &str,
    ffprobe: &str,
    output_name: &str,
) -> Result<PathBuf, ManifestError> {
    Ok(stitch_complete_with_media(root, segments, ffmpeg, ffprobe, output_name)?.path)
}

/// As [`stitch_complete`], while retaining the already verified final media
/// facts for a caller that needs factual capture metadata.
pub fn stitch_complete_with_media(
    root: &Path,
    segments: &[Checkpoint],
    ffmpeg: &str,
    ffprobe: &str,
    output_name: &str,
) -> Result<StitchedMedia, ManifestError> {
    stitch_complete_with_media_bounded(
        root,
        segments,
        ffmpeg,
        ffprobe,
        output_name,
        MAX_CONCAT_ENTRIES,
    )
}

pub(crate) fn stitch_complete_with_media_bounded(
    root: &Path,
    segments: &[Checkpoint],
    ffmpeg: &str,
    ffprobe: &str,
    output_name: &str,
    max_concat_entries: u64,
) -> Result<StitchedMedia, ManifestError> {
    if segments.is_empty() || !safe_name(output_name) || !is_plain_dir(root)? {
        return Err(ManifestError::Invalid(
            "no segments, unsafe output, or unsafe root".into(),
        ));
    }
    let (usable, issue) = verified_prefix(root, segments, ffmpeg, ffprobe)?;
    if issue.is_some() || usable.len() != segments.len() {
        return Err(ManifestError::Invalid(
            "refusing to stitch unverified checkpoint".into(),
        ));
    }
    // Both normal Stop and interrupted recovery consume the same durable
    // admission. Container nominal rates describe native packets, not intent.
    let output_cadence = read_manifest(root)?.start.output_cadence;
    let source_media = usable
        .iter()
        .map(|segment| checkpoint_media(root, segment, ffmpeg, ffprobe))
        .collect::<Result<Vec<_>, _>>()?;
    let grid = grid(output_cadence, &source_media);
    let elapsed_ms = usable.last().map_or(0, |segment| segment.facts.end_ms);
    let final_path = root.join(output_name);
    if is_plain_regular_file(&final_path)? {
        let media = verify_media(ffmpeg, ffprobe, &final_path)?;
        let plans = plan_segments(&usable, &source_media)?;
        let strict = existing_matches(&plans, &source_media, &media, &grid, elapsed_ms);
        // Notes this engine saved for this exact file win: a take saved with a
        // problem keeps saying so on every later stitch of the same capture.
        // Without that proof the take must pass the checkpoint contract.
        let notes = notes::recorded_for(root, &final_path).or(strict.then(Vec::new));
        return notes
            .map(|notes| StitchedMedia {
                path: final_path,
                media,
                notes,
            })
            .ok_or_else(|| {
                ManifestError::Invalid(
                    "existing stitched output does not match the verified checkpoint contract"
                        .into(),
                )
            });
    }
    let mut workspace = StitchWorkspace::new(root)?;
    let mut notes = Vec::new();
    // Segments whose part could not be made drop out; the gap they leave holds
    // the picture before them, exactly like a restart gap.
    let mut kept: Vec<usize> = (0..usable.len()).collect();
    let mut made: Vec<Option<(SegmentPlan, cfr::Made)>> = vec![None; usable.len()];
    let mut unit_seconds = 1;
    let parts = loop {
        let segments: Vec<Checkpoint> = kept.iter().map(|&i| usable[i].clone()).collect();
        let media: Vec<MediaFacts> = kept.iter().map(|&i| source_media[i].clone()).collect();
        let plans = plan_segments(&segments, &media)?;
        let selected_seconds = frames::compact_unit_seconds(
            &plans,
            grid.num,
            grid.den,
            max_concat_entries,
        )
        .ok_or(ManifestError::StitchLimit(
            "too many parts to join safely; keep the original capture folder and contact support for recovery",
        ))?;
        if selected_seconds != unit_seconds {
            made.fill(None);
            unit_seconds = selected_seconds;
        }
        let mut failed = None;
        for (done, (&index, plan)) in kept.iter().zip(&plans).enumerate() {
            notes::write_progress(root, done, kept.len());
            if made[index].as_ref().is_some_and(|(done, _)| done == plan) {
                continue;
            }
            let segment = &usable[index];
            if let Some(part) = cfr::cached_with_unit(root, segment, plan, &grid, unit_seconds) {
                made[index] = Some((*plan, part));
                continue;
            }
            let name = format!("segment-{:06}-{}", segment.sequence, plan.target_ms());
            let output = workspace.reserve(&format!("{name}.mp4"))?;
            let unit_output = workspace.reserve(&format!("{name}.unit.mp4"))?;
            let result = cfr::transcode(
                ffmpeg,
                ffprobe,
                root,
                segment,
                &source_media[index],
                plan,
                &grid,
                &output,
                &unit_output,
                unit_seconds,
            );
            match result {
                Ok(part) => made[index] = Some((*plan, part)),
                Err(error) => {
                    failed = Some((index, error));
                    break;
                }
            }
        }
        match failed {
            None => {
                break kept
                    .iter()
                    .map(|&index| made[index].clone().expect("every kept part was made"))
                    .collect::<Vec<_>>()
            }
            Some((index, error)) => {
                notes.push(notes::lost_segment(&usable[index], &error));
                kept.retain(|&kept_index| kept_index != index);
                if kept.is_empty() {
                    return Err(error);
                }
            }
        }
    };
    let (rows, expected_duration_ms, expected_frames) = concat::rows(&parts, max_concat_entries)?;
    let list = workspace.write("concat.txt", rows.as_bytes())?;
    let part = workspace.reserve("output.part.mp4")?;
    let status = bounded_status(
        Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-protocol_whitelist",
                cut_media::ffmpeg::LOCAL_INPUT_PROTOCOLS,
                "-format_whitelist",
                "concat,mov",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
            ])
            .arg(&list)
            .args(["-c", "copy"])
            .arg(&part),
        "concat finalized checkpoints",
    )?;
    let final_media = status
        .success()
        .then(|| probe_own_output(ffprobe, &part).ok())
        .flatten()
        .ok_or_else(|| {
            ManifestError::Invalid("ffmpeg could not produce a playable stitched checkpoint".into())
        })?;
    notes.extend(notes::contract_mismatches(
        &final_media,
        grid.admitted,
        (expected_duration_ms, expected_frames),
        elapsed_ms,
        (grid.tolerance_ms, ELAPSED_TOLERANCE_MS),
    ));
    let notes_hash = if notes.is_empty() {
        None
    } else {
        Some(notes::record(root, &part, output_name, &notes)?)
    };
    crate::atomic::publish_new_synced(&part, &final_path).map_err(|source| ManifestError::Io {
        path: final_path.clone(),
        source,
    })?;
    if let Some(hash) = notes_hash {
        notes::remember_take(&final_path, &hash);
    }
    Ok(StitchedMedia {
        path: final_path,
        media: final_media,
        notes,
    })
}

pub(crate) fn grid(cadence: Option<record_core::FrameRate>, sources: &[MediaFacts]) -> cfr::Grid {
    let (num, den) = cadence.map_or((stitch_frame_rate(sources), 1), |rate| (rate.num, rate.den));
    cfr::Grid {
        rate: format!("{num}/{den}"),
        num,
        den,
        admitted: cadence,
        tolerance_ms: cadence_duration_tolerance_ms(cadence),
    }
}

fn stitch_frame_rate(sources: &[MediaFacts]) -> u64 {
    sources
        .iter()
        .flat_map(|media| [media.r_frame_rate, media.avg_frame_rate])
        .flatten()
        .map(|rate| u128::from(rate.num).div_ceil(u128::from(rate.den)))
        .max()
        .unwrap_or(30)
        .clamp(30, 240) as u64
}

pub(crate) fn matches_output_cadence(
    media: &MediaFacts,
    cadence: Option<record_core::FrameRate>,
) -> bool {
    cadence
        .is_none_or(|rate| media.avg_frame_rate == Some(rate) && media.r_frame_rate == Some(rate))
}

fn cadence_duration_tolerance_ms(cadence: Option<record_core::FrameRate>) -> u64 {
    cadence.map_or(CONCAT_TOLERANCE_MS, |rate| {
        // A CFR duration occupies whole frames, including admitted rates below
        // 30. Quantization may extend the capture span by less than one frame.
        CONCAT_TOLERANCE_MS
            .max((1_000 * u128::from(rate.den)).div_ceil(u128::from(rate.num)) as u64)
    })
}

fn checkpoint_media(
    root: &Path,
    segment: &Checkpoint,
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<MediaFacts, ManifestError> {
    if let Some(media) = &segment.media {
        return Ok(media.clone());
    }
    if !crate::manifest::is_local_checkpoint_file(root, segment.sequence)? {
        return Err(ManifestError::Invalid("checkpoint became unsafe".into()));
    }
    verify_media(
        ffmpeg,
        ffprobe,
        &crate::manifest::checkpoint_path(root, segment.sequence),
    )
}

fn existing_matches(
    plans: &[SegmentPlan],
    sources: &[MediaFacts],
    media: &MediaFacts,
    grid: &cfr::Grid,
    elapsed_ms: u64,
) -> bool {
    let expected_duration_ms: u64 = plans.iter().map(SegmentPlan::target_ms).sum();
    let expected_frames: u64 = sources.iter().map(|facts| facts.decoded_video_frames).sum();
    !media.has_audio
        && matches_output_cadence(media, grid.admitted)
        && (grid.admitted.is_some() || media.decoded_video_frames >= expected_frames)
        && media.duration_ms.abs_diff(expected_duration_ms) <= grid.tolerance_ms
        && media.duration_ms.abs_diff(elapsed_ms) <= ELAPSED_TOLERANCE_MS
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.contains("..")
}
