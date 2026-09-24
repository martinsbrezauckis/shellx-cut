//! Capture-clock stitch workspace with local no-follow temporary artifacts.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::integrity::verified_prefix;
use crate::manifest::{
    checkpoint_path, is_local_checkpoint_file, is_plain_dir, is_plain_regular_file,
};
use crate::media::{bounded_status, verify_media};
use crate::{Checkpoint, ManifestError, MediaFacts};

mod workspace;
use workspace::StitchWorkspace;

const CONCAT_TOLERANCE_MS: u64 = 120;
const ELAPSED_TOLERANCE_MS: u64 = 1_120;
// A native encoder can drain a few already-captured frames after the Stop clock
// is sampled. Keep this separate from concat/output tolerance: the PC2
// GStreamer capture measured 7_033 ms while its verified 25 fps file held
// 7_160 ms (179 frames). Larger overruns still indicate inconsistent input.
const CAPTURE_DRAIN_TOLERANCE_MS: u64 = 200;

/// A final source path coupled to the exact facts already verified before its
/// no-replace publication. Consumers must carry these facts forward instead of
/// launching a second long-running final-source probe.
#[derive(Debug, Clone)]
pub struct StitchedMedia {
    pub path: PathBuf,
    pub media: MediaFacts,
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
    let final_path = root.join(output_name);
    if is_plain_regular_file(&final_path)? {
        let media = verify_media(ffmpeg, ffprobe, &final_path)?;
        return existing_stitch_matches(root, &usable, ffmpeg, ffprobe, &media)?
            .then_some(StitchedMedia {
                path: final_path,
                media,
            })
            .ok_or_else(|| {
                ManifestError::Invalid(
                    "existing stitched output does not match the verified checkpoint contract"
                        .into(),
                )
            });
    }
    let mut workspace = StitchWorkspace::new(root)?;
    let mut transcodes = Vec::new();
    let mut rows = String::new();
    let mut expected_duration_ms = 0u64;
    let mut expected_frames = 0u64;
    for (index, segment) in usable.iter().enumerate() {
        let leading_gap_ms = leading_gap(&usable, index)?;
        if !is_local_checkpoint_file(root, segment.sequence)? {
            return Err(ManifestError::Invalid("checkpoint became unsafe".into()));
        }
        let source = checkpoint_path(root, segment.sequence);
        let source_media = checkpoint_media(root, segment, ffmpeg, ffprobe)?;
        if source_media.has_audio {
            return Err(ManifestError::Invalid(
                "checkpoint media contains embedded audio".into(),
            ));
        }
        let capture_span_ms = capture_span_ms(segment, &source_media)?;
        let transcode = workspace.reserve(&format!("segment-{:06}.mp4", segment.sequence))?;
        let start_padding = format!("{:.3}", leading_gap_ms as f64 / 1000.0);
        let stop_padding = format!(
            "{:.3}",
            capture_span_ms.saturating_sub(source_media.duration_ms) as f64 / 1000.0
        );
        let status = bounded_status(
            Command::new(ffmpeg)
                .args(["-v", "error", "-y", "-i"])
                .arg(&source)
                .args([
                    "-vf",
                    &format!(
                        "tpad=start_mode=clone:start_duration={start_padding}:stop_mode=clone:stop_duration={stop_padding}"
                    ),
                    "-an",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                ])
                .arg(&transcode),
            "materialize checkpoint restart gap",
        )?;
        let media = verify_media(ffmpeg, ffprobe, &transcode).ok();
        if !status.success() || media.is_none() {
            return Err(ManifestError::Invalid(
                "ffmpeg could not materialize checkpoint gap".into(),
            ));
        }
        let media = media.expect("checked above");
        let expected_segment_ms = capture_span_ms.saturating_add(leading_gap_ms);
        if media.duration_ms.abs_diff(expected_segment_ms) > CONCAT_TOLERANCE_MS || media.has_audio
        {
            return Err(ManifestError::Invalid(
                "transcoded checkpoint does not preserve duration/audio contract".into(),
            ));
        }
        expected_duration_ms = expected_duration_ms.saturating_add(media.duration_ms);
        expected_frames = expected_frames.saturating_add(media.decoded_video_frames);
        rows.push_str("file '");
        rows.push_str(&transcode.to_string_lossy().replace('\'', "'\\\\''"));
        rows.push_str("'\n");
        transcodes.push(transcode);
    }
    let list = workspace.write("concat.txt", rows.as_bytes())?;
    let part = workspace.reserve("output.part.mp4")?;
    let status = bounded_status(
        Command::new(ffmpeg)
            .args(["-v", "error", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list)
            .args(["-c", "copy"])
            .arg(&part),
        "concat finalized checkpoints",
    )?;
    let final_media = verify_media(ffmpeg, ffprobe, &part).ok();
    if !status.success() || final_media.is_none() {
        return Err(ManifestError::Invalid(
            "ffmpeg could not produce a playable stitched checkpoint".into(),
        ));
    }
    let final_media = final_media.expect("checked above");
    let elapsed_ms = usable
        .last()
        .map(|segment| segment.facts.end_ms)
        .unwrap_or(0);
    if final_media.has_audio
        || final_media.decoded_video_frames.abs_diff(expected_frames) > 1
        || final_media.duration_ms.abs_diff(expected_duration_ms) > CONCAT_TOLERANCE_MS
        || final_media.duration_ms.abs_diff(elapsed_ms) > ELAPSED_TOLERANCE_MS
    {
        return Err(ManifestError::Invalid(
            "stitched media does not match verified frame/duration/audio contract".into(),
        ));
    }
    crate::atomic::publish_new_synced(&part, &final_path).map_err(|source| ManifestError::Io {
        path: final_path.clone(),
        source,
    })?;
    Ok(StitchedMedia {
        path: final_path,
        media: final_media,
    })
}

fn leading_gap(segments: &[Checkpoint], index: usize) -> Result<u64, ManifestError> {
    let segment = &segments[index];
    if segment.facts.event_offset_ms != segment.facts.start_ms {
        return Err(ManifestError::Invalid(
            "checkpoint event offset differs from capture start".into(),
        ));
    }
    if index == 0 {
        return Ok(segment.facts.event_offset_ms);
    }
    let previous = &segments[index - 1];
    segment
        .facts
        .event_offset_ms
        .checked_sub(previous.facts.end_ms)
        .ok_or_else(|| ManifestError::Invalid("checkpoint facts overlap".into()))
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
    if !is_local_checkpoint_file(root, segment.sequence)? {
        return Err(ManifestError::Invalid("checkpoint became unsafe".into()));
    }
    verify_media(ffmpeg, ffprobe, &checkpoint_path(root, segment.sequence))
}

/// Return the capture-clock span this segment must occupy. A closed native encoder
/// can deliver fewer frames than elapsed wall time; stitching must preserve that
/// loss as cloned trailing video, not erase it. Media that materially exceeds its
/// independently measured span remains inconsistent and is refused.
fn capture_span_ms(segment: &Checkpoint, media: &MediaFacts) -> Result<u64, ManifestError> {
    let observed = segment
        .facts
        .end_ms
        .checked_sub(segment.facts.start_ms)
        .ok_or_else(|| ManifestError::Invalid("checkpoint capture span is negative".into()))?;
    if media.duration_ms > observed.saturating_add(CAPTURE_DRAIN_TOLERANCE_MS) {
        return Err(ManifestError::Invalid(
            "checkpoint media exceeds its observed capture span".into(),
        ));
    }
    Ok(observed.max(media.duration_ms))
}

fn existing_stitch_matches(
    root: &Path,
    segments: &[Checkpoint],
    ffmpeg: &str,
    ffprobe: &str,
    media: &MediaFacts,
) -> Result<bool, ManifestError> {
    let mut expected_duration_ms = 0u64;
    let mut expected_frames = 0u64;
    for (index, segment) in segments.iter().enumerate() {
        let facts = checkpoint_media(root, segment, ffmpeg, ffprobe)?;
        if facts.has_audio {
            return Ok(false);
        }
        expected_duration_ms = expected_duration_ms
            .saturating_add(capture_span_ms(segment, &facts)? + leading_gap(segments, index)?);
        expected_frames = expected_frames.saturating_add(facts.decoded_video_frames);
    }
    let elapsed_ms = segments
        .last()
        .map(|segment| segment.facts.end_ms)
        .unwrap_or(0);
    Ok(!media.has_audio
        && media.decoded_video_frames >= expected_frames
        && media.duration_ms.abs_diff(expected_duration_ms) <= CONCAT_TOLERANCE_MS
        && media.duration_ms.abs_diff(elapsed_ms) <= ELAPSED_TOLERANCE_MS)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.contains("..")
}
