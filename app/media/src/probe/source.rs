//! Probe a file someone wants to edit, and say plainly when it cannot be used.
//!
//! `probe()` reports ffprobe's own words, which suit a log but not a person:
//! "moov atom not found" does not tell anyone that the file is a renamed text
//! file or a download that never finished. Import and relink go through
//! [`probe_source`], which keeps every other error as it is (a missing ffprobe,
//! a probe that could not start) but turns "ffprobe looked at the file and
//! could not read it as media" into one stable `unreadable_media` error with
//! the file's name, the reason, and what to do.

use super::{normalize_ffprobe, MediaProbe};
use cut_core::{error_codes, CutError};
use std::path::Path;

/// What a person can do with a file Cut cannot read.
pub const UNREADABLE_ACTION: &str = "Check that the file plays in a media player. If it does not, \
copy or export it again from where it came from, then import the new copy.";

/// The file's name for a sentence a person reads (the full path stays in `cause`).
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// ffprobe's useful lines, without the demuxer address prefixes, bounded.
fn ffprobe_reason(stderr: &str) -> String {
    let lines: Vec<String> = stderr
        .lines()
        .map(|line| {
            // "[mov,mp4,... @ 0x55d...] moov atom not found" -> "moov atom not found"
            match line.find("] ") {
                Some(i) if line.starts_with('[') => line[i + 2..].trim().to_string(),
                _ => line.trim().to_string(),
            }
        })
        .filter(|line| !line.is_empty())
        .collect();
    let mut text = lines.join("; ");
    if text.len() > 400 {
        let mut cut = 400;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('…');
    }
    if text.is_empty() {
        "ffprobe gave no reason".into()
    } else {
        text
    }
}

/// The error for a file that exists but is not usable media. `why` is the
/// technical reason (ffprobe's words or the missing fact).
pub fn unreadable_media_error(path: &Path, why: &str) -> CutError {
    CutError::new(
        error_codes::UNREADABLE_MEDIA,
        format!(
            "{} cannot be used: it is not a video, audio or image file that Cut can read, or it is damaged",
            display_name(path)
        ),
        format!("{}: {why}", path.display()),
    )
    .with_suggested_action(UNREADABLE_ACTION)
}

/// The error for an empty file, found before any tool runs.
pub fn empty_media_error(path: &Path) -> CutError {
    CutError::new(
        error_codes::UNREADABLE_MEDIA,
        format!(
            "{} cannot be used: the file is empty (0 bytes)",
            display_name(path)
        ),
        format!("{}: file size is 0 bytes", path.display()),
    )
    .with_suggested_action(
        "Check that the file finished copying or downloading, then import it again.",
    )
}

/// Probe a source file for import or relink. Errors that say nothing about the
/// file (ffprobe missing, ffprobe could not start or timed out) pass through
/// unchanged; a file ffprobe read and rejected, or one with no usable picture
/// or sound, becomes [`unreadable_media_error`].
pub fn probe_source(path: &Path) -> Result<MediaProbe, CutError> {
    if std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == 0) {
        return Err(empty_media_error(path));
    }
    let raw = match crate::ffmpeg::ffprobe_json(path) {
        Ok(raw) => raw,
        Err(e) if e.code == error_codes::FFMPEG && e.message.starts_with("ffprobe failed on ") => {
            return Err(unreadable_media_error(path, &ffprobe_reason(&e.cause)));
        }
        Err(e) => return Err(e),
    };
    let probe = classify_normalized(path, normalize_ffprobe(raw, path))?;
    if probe.kind == super::kinds::IMAGE {
        crate::picture_decode::check(path)?;
    }
    Ok(probe)
}

/// Pure half of [`probe_source`]: judge a normalized probe result.
pub(super) fn classify_normalized(
    path: &Path,
    probed: Result<MediaProbe, CutError>,
) -> Result<MediaProbe, CutError> {
    let probe = probed.map_err(|e| {
        if e.code == error_codes::FFMPEG {
            unreadable_media_error(path, &format!("{} ({})", e.message, e.cause))
        } else {
            e
        }
    })?;
    // No picture and no sound: a container with only data or subtitle streams,
    // or none at all. It would otherwise become an "audio" asset with no audio.
    if probe.width.is_none() && !probe.has_audio {
        return Err(unreadable_media_error(
            path,
            &format!("no video or audio stream found ({})", probe.format),
        ));
    }
    Ok(probe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ffprobe_noise_becomes_a_short_reason() {
        let reason = ffprobe_reason(
            "[mov,mp4,m4a,3gp,3g2,mj2 @ 0x5f25ed113fc0] moov atom not found\n\
             text.mp4: Invalid data found when processing input\n",
        );
        assert_eq!(
            reason,
            "moov atom not found; text.mp4: Invalid data found when processing input"
        );
        assert_eq!(ffprobe_reason(""), "ffprobe gave no reason");
    }

    #[test]
    fn unreadable_error_names_the_file_the_reason_and_the_next_step() {
        let e = unreadable_media_error(Path::new("/tmp/clips/textfile.mp4"), "moov atom not found");
        assert_eq!(e.code, error_codes::UNREADABLE_MEDIA);
        assert!(e.message.starts_with("textfile.mp4 cannot be used"));
        assert!(e.message.contains("not a video, audio or image file"));
        assert!(e.cause.contains("/tmp/clips/textfile.mp4"));
        assert!(e.cause.contains("moov atom not found"));
        assert_eq!(e.suggested_action.as_deref(), Some(UNREADABLE_ACTION));
    }

    #[test]
    fn a_probe_with_neither_picture_nor_sound_is_unreadable() {
        let raw = json!({
            "format": {"format_name": "mov,mp4", "duration": "2.0"},
            "streams": [{"codec_type": "data", "codec_name": "bin_data"}]
        });
        let path = Path::new("data-only.mp4");
        let err = classify_normalized(path, normalize_ffprobe(raw, path)).unwrap_err();
        assert_eq!(err.code, error_codes::UNREADABLE_MEDIA);
        assert!(err.cause.contains("no video or audio stream"));
    }

    #[test]
    fn a_normalize_failure_is_unreadable_and_a_good_probe_passes() {
        // text renamed .png: ffprobe exits 0 with a png stream of width 0.
        let bad = json!({
            "format": {"format_name": "image2", "duration": "0.04"},
            "streams": [{"codec_type": "video", "codec_name": "png", "width": 0, "height": 0}]
        });
        let path = Path::new("text.png");
        let err = classify_normalized(path, normalize_ffprobe(bad, path)).unwrap_err();
        assert_eq!(err.code, error_codes::UNREADABLE_MEDIA);
        let good = json!({
            "format": {"format_name": "mov,mp4", "duration": "1.0"},
            "streams": [{"codec_type": "video", "codec_name": "h264", "width": 64,
                         "height": 64, "avg_frame_rate": "30/1"}]
        });
        let path = Path::new("good.mp4");
        assert!(classify_normalized(path, normalize_ffprobe(good, path)).is_ok());
    }

    #[test]
    fn an_empty_file_is_refused_before_any_tool_runs() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.mp4");
        std::fs::write(&empty, b"").unwrap();
        let err = probe_source(&empty).unwrap_err();
        assert_eq!(err.code, error_codes::UNREADABLE_MEDIA);
        assert!(err.message.contains("empty (0 bytes)"), "{}", err.message);
    }
}
