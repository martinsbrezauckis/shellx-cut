//! Backend-facing checkpoint adapter. It owns no encoder state: a backend asks for
//! an `.open.mp4` path, closes that encoder, then atomically publishes the segment.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use record_core::{error_codes, RecordError, Result};
use record_recovery::{CheckpointFacts, ManifestOwner, MediaFacts, PrivateStaging};

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
    ) -> Result<()> {
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let ffprobe = std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into());
        self.publish_with_tools(sequence, staging, facts, &ffmpeg, &ffprobe)
    }

    fn publish_with_tools(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
        ffmpeg: &str,
        ffprobe: &str,
    ) -> Result<()> {
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
        self.owner
            .publish(sequence, staging, facts, media)
            .map_err(|e| error(&e.to_string()))?;
        Ok(())
    }

    pub(crate) fn stitch(&self, ffmpeg: &str, ffprobe: &str, source_name: &str) -> Result<PathBuf> {
        record_recovery::stitch_complete(
            &self.root,
            &self.owner.manifest().checkpoints,
            ffmpeg,
            ffprobe,
            source_name,
        )
        .map_err(|e| error(&e.to_string()))
    }
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

    use record_recovery::{CaptureStart, ManifestOwner};
    use tempfile::tempdir;

    use super::{CheckpointConfig, Checkpoints};

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

        checkpoints
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
            )
            .unwrap();

        let checkpoint = &checkpoints.owner.manifest().checkpoints[0];
        assert!(!checkpoint.media.as_ref().unwrap().has_audio);
        assert_eq!(checkpoint.media.as_ref().unwrap().decoded_video_frames, 3);
        assert_eq!(
            fs::read(root.path().join("checkpoints/segment-000000.mp4")).unwrap(),
            b"video-only"
        );
    }
}
