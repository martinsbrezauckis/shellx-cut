//! Private FFmpeg staging and no-replace publication of Linux `source.mp4`.

use std::path::Path;
use std::process::Command;

/// CFR-normalize a sparse raw capture into a random private stage, verify it,
/// then create the user-facing `source.mp4` only if that final name is absent.
pub(crate) fn normalize_and_publish(
    raw: &Path,
    final_path: &Path,
    duration_ms: u64,
    fps: u32,
    ffmpeg: &str,
    ffprobe: &str,
) -> Result<(), String> {
    let parent = final_path
        .parent()
        .ok_or_else(|| "normalized source path has no parent".to_string())?;
    let stage = record_recovery::PrivateStaging::create(parent, "source-normalize", "source.mp4")
        .map_err(|error| format!("reserve normalized source staging: {error}"))?;
    let staged = stage.path().to_path_buf();
    let staged_s = staged.display().to_string();
    let raw_s = raw.display().to_string();
    let duration_s = format!("{:.3}", duration_ms as f64 / 1000.0);
    let filter = format!("fps={fps},tpad=stop_mode=clone:stop_duration=3600");
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
            "20",
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
    record_recovery::publish_new_synced(&staged, final_path)
        .map_err(|error| format!("publish normalized source: {error}"))?;
    let _ = stage.cleanup();
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::normalize_and_publish;

    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    fn fake_tools(root: &Path, final_path: &Path) -> (String, String, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let ffmpeg = root.join("fake-ffmpeg");
        let ffprobe = root.join("fake-ffprobe");
        let args = root.join("ffmpeg-args.txt");
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
            "#!/bin/sh\nprintf '{\"format\":{\"duration\":\"0.100\"},\"streams\":[{\"codec_type\":\"video\",\"nb_read_frames\":\"1\"}]}'\n",
        )
        .unwrap();
        for tool in [&ffmpeg, &ffprobe] {
            fs::set_permissions(tool, fs::Permissions::from_mode(0o755)).unwrap();
        }
        (
            ffmpeg.display().to_string(),
            ffprobe.display().to_string(),
            args,
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
    fn normalization_uses_a_private_ffmpeg_target_then_publishes_source_once() {
        let root = tempdir().unwrap();
        let raw = root.path().join("raw.mp4");
        let final_path = root.path().join("source.mp4");
        fs::write(&raw, b"raw").unwrap();
        let (ffmpeg, ffprobe, args) = fake_tools(root.path(), &final_path);

        normalize_and_publish(&raw, &final_path, 100, 30, &ffmpeg, &ffprobe).unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"stage");
        assert_private_ffmpeg_target(&args, &final_path);
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
            let (ffmpeg, ffprobe, args) = fake_tools(root.path(), &final_path);

            assert!(normalize_and_publish(&raw, &final_path, 100, 30, &ffmpeg, &ffprobe).is_err());
            assert_private_ffmpeg_target(&args, &final_path);
            assert!(fs::read(&preserved)
                .unwrap()
                .ends_with(b"final remains untouched"));
            assert!(!Path::new(env!("CARGO_MANIFEST_DIR")).join("-").exists());
        }
    }
}
