//! Backend-facing checkpoint adapter. It owns no encoder state: a backend asks for
//! an `.open.mp4` path, closes that encoder, then atomically publishes the segment.
#![cfg_attr(
    not(any(
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos"),
        all(target_os = "linux", feature = "capture-linux")
    )),
    allow(dead_code)
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use record_core::{error_codes, RecordError, Result};
use record_recovery::{CheckpointFacts, ManifestOwner, MediaFacts, PrivateStaging};

use crate::windows_wgc_timing::WgcTimingRecorder;
use crate::CheckpointConfig;

pub(crate) struct Checkpoints {
    root: PathBuf,
    owner: ManifestOwner,
    interval_ms: u64,
    next: u64,
}

impl Checkpoints {
    pub(crate) fn open(config: Option<&CheckpointConfig>) -> Result<Option<Self>> {
        let Some(config) = config else {
            return Ok(None);
        };
        if config.interval_ms == 0 {
            return Err(error("checkpoint interval must be positive"));
        }
        let root = PathBuf::from(&config.manifest_dir);
        let owner = ManifestOwner::open(&root).map_err(|e| error(&e.to_string()))?;
        if owner.manifest().receipt.is_some() {
            return Err(error("checkpoint manifest is already complete"));
        }
        if owner.manifest().has_open_segment() {
            return Err(error(
                "checkpoint manifest has an unresolved open segment; recovery must run first",
            ));
        }
        let next = owner.manifest().next_sequence();
        Ok(Some(Self {
            root,
            owner,
            interval_ms: config.interval_ms,
            next,
        }))
    }

    pub(crate) fn interval_ms(&self) -> u64 {
        self.interval_ms
    }

    #[cfg_attr(all(windows, feature = "capture-windows"), allow(dead_code))]
    pub(crate) fn begin(&mut self, start_ms: u64) -> Result<(u64, PathBuf)> {
        self.begin_with(start_ms, ManifestOwner::begin_segment)
    }

    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn begin_windows_wgc(&mut self, start_ms: u64) -> Result<(u64, PathBuf)> {
        self.begin_with(start_ms, ManifestOwner::begin_windows_wgc_segment)
    }

    fn begin_with(
        &mut self,
        start_ms: u64,
        reserve: fn(
            &mut ManifestOwner,
            u64,
            u64,
        ) -> std::result::Result<PathBuf, record_recovery::ManifestError>,
    ) -> Result<(u64, PathBuf)> {
        let sequence = self.next;
        let path =
            reserve(&mut self.owner, sequence, start_ms).map_err(|e| error(&e.to_string()))?;
        self.next = self.next.saturating_add(1);
        Ok((sequence, path))
    }

    #[cfg(not(all(windows, feature = "capture-windows")))]
    pub(crate) fn publish(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
    ) -> Result<record_recovery::Checkpoint> {
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
        self.publish_with_tools(sequence, staging, facts, &ffmpeg, &ffprobe, None)
    }

    /// Ordinary WGC owns an independent observation of each accepted native
    /// frame. Use it only to correct a proven encoder-generated terminal tail.
    #[cfg_attr(not(all(windows, feature = "capture-windows")), allow(dead_code))]
    pub(crate) fn publish_windows_wgc(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
        timing: Option<&WgcTimingRecorder>,
    ) -> Result<record_recovery::Checkpoint> {
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
        self.publish_with_tools(sequence, staging, facts, &ffmpeg, &ffprobe, timing)
    }

    fn publish_with_tools(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
        ffmpeg: &str,
        ffprobe: &str,
        timing: Option<&WgcTimingRecorder>,
    ) -> Result<record_recovery::Checkpoint> {
        // WGC may reopen a checkpoint immediately before Stop, then receive no
        // frame at all. Its closed MP4 cannot be probed. Keep the elapsed span
        // by holding the last verified frame, and mark that fact in the journal.
        // A segment with any accepted native frame still follows ordinary
        // verification; failures there must not be hidden by this fallback.
        let held_tail = timing.is_some_and(|observation| {
            observation.accepted_frames() == 0 && observation.control_stop_succeeded()
        }) && sequence > 0;
        if held_tail {
            materialize_empty_wgc_tail(
                &self.root,
                self.owner.manifest(),
                sequence,
                staging,
                &facts,
                ffmpeg,
                ffprobe,
            )?;
        }
        // A closed encoder file is still not a checkpoint until both the container
        // facts and a full decode succeed. The manifest never names an open or merely
        // non-empty MP4.
        if !record_recovery::is_plain_regular_file(staging).map_err(|e| error(&e.to_string()))? {
            return Err(error(
                "native checkpoint result is not a local regular file",
            ));
        }
        let media = record_recovery::verify_media(ffmpeg, ffprobe, staging)
            .map_err(|e| error(&e.to_string()))?;
        let media = normalize_video_only_checkpoint(staging, media, ffmpeg, ffprobe)?;
        let media = if let Some(timing) = timing {
            trim_extrapolated_wgc_tail(staging, &facts, media, timing, ffmpeg, ffprobe)?
        } else {
            media
        };
        let published = if held_tail {
            self.owner
                .publish_held_tail(sequence, staging, facts, media)
        } else {
            self.owner.publish(sequence, staging, facts, media)
        };
        published.map_err(|e| error(&e.to_string()))
    }

