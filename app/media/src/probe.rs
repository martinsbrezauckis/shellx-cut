//! probe.rs — media probing (public verb contract media.probe, "probe → normalized JSON").
//!
//! Role: turn ffprobe's raw JSON into the normalized MediaProbe stored on
//! Asset::probe. Dependencies: ffmpeg.rs, cut-core. Primary callers: server
//! media.import job chain, media.probe verb.

use cut_core::CutError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Asset kind classification: "video" | "audio" | "image". Image = a still
/// (PNG/JPEG/…): it has geometry but NO intrinsic duration — clips of it get
/// their duration from the edit (edit.insert duration_ms) and the renderer
/// loops the still for the clip length.
pub mod kinds {
    pub const VIDEO: &str = "video";
    pub const AUDIO: &str = "audio";
    pub const IMAGE: &str = "image";
}

/// ffprobe format_name values that mean "still image" (single-image
/// demuxers). NOTE: detection keys on the DEMUXER, not on missing duration —
/// JPEG via image2 reports a bogus one-frame duration (0.04 s @ 25 fps),
/// while PNG via png_pipe reports none at all. "gif" is
/// deliberately absent (may be animated → treated as video).
const IMAGE_FORMAT_NAMES: &[&str] = &[
    "png_pipe",
    "jpeg_pipe",
    "image2",
    "bmp_pipe",
    "webp_pipe",
    "tiff_pipe",
];

/// Normalized probe result — the fields the rest of the system needs, plus
/// the raw ffprobe JSON for anything else (`raw` is kept verbatim).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaProbe {
    /// Asset kind: "video" | "audio" | "image" (kinds module).
    pub kind: String,
    /// Container duration in ms (rounded). None for still images — they have
    /// no intrinsic duration (the bogus one-frame duration some image
    /// demuxers report is discarded; clip duration comes from the edit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Video stream geometry, if any video stream exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Average video frame rate (fps), if video. None for stills (the
    /// demuxer's nominal 25/1 is meaningless).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    /// True if at least one audio stream exists.
    pub has_audio: bool,
    /// Audio sample rate, if audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_rate: Option<u32>,
    /// Container/codec summary, e.g. "mov,mp4 / h264+aac".
    pub format: String,
    /// Raw ffprobe output for forensic use.
    pub raw: serde_json::Value,
}

/// Probe `path` via ffprobe and normalize (media-engine contract). Errors carry the
/// ffprobe stderr as cause. Still images (PNG/JPEG/…) probe successfully
/// with kind="image" and no duration; video/audio without a measurable
/// duration is an error (corrupt file).
pub fn probe(path: &Path) -> Result<MediaProbe, CutError> {
    let raw = crate::ffmpeg::ffprobe_json(path)?;
    normalize_ffprobe(raw, path)
}

fn normalize_ffprobe(raw: serde_json::Value, path: &Path) -> Result<MediaProbe, CutError> {
    let fmt = &raw["format"];
    let streams = raw["streams"].as_array().cloned().unwrap_or_default();
    let parse_secs = |v: &serde_json::Value| {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0)
    };
    let is_attached_pic = |s: &serde_json::Value| {
        s["disposition"]["attached_pic"]
            .as_i64()
            .or_else(|| s["attached_pic"].as_i64())
            .unwrap_or(0)
            != 0
    };

    // First video / first audio stream carry geometry + rate fields.
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && !is_attached_pic(s));
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");

    let format_name = fmt["format_name"].as_str().unwrap_or("unknown");
    let is_image = IMAGE_FORMAT_NAMES.contains(&format_name) && video.is_some() && audio.is_none();
    let kind = if is_image {
        kinds::IMAGE
    } else if video.is_some() {
        kinds::VIDEO
    } else {
        kinds::AUDIO
    };

    // Duration: prefer format.duration (container truth); fall back to the
    // longest stream duration (some raw streams lack a container duration).
    // Stills: discard the demuxer's bogus one-frame duration entirely.
    let duration_ms = if is_image {
        None
    } else {
        let duration_s = parse_secs(&fmt["duration"])
            .or_else(|| {
                streams
                    .iter()
                    .filter_map(|s| parse_secs(&s["duration"]))
                    .fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.max(d))))
            })
            .ok_or_else(|| {
                CutError::new(
                    cut_core::error_codes::FFMPEG,
                    format!("{} has no measurable duration", path.display()),
                    "neither format.duration nor any stream duration in ffprobe output",
                )
                .with_suggested_action("file may be corrupt — import real media")
            })?;
        Some((duration_s * 1000.0).round() as u64)
    };

    // avg_frame_rate is a rational string like "30/1"; "0/0" means unknown.
    // Stills get None — their nominal rate is a demuxer artifact.
    let fps = if is_image {
        None
    } else {
        video
            .and_then(|v| v["avg_frame_rate"].as_str())
            .and_then(|r| {
                let (num, den) = r.split_once('/')?;
                let (num, den) = (num.parse::<f64>().ok()?, den.parse::<f64>().ok()?);
                (den > 0.0 && num > 0.0)
                    .then(|| num / den)
                    .filter(|fps| fps.is_finite() && *fps > 0.0)
            })
    };

    // "mov,mp4 / h264+aac" style summary for humans and receipts.
    let codecs: Vec<&str> = streams
        .iter()
        .filter_map(|s| s["codec_name"].as_str())
        .collect();
    let format = format!(
        "{} / {}",
        format_name,
        if codecs.is_empty() {
            "no-streams".to_string()
        } else {
            codecs.join("+")
        }
    );

    Ok(MediaProbe {
        kind: kind.to_string(),
        duration_ms,
        width: probe_dimension(video, "width", path)?,
        height: probe_dimension(video, "height", path)?,
        fps,
        has_audio: audio.is_some(),
        audio_rate: audio
            .and_then(|a| a["sample_rate"].as_str())
            .and_then(|r| r.parse().ok()),
        format,
        raw,
    })
}

