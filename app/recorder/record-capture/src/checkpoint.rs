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
mod tests {
    use std::fs;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use record_recovery::{CaptureStart, ManifestOwner};
    use tempfile::tempdir;

    use super::{trim_extrapolated_wgc_tail, CheckpointConfig, Checkpoints};
    use crate::windows_wgc_timing::WgcTimingRecorder;

    #[test]
    fn zero_frame_final_wgc_checkpoint_holds_verified_frame_and_stitches_full_span() {
        let root = tempdir().unwrap();
        ManifestOwner::begin(root.path(), CaptureStart::new("zero-final-wgc", 400)).unwrap();
        let config = CheckpointConfig {
            manifest_dir: root.path().to_string_lossy().into_owned(),
            interval_ms: 400,
        };
        let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
        let (sequence, first) = checkpoints.begin_windows_wgc(0).unwrap();
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=32x32:r=30"
            ])
            .args(["-t", "0.4", "-an", "-c:v", "libx264"])
            .arg(&first)
            .status()
            .unwrap()
            .success());
        let facts = record_recovery::CheckpointFacts {
            start_ms: 0,
            end_ms: 400,
            event_offset_ms: 0,
            audio_offset_ms: None,
        };
        checkpoints
            .publish_with_tools(sequence, &first, facts, "ffmpeg", "ffprobe", None)
            .unwrap();

        let (sequence, empty) = checkpoints.begin_windows_wgc(400).unwrap();
        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 30, &empty);
        timing.control_stopped(origin + Duration::from_millis(800), true);
        let held = checkpoints
            .publish_with_tools(
                sequence,
                &empty,
                record_recovery::CheckpointFacts {
                    start_ms: 400,
                    end_ms: 800,
                    event_offset_ms: 400,
                    audio_offset_ms: None,
                },
                "ffmpeg",
                "ffprobe",
                Some(&timing),
            )
            .unwrap();
        assert_eq!(held.held_last_frame_ms, Some(400));
        assert!(held.media.as_ref().unwrap().duration_ms.abs_diff(400) <= 100);
        assert_ne!(held.file, checkpoints.owner.manifest().checkpoints[0].file);
        assert!(root.path().join(&held.file).is_file());
        let (source, media) = checkpoints
            .stitch("ffmpeg", "ffprobe", "source.mp4")
            .unwrap();
        assert!(source.is_file());
        assert!(media.duration_ms.abs_diff(800) <= 100);
        let manifest = record_recovery::read_manifest(root.path()).unwrap();
        assert!(!manifest.has_open_segment());
        assert_eq!(manifest.checkpoints[1].held_last_frame_ms, Some(400));
        assert_eq!(
            record_recovery::recovery_status(root.path()).held_last_frame_ms,
            Some(400)
        );
    }

    #[test]
    fn accepted_wgc_frame_never_uses_held_tail_fallback() {
        let root = tempdir().unwrap();
        ManifestOwner::begin(root.path(), CaptureStart::new("accepted-final-wgc", 400)).unwrap();
        let config = CheckpointConfig {
            manifest_dir: root.path().to_string_lossy().into_owned(),
            interval_ms: 400,
        };
        let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
        let (_, first) = checkpoints.begin_windows_wgc(0).unwrap();
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=32x32:r=30"
            ])
            .args(["-t", "0.4", "-an", "-c:v", "libx264"])
            .arg(&first)
            .status()
            .unwrap()
            .success());
        checkpoints
            .publish_with_tools(
                0,
                &first,
                record_recovery::CheckpointFacts {
                    start_ms: 0,
                    end_ms: 400,
                    event_offset_ms: 0,
                    audio_offset_ms: None,
                },
                "ffmpeg",
                "ffprobe",
                None,
            )
            .unwrap();
        let (sequence, invalid) = checkpoints.begin_windows_wgc(400).unwrap();
        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 30, &invalid);
        timing.accepted_frame(1, origin + Duration::from_millis(450));
        let result = checkpoints.publish_with_tools(
            sequence,
            &invalid,
            record_recovery::CheckpointFacts {
                start_ms: 400,
                end_ms: 800,
                event_offset_ms: 400,
                audio_offset_ms: None,
            },
            "ffmpeg",
            "ffprobe",
            Some(&timing),
        );
        assert!(result.is_err());
        assert_eq!(checkpoints.owner.manifest().checkpoints.len(), 1);

        let failed_stop = WgcTimingRecorder::new(origin, 30, &invalid);
        failed_stop.control_stopped(origin + Duration::from_millis(800), false);
        let result = checkpoints.publish_with_tools(
            sequence,
            &invalid,
            record_recovery::CheckpointFacts {
                start_ms: 400,
                end_ms: 800,
                event_offset_ms: 400,
                audio_offset_ms: None,
            },
            "ffmpeg",
            "ffprobe",
            Some(&failed_stop),
        );
        assert!(result.is_err());
        assert!(!invalid.exists());
        assert_eq!(checkpoints.owner.manifest().checkpoints.len(), 1);
    }

    #[test]
    fn proven_wgc_tail_clip_preserves_frames_and_stop_span() {
        let root = tempdir().unwrap();
        let staging = root.path().join("source.mp4");
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=32x32:r=60"
            ])
            .args(["-t", "7.75", "-c:v", "libx264"])
            .arg(&staging)
            .status()
            .unwrap()
            .success());
        let original = record_recovery::verify_media("ffmpeg", "ffprobe", &staging).unwrap();
        assert_eq!(original.duration_ms, 7_750);
        assert_eq!(original.decoded_video_frames, 465);
        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._timing.d/s.mp4"));
        let first = 5_978_568_688_251i64;
        timing.accepted_frame(first, origin + Duration::from_millis(477));
        timing.accepted_frame(first + 11_000_000, origin + Duration::from_millis(1_577));
        timing.accepted_frame(first + 44_334_192, origin + Duration::from_millis(4_910));
        timing.control_stopped(origin + Duration::from_millis(7_337), true);
        let corrected = trim_extrapolated_wgc_tail(
            &staging,
            &record_recovery::CheckpointFacts {
                start_ms: 0,
                end_ms: 7_338,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            original,
            &timing,
            "ffmpeg",
            "ffprobe",
        )
        .unwrap();
        assert!(corrected.duration_ms.abs_diff(7_338) <= 100);
        assert!(corrected.decoded_video_frames >= timing.accepted_frames());
        assert!(corrected.decoded_video_frames < 465);
        assert_eq!(
            record_recovery::verify_media("ffmpeg", "ffprobe", &staging)
                .unwrap()
                .duration_ms,
            corrected.duration_ms
        );
    }

    #[test]
    fn earlier_max_gap_clip_verifies_real_mp4_at_observed_stop() {
        let root = tempdir().unwrap();
        let staging = root.path().join("source.mp4");
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=32x32:r=60",
            ])
            .args(["-t", "7.933", "-c:v", "libx264"])
            .arg(&staging)
            .status()
            .unwrap()
            .success());
        let original = record_recovery::verify_media("ffmpeg", "ffprobe", &staging).unwrap();
        assert_eq!(original.duration_ms, 7_933);
        assert_eq!(original.decoded_video_frames, 476);

        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._timing.d/s.mp4"));
        let first = 6_146_209_642_756i64;
        timing.accepted_frame(first, origin + Duration::from_millis(480));
        timing.accepted_frame(first + 34_000_076, origin + Duration::from_millis(3_880));
        timing.accepted_frame(first + 45_500_670, origin + Duration::from_millis(5_030));
        timing.control_stopped(origin + Duration::from_millis(7_497), true);
        let corrected = trim_extrapolated_wgc_tail(
            &staging,
            &record_recovery::CheckpointFacts {
                start_ms: 0,
                end_ms: 7_499,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            original,
            &timing,
            "ffmpeg",
            "ffprobe",
        )
        .unwrap();
        assert!(corrected.duration_ms.abs_diff(7_499) <= 100);
        assert!(corrected.decoded_video_frames >= timing.accepted_frames());
        assert!(corrected.decoded_video_frames < 476);
        assert_eq!(
            record_recovery::verify_media("ffmpeg", "ffprobe", &staging)
                .unwrap()
                .duration_ms,
            corrected.duration_ms
        );
    }

    #[test]
    fn checkpoint_rejects_a_planted_native_output_link_before_media_verification() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        drop(ManifestOwner::begin(root.path(), CaptureStart::new("capture", 100)).unwrap());
        let config = CheckpointConfig {
            manifest_dir: root.path().display().to_string(),
            interval_ms: 100,
        };
        let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
        let (sequence, staging) = checkpoints.begin(0).unwrap();
        let outside = tempdir().unwrap();
        let target = outside.path().join("outside.mp4");
        fs::write(&target, b"outside remains untouched").unwrap();
        symlink(&target, &staging).unwrap();

        assert!(checkpoints
            .publish(
                sequence,
                &staging,
                record_recovery::CheckpointFacts {
                    start_ms: 0,
                    end_ms: 100,
                    event_offset_ms: 0,
                    audio_offset_ms: None,
                },
            )
            .is_err());
        assert_eq!(fs::read(target).unwrap(), b"outside remains untouched");
    }

    #[test]
    fn checkpoint_strips_native_encoder_audio_before_publication() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempdir().unwrap();
        drop(ManifestOwner::begin(root.path(), CaptureStart::new("capture", 100)).unwrap());
        let config = CheckpointConfig {
            manifest_dir: root.path().display().to_string(),
            interval_ms: 100,
        };
        let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
        let (sequence, staging) = checkpoints.begin_windows_wgc(0).unwrap();
        fs::write(&staging, b"native-with-audio").unwrap();

        let ffmpeg = root.path().join("fake-ffmpeg");
        let ffprobe = root.path().join("fake-ffprobe");
        fs::write(
            &ffmpeg,
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\n[ \"$last\" = - ] && exit 0\nprintf video-only > \"$last\"\n",
        )
        .unwrap();
        fs::write(
            &ffprobe,
            "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nif grep -q video-only \"$last\"; then audio=; else audio=',{\"codec_type\":\"audio\",\"nb_read_frames\":\"1\"}'; fi\nprintf '{\"format\":{\"duration\":\"0.100\"},\"streams\":[{\"codec_type\":\"video\",\"nb_read_frames\":\"3\"}%s]}' \"$audio\"\n",
        )
        .unwrap();
        for tool in [&ffmpeg, &ffprobe] {
            fs::set_permissions(tool, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let checkpoint = checkpoints
            .publish_with_tools(
                sequence,
                &staging,
                record_recovery::CheckpointFacts {
                    start_ms: 1,
                    end_ms: 101,
                    event_offset_ms: 1,
                    audio_offset_ms: None,
                },
                ffmpeg.to_str().unwrap(),
                ffprobe.to_str().unwrap(),
                None,
            )
            .unwrap();

        assert_eq!(checkpoint.sequence, 0);
        assert_eq!(checkpoint.file, "checkpoints/segment-000000.mp4");
        let checkpoint = &checkpoints.owner.manifest().checkpoints[0];
        assert!(!checkpoint.media.as_ref().unwrap().has_audio);
        assert_eq!(checkpoint.media.as_ref().unwrap().decoded_video_frames, 3);
        assert_eq!(
            fs::read(root.path().join("checkpoints/segment-000000.mp4")).unwrap(),
            b"video-only"
        );
    }
}