    /// Return the final path together with facts from the one verification pass
    /// required before its publication. Do not make a later metadata consumer
    /// re-probe the same long capture.
    pub(crate) fn stitch(
        &self,
        ffmpeg: &str,
        ffprobe: &str,
        source_name: &str,
    ) -> Result<(PathBuf, MediaFacts)> {
        let stitched = record_recovery::stitch_complete_with_media(
            &self.root,
            &self.owner.manifest().checkpoints,
            ffmpeg,
            ffprobe,
            source_name,
        )
        .map_err(|e| error(&e.to_string()))?;
        Ok((stitched.path, stitched.media))
    }
}

fn materialize_empty_wgc_tail(
    root: &Path,
    manifest: &record_recovery::CaptureManifest,
    sequence: u64,
    staging: &Path,
    facts: &CheckpointFacts,
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<()> {
    let prior = manifest
        .checkpoints
        .last()
        .filter(|checkpoint| checkpoint.sequence.checked_add(1) == Some(sequence))
        .ok_or_else(|| error("a frame-free WGC tail has no verified prior checkpoint"))?;
    let prior_media = prior
        .media
        .as_ref()
        .filter(|media| media.decoded_video_frames > 0)
        .ok_or_else(|| error("the prior checkpoint has no verified video frame"))?;
    let prior_path = root.join(format!("checkpoints/segment-{:06}.mp4", prior.sequence));
    if prior.file != format!("checkpoints/segment-{:06}.mp4", prior.sequence)
        || !record_recovery::is_plain_regular_file(&prior_path)
            .map_err(|cause| error(&cause.to_string()))?
    {
        return Err(error("the prior checkpoint is not a local regular file"));
    }
    let span_ms = facts.end_ms.saturating_sub(facts.start_ms);
    if span_ms == 0 {
        return Err(error("a frame-free WGC tail has no elapsed capture span"));
    }
    let held = PrivateStaging::create(root, "held-wgc-tail", "segment.mp4")
        .map_err(|cause| error(&format!("reserve held WGC tail: {cause}")))?;
    let last_frame = prior_media.decoded_video_frames - 1;
    let filter = format!(
        "select=eq(n\\,{last_frame}),setpts=PTS-STARTPTS,tpad=stop_mode=clone:stop_duration={:.3}",
        span_ms as f64 / 1000.0
    );
    let duration = format!("{:.3}", span_ms as f64 / 1000.0);
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(120), || false);
    let output = cut_media::ffmpeg::run_owned_command(
        Command::new(ffmpeg)
            .args(["-v", "error", "-n", "-i"])
            .arg(&prior_path)
            .args([
                "-map", "0:v:0", "-vf", &filter, "-an", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                "-t", &duration,
            ])
            .arg(held.path()),
        &control,
        "hold last verified frame for WGC tail without native frames",
    )
    .map_err(|cause| error(&cause.to_string()))?;
    if !output.status.success() {
        return Err(error("ffmpeg could not hold the last verified WGC frame"));
    }
    let media = record_recovery::verify_media(ffmpeg, ffprobe, held.path())
        .map_err(|cause| error(&cause.to_string()))?;
    if media.has_audio
        || media.width != prior_media.width
        || media.height != prior_media.height
        || media.duration_ms.abs_diff(span_ms) > 100
    {
        return Err(error(
            "held WGC tail does not preserve resolution and elapsed duration",
        ));
    }
    let install = match std::fs::symlink_metadata(staging) {
        Ok(_)
            if record_recovery::is_plain_regular_file(staging)
                .map_err(|cause| error(&cause.to_string()))? =>
        {
            record_recovery::replace_file_synced(held.path(), staging)
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            record_recovery::publish_new_synced(held.path(), staging)
        }
        Ok(_) => return Err(error("WGC tail stage is not a local regular file")),
        Err(cause) => return Err(error(&format!("inspect WGC tail stage: {cause}"))),
    };
    install.map_err(|cause| error(&format!("install held WGC tail: {cause}")))
}