fn probe_dimension(
    video: Option<&serde_json::Value>,
    field: &str,
    path: &Path,
) -> Result<Option<u32>, CutError> {
    let Some(value) = video.and_then(|v| v[field].as_u64()) else {
        return Ok(None);
    };
    let dimension = u32::try_from(value).map_err(|_| {
        CutError::new(
            cut_core::error_codes::FFMPEG,
            format!("{} has unsupported video {field}", path.display()),
            format!("ffprobe reported {field}={value}, which exceeds u32::MAX"),
        )
        .with_suggested_action(
            "file metadata is invalid or unsupported; transcode or import a normal media file",
        )
    })?;
    if dimension == 0 {
        return Err(CutError::new(
            cut_core::error_codes::FFMPEG,
            format!("{} has invalid video {field}", path.display()),
            format!("ffprobe reported {field}=0"),
        )
        .with_suggested_action(
            "file metadata is invalid or unsupported; transcode or import a normal media file",
        ));
    }
    Ok(Some(dimension))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_rejects_probe_geometry_that_would_wrap() {
        let raw = json!({
            "format": {
                "format_name": "mov,mp4",
                "duration": "1.0"
            },
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": u64::from(u32::MAX) + 1,
                    "height": 720,
                    "avg_frame_rate": "30/1"
                }
            ]
        });

        let err = normalize_ffprobe(raw, Path::new("oversized.mp4"))
            .expect_err("oversized video width must be rejected");

        assert_eq!(err.code, cut_core::error_codes::FFMPEG);
        assert!(
            err.cause.contains("width"),
            "cause should name the bad geometry field: {}",
            err.cause
        );
    }

    #[test]
    fn normalize_ignores_attached_cover_art_when_classifying_audio() {
        let raw = json!({
            "format": {
                "format_name": "mp3",
                "duration": "2.5"
            },
            "streams": [
                {
                    "codec_type": "audio",
                    "codec_name": "mp3",
                    "sample_rate": "44100",
                    "duration": "2.5"
                },
                {
                    "codec_type": "video",
                    "codec_name": "mjpeg",
                    "width": 600,
                    "height": 600,
                    "avg_frame_rate": "0/0",
                    "disposition": { "attached_pic": 1 }
                }
            ]
        });

        let p = normalize_ffprobe(raw, Path::new("album-art.mp3"))
            .expect("audio with cover art should normalize");

        assert_eq!(p.kind, kinds::AUDIO);
        assert_eq!(p.duration_ms, Some(2500));
        assert_eq!(p.width, None);
        assert_eq!(p.height, None);
        assert_eq!(p.audio_rate, Some(44100));
    }

    #[test]
    fn normalize_rejects_negative_or_zero_duration() {
        for duration in ["-1.0", "0"] {
            let raw = json!({
                "format": {
                    "format_name": "mov,mp4",
                    "duration": duration
                },
                "streams": [
                    {
                        "codec_type": "video",
                        "codec_name": "h264",
                        "width": 1920,
                        "height": 1080,
                        "avg_frame_rate": "30/1"
                    }
                ]
            });

            let err = normalize_ffprobe(raw, Path::new("bad-duration.mp4"))
                .expect_err("non-positive video duration must be rejected");
            assert_eq!(err.code, cut_core::error_codes::FFMPEG);
            assert!(
                err.cause.contains("duration"),
                "cause should name duration: {}",
                err.cause
            );
        }
    }

    #[test]
    fn normalize_rejects_zero_video_geometry() {
        let raw = json!({
            "format": {
                "format_name": "mov,mp4",
                "duration": "1.0"
            },
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 0,
                    "height": 1080,
                    "avg_frame_rate": "30/1"
                }
            ]
        });

        let err = normalize_ffprobe(raw, Path::new("zero-width.mp4"))
            .expect_err("zero-width video geometry must be rejected");
        assert_eq!(err.code, cut_core::error_codes::FFMPEG);
        assert!(
            err.cause.contains("width"),
            "bad field named: {}",
            err.cause
        );
    }

    #[test]
    fn normalize_ignores_negative_frame_rate() {
        let raw = json!({
            "format": {
                "format_name": "mov,mp4",
                "duration": "1.0"
            },
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1920,
                    "height": 1080,
                    "avg_frame_rate": "-30/1"
                }
            ]
        });

        let p = normalize_ffprobe(raw, Path::new("negative-fps.mp4"))
            .expect("negative frame rate should be treated as unknown, not fatal");
        assert_eq!(p.fps, None);
    }
}
