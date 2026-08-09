//! Capture-clock stitch workspace with local no-follow temporary artifacts.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::integrity::verified_prefix;
use crate::manifest::{
    checkpoint_path, is_local_checkpoint_file, is_plain_dir, is_plain_regular_file,
};
use crate::media::{bounded_status, verify_media};
use crate::{Checkpoint, ManifestError, MediaFacts};

const CONCAT_TOLERANCE_MS: u64 = 120;
const ELAPSED_TOLERANCE_MS: u64 = 1_120;
static WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

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
            .then_some(final_path)
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
    Ok(final_path)
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
    if media.duration_ms > observed.saturating_add(CONCAT_TOLERANCE_MS) {
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

struct StitchWorkspace {
    root: PathBuf,
    files: Vec<PathBuf>,
}

impl StitchWorkspace {
    fn new(capture_root: &Path) -> Result<Self, ManifestError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        for attempt in 0..32 {
            let counter = WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = capture_root.join(format!(
                ".stitch-work-{}-{nonce}-{counter}-{attempt}",
                std::process::id()
            ));
            match create_private_dir(&root) {
                Ok(()) => {
                    if is_plain_dir(&root)? {
                        return Ok(Self {
                            root,
                            files: Vec::new(),
                        });
                    }
                    return Err(ManifestError::Invalid("unsafe stitch workspace".into()));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(ManifestError::Io { path: root, source }),
            }
        }
        Err(ManifestError::Invalid(
            "could not reserve stitch workspace".into(),
        ))
    }

    fn reserve(&mut self, name: &str) -> Result<PathBuf, ManifestError> {
        self.create(name, None)
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf, ManifestError> {
        self.create(name, Some(bytes))
    }

    fn create(&mut self, name: &str, bytes: Option<&[u8]>) -> Result<PathBuf, ManifestError> {
        let path = self.root.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| ManifestError::Io {
                path: path.clone(),
                source,
            })?;
        if let Some(bytes) = bytes {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|source| ManifestError::Io {
                    path: path.clone(),
                    source,
                })?;
        }
        drop(file);
        if !is_plain_regular_file(&path)? {
            return Err(ManifestError::Invalid("unsafe stitch temporary".into()));
        }
        self.files.push(path.clone());
        Ok(path)
    }
}

impl Drop for StitchWorkspace {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = fs::remove_file(file);
        }
        let _ = fs::remove_dir(&self.root);
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.contains("..")
}