fn trim_extrapolated_wgc_tail(
    staging: &Path,
    facts: &CheckpointFacts,
    media: MediaFacts,
    timing: &WgcTimingRecorder,
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<MediaFacts> {
    let span_ms = facts.end_ms.saturating_sub(facts.start_ms);
    if !timing.explains_extrapolated_tail(
        facts.start_ms,
        facts.end_ms,
        media.duration_ms,
        media.decoded_video_frames,
    ) {
        return Ok(media);
    }
    let parent = staging
        .parent()
        .ok_or_else(|| error("WGC checkpoint staging path has no parent"))?;
    let clipped = PrivateStaging::create_windows_wgc(parent)
        .map_err(|cause| error(&format!("reserve WGC tail correction: {cause}")))?;
    let duration = format!("{:.3}", span_ms as f64 / 1000.0);
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(60), || false);
    let output = cut_media::ffmpeg::run_owned_command(
        Command::new(ffmpeg)
            .args(["-v", "error", "-n", "-i"])
            .arg(staging)
            .args(["-map", "0:v:0", "-an", "-t", &duration, "-c:v", "copy"])
            .arg(clipped.path()),
        &control,
        "clip WGC encoder tail to observed Stop",
    )
    .map_err(|cause| error(&cause.to_string()))?;
    if !output.status.success() {
        return Err(error("ffmpeg could not clip the WGC encoder tail"));
    }
    let corrected = record_recovery::verify_media(ffmpeg, ffprobe, clipped.path())
        .map_err(|cause| error(&cause.to_string()))?;
    if corrected.has_audio
        || corrected.decoded_video_frames < timing.accepted_frames()
        || corrected.decoded_video_frames >= media.decoded_video_frames
        || corrected.duration_ms > span_ms.saturating_add(100)
        || corrected.duration_ms.saturating_add(100) < span_ms
        || corrected.width != media.width
        || corrected.height != media.height
        || corrected.codec_name != media.codec_name
    {
        return Err(error(
            "WGC tail correction did not preserve native frames and capture duration",
        ));
    }
    record_recovery::replace_file_synced(clipped.path(), staging)
        .map_err(|cause| error(&format!("install corrected WGC checkpoint: {cause}")))?;
    Ok(corrected)
}

/// `windows-capture` 2.x always describes an audio stream to Media Foundation,
/// even when its audio source is disabled. The resulting checkpoint therefore
/// can carry a non-authoritative AAC stream. Checkpoints are deliberately video-only because
/// microphone and system audio have independent capture clocks. Strip any native
/// encoder audio into a private stage, verify the video facts, then atomically
/// replace only the still-owned open checkpoint before immutable publication.
fn normalize_video_only_checkpoint(
    staging: &Path,
    media: MediaFacts,
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<MediaFacts> {
    if !media.has_audio {
        return Ok(media);
    }
    let parent = staging
        .parent()
        .ok_or_else(|| error("checkpoint staging path has no parent"))?;
    // Use the compact WGC reservation shape here as well: project paths can
    // legitimately sit close to the upstream WinRT path limit.
    let normalized = PrivateStaging::create_windows_wgc(parent)
        .map_err(|cause| error(&format!("reserve video-only checkpoint: {cause}")))?;
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(60), || false);
    let output = cut_media::ffmpeg::run_owned_command(
        Command::new(ffmpeg)
            .args(["-v", "error", "-n", "-i"])
            .arg(staging)
            .args(["-map", "0:v:0", "-an", "-c:v", "copy"])
            .arg(normalized.path()),
        &control,
        "strip native checkpoint audio",
    )
    .map_err(|cause| error(&cause.to_string()))?;
    if !output.status.success() {
        return Err(error("ffmpeg could not strip native checkpoint audio"));
    }
    let video_only = record_recovery::verify_media(ffmpeg, ffprobe, normalized.path())
        .map_err(|cause| error(&cause.to_string()))?;
    if video_only.has_audio
        || video_only.decoded_video_frames != media.decoded_video_frames
        || video_only.duration_ms.abs_diff(media.duration_ms) > 20
    {
        return Err(error(
            "video-only checkpoint does not preserve decoded frames and duration",
        ));
    }
    record_recovery::replace_file_synced(normalized.path(), staging)
        .map_err(|cause| error(&format!("install video-only checkpoint: {cause}")))?;
    Ok(video_only)
}

fn error(cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, "checkpoint publication failed", cause)
        .with_action("keep the capture directory writable and retry recording")
}

#[cfg(all(test, unix))]
mod tests;
