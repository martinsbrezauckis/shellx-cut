//! Private FFmpeg staging and no-replace publication of Linux `source.mp4`.

use std::path::Path;
use std::process::Command;

use record_core::{CaptureQualityProfile, CaptureQualityRequest, CaptureQualityResolution};

pub(crate) struct NormalizedSource {
    pub(crate) media: record_recovery::MediaFacts,
    pub(crate) quality: Option<CaptureQualityResolution>,
}

fn normalized_filter(fps: u32, quality: Option<&CaptureQualityRequest>) -> String {
    let scale = quality
        .and_then(|quality| quality.output_size.max_height())
        // The target is a maximum output height. `min` retains a smaller source
        // rather than enlarging it; -2 keeps the paired width encoder-safe/even.
        .map(|height| format!("scale=-2:min({height}\\,ih)"));
    [
        scale,
        Some(format!("fps={fps}")),
        Some("tpad=stop_mode=clone:stop_duration=3600".into()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(",")
}

fn profile_crf(quality: Option<&CaptureQualityRequest>) -> &'static str {
    match quality.map(|quality| quality.profile) {
        Some(CaptureQualityProfile::Standard) => "23",
        // Existing callers that omit the new request retain the historical
        // CRF-20 normalizer behavior.
        Some(CaptureQualityProfile::High) | None => "20",
    }
}

fn resolve_quality(
    request: CaptureQualityRequest,
    media: &record_recovery::MediaFacts,
) -> Result<CaptureQualityResolution, String> {
    let width = media
        .width
        .ok_or_else(|| "final source verification omitted output width".to_string())?;
    let height = media
        .height
        .ok_or_else(|| "final source verification omitted output height".to_string())?;
    if media.codec_name.as_deref() != Some("h264") {
        return Err(format!(
            "final source verification did not confirm libx264's H.264 output: {:?}",
            media.codec_name
        ));
    }
    if let Some(max_height) = request.output_size.max_height() {
        if height > max_height {
            return Err(format!(
                "final source verification resolved {width}x{height}, above the requested {max_height}p maximum"
            ));
        }
    }
    CaptureQualityResolution::new(request, width, height, "libx264")
        .ok_or_else(|| "final source verification returned an incomplete quality resolution".into())
}

/// CFR-normalize a sparse raw capture into a random private stage, verify it,
/// then create the user-facing `source.mp4` only if that final name is absent.
pub(crate) fn normalize_and_publish(
    raw: &Path,
    final_path: &Path,
    duration_ms: u64,
    fps: u32,
    ffmpeg: &str,
    ffprobe: &str,
    quality: Option<&CaptureQualityRequest>,
) -> Result<NormalizedSource, String> {
    let parent = final_path
        .parent()
        .ok_or_else(|| "normalized source path has no parent".to_string())?;
    let stage = record_recovery::PrivateStaging::create(parent, "source-normalize", "source.mp4")
        .map_err(|error| format!("reserve normalized source staging: {error}"))?;
    let staged = stage.path().to_path_buf();
    let staged_s = staged.display().to_string();
    let raw_s = raw.display().to_string();
    let duration_s = format!("{:.3}", duration_ms as f64 / 1000.0);
    let filter = normalized_filter(fps, quality);
    let crf = profile_crf(quality);
    let status = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-n",
            "-i",
            &raw_s,
            "-vf",
            &filter,
            "-t",
            &duration_s,
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            crf,
            "-preset",
            "medium",
            &staged_s,
        ])
        .status()
        .map_err(|error| format!("spawn CFR normalize: {error}"))?;
    if !status.success() {
        return Err(format!("CFR normalize failed: ffmpeg exit {status}"));
    }
    if !record_recovery::is_plain_regular_file(&staged)
        .map_err(|error| format!("validate normalized source staging: {error}"))?
    {
        return Err("ffmpeg did not create a local regular source file".into());
    }
    let normalized = record_recovery::verify_media(ffmpeg, ffprobe, &staged)
        .map_err(|error| format!("verify normalized source: {error}"))?;
    if normalized.has_audio || normalized.duration_ms.abs_diff(duration_ms) > 1_120 {
        return Err(format!(
            "normalized source clock mismatch: expected {duration_ms}ms video-only source, decoded {normalized:?}"
        ));
    }
    let quality = quality
        .cloned()
        .map(|request| resolve_quality(request, &normalized))
        .transpose()?;
    record_recovery::publish_new_synced(&staged, final_path)
        .map_err(|error| format!("publish normalized source: {error}"))?;
    let _ = stage.cleanup();
    Ok(NormalizedSource {
        media: normalized,
        quality,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{normalized_filter, resolve_quality};

    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    fn fake_tools(root: &Path, final_path: &Path) -> (String, String, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let ffmpeg = root.join("fake-ffmpeg");
        let ffprobe = root.join("fake-ffprobe");
        let args = root.join("ffmpeg-args.txt");
        let probe_calls = root.join("ffprobe-calls.txt");
        fs::write(
            &ffmpeg,
            format!(
                "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\n[ \"$last\" != {} ] || exit 90\n[ \"$last\" != '-' ] || exit 0\nprintf '%s\\n' \"$@\" > {}\nprintf stage > \"$last\"\n",
                shell_quote(final_path),
                shell_quote(&args),
            ),
        )
        .unwrap();
        fs::write(
            &ffprobe,
            format!(
                "#!/bin/sh\nprintf 'probe\\n' >> {}\nprintf '{{\"format\":{{\"duration\":\"0.100\"}},\"streams\":[{{\"codec_type\":\"video\",\"codec_name\":\"h264\",\"width\":1280,\"height\":720,\"nb_read_frames\":\"1\",\"avg_frame_rate\":\"30000/1001\",\"r_frame_rate\":\"30/1\"}}]}}'\n",
                shell_quote(&probe_calls),
            ),
        )
        .unwrap();
        for tool in [&ffmpeg, &ffprobe] {
            fs::set_permissions(tool, fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            ffmpeg.display().to_string(),
            ffprobe.display().to_string(),
            args,
            probe_calls,
        )
    }

    fn assert_private_ffmpeg_target(args: &Path, final_path: &Path) {
        let args = fs::read_to_string(args).unwrap();
        assert!(args.lines().any(|arg| arg == "-n"));
        assert!(
            !args
                .lines()
                .any(|arg| arg == final_path.display().to_string()),
            "ffmpeg must never receive final source.mp4 as its output target"
        );
    }

    #[test]
    fn final_source_publication_uses_one_verifier_without_a_cadence_reprobe() {
        let root = tempdir().unwrap();
        let raw = root.path().join("raw.mp4");
        let final_path = root.path().join("source.mp4");
        fs::write(&raw, b"raw").unwrap();
        let (ffmpeg, ffprobe, args, probe_calls) = fake_tools(root.path(), &final_path);

        let verified =
            super::normalize_and_publish(&raw, &final_path, 100, 30, &ffmpeg, &ffprobe, None)
                .unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"stage");
        assert_private_ffmpeg_target(&args, &final_path);
        assert_eq!(
            verified.media.avg_frame_rate,
            record_core::FrameRate::from_ffprobe("30000/1001")
        );
        assert_eq!(
            verified.media.r_frame_rate,
            record_core::FrameRate::from_ffprobe("30/1")
        );
        assert_eq!(
            fs::read_to_string(probe_calls).unwrap().lines().count(),
            1,
            "final source publication uses its mandatory verifier once, never a cadence re-probe"
        );
        assert!(
            !fs::read_dir(root.path())
                .unwrap()
                .flatten()
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .contains("source-normalize")),
            "a successfully published stage leaves no private directory behind"
        );
        assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).join("-").exists());
    }

    #[test]
    fn normalization_preserves_existing_or_linked_final_without_giving_ffmpeg_that_path() {
        use std::os::unix::fs::symlink;

        for linked in [false, true] {
            let root = tempdir().unwrap();
            let outside = tempdir().unwrap();
            let raw = root.path().join("raw.mp4");
            let final_path = root.path().join("source.mp4");
            fs::write(&raw, b"raw").unwrap();
            let preserved = if linked {
                let target = outside.path().join("outside.mp4");
                fs::write(&target, b"linked final remains untouched").unwrap();
                symlink(&target, &final_path).unwrap();
                target
            } else {
                fs::write(&final_path, b"existing final remains untouched").unwrap();
                final_path.clone()
            };
            let (ffmpeg, ffprobe, args, _probe_calls) = fake_tools(root.path(), &final_path);

            assert!(super::normalize_and_publish(
                &raw,
                &final_path,
                100,
                30,
                &ffmpeg,
                &ffprobe,
                None
            )
            .is_err());
            assert_private_ffmpeg_target(&args, &final_path);
            assert!(fs::read(&preserved)
                .unwrap()
                .ends_with(b"final remains untouched"));
            assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).join("-").exists());
        }
    }

    #[test]
    fn quality_filter_downscales_without_upscaling_and_keeps_codec_controls_private() {
        let request = record_core::CaptureQualityRequest::new(
            record_core::CaptureOutputSize::P1080,
            record_core::CaptureQualityProfile::Standard,
        );
        let filter = normalized_filter(30, Some(&request));
        assert!(filter.starts_with("scale=-2:min(1080\\,ih),fps=30"));
        assert!(!filter.contains("crf"));
        assert_eq!(super::profile_crf(Some(&request)), "23");
    }

    #[test]
    fn normalization_threads_requested_quality_into_the_verified_resolution() {
        let root = tempdir().unwrap();
        let raw = root.path().join("raw.mp4");
        let final_path = root.path().join("source.mp4");
        fs::write(&raw, b"raw").unwrap();
        let (ffmpeg, ffprobe, args, _probe_calls) = fake_tools(root.path(), &final_path);
        let request = record_core::CaptureQualityRequest::new(
            record_core::CaptureOutputSize::P720,
            record_core::CaptureQualityProfile::Standard,
        );

        let normalized = super::normalize_and_publish(
            &raw,
            &final_path,
            100,
            30,
            &ffmpeg,
            &ffprobe,
            Some(&request),
        )
        .unwrap();
        let invocation = fs::read_to_string(args).unwrap();
        assert!(invocation.contains("scale=-2:min(720\\,ih),fps=30"));
        assert!(invocation
            .lines()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| { pair == ["-crf", "23"] }));
        let quality = normalized.quality.unwrap();
        assert_eq!(quality.requested, request);
        assert_eq!((quality.width, quality.height), (1_280, 720));
    }

    #[test]
    fn quality_resolution_requires_final_dimensions_and_the_expected_verified_codec() {
        let request = record_core::CaptureQualityRequest::new(
            record_core::CaptureOutputSize::P720,
            record_core::CaptureQualityProfile::High,
        );
        let media = record_recovery::MediaFacts {
            duration_ms: 100,
            decoded_video_frames: 3,
            has_audio: false,
            width: Some(1_280),
            height: Some(720),
            codec_name: Some("h264".into()),
            avg_frame_rate: None,
            r_frame_rate: None,
        };
        let resolution = resolve_quality(request.clone(), &media).unwrap();
        assert_eq!((resolution.width, resolution.height), (1_280, 720));
        assert_eq!(resolution.encoder, "libx264");
        let mut incompatible = media;
        incompatible.codec_name = Some("hevc".into());
        assert!(resolve_quality(request, &incompatible).is_err());
    }
}
