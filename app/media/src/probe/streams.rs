//! Per-stream length of a media file.
//!
//! A container's duration is its LONGEST stream. A file whose picture stops
//! after 1.7 s while its sound runs 7.6 s still reports 7.6 s, so any check
//! that reads only the container cannot see a short picture. This module
//! reports each video and audio stream on its own: its duration and, for
//! video, its frame count.

use cut_core::CutError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One video or audio stream of a file, as measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamExtent {
    /// "video" | "audio".
    pub kind: String,
    /// ffprobe stream index.
    pub index: u32,
    /// Stream duration in ms, when the file states one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Video frame count (packets), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
}

/// "HH:MM:SS.fffffffff" (Matroska/WebM `DURATION` tag) -> ms.
fn tag_duration_ms(tag: &str) -> Option<u64> {
    let mut parts = tag.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    let total = (h * 3600.0 + m * 60.0 + s) * 1000.0;
    (total.is_finite() && total >= 0.0).then(|| total.round() as u64)
}

fn number(v: &serde_json::Value) -> Option<f64> {
    v.as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| v.as_f64())
        .filter(|n| n.is_finite() && *n >= 0.0)
}

/// Read the stream extents out of ffprobe's `-show_streams` JSON. Attached
/// cover pictures are not video. `nb_read_packets` (from `-count_packets`) is
/// used when `nb_frames` is missing.
pub fn stream_extents_from_raw(raw: &serde_json::Value) -> Vec<StreamExtent> {
    let streams = raw["streams"].as_array().cloned().unwrap_or_default();
    streams
        .iter()
        .filter_map(|s| {
            let kind = s["codec_type"].as_str()?;
            if kind != "video" && kind != "audio" {
                return None;
            }
            if s["disposition"]["attached_pic"].as_i64().unwrap_or(0) != 0 {
                return None;
            }
            let duration_ms = number(&s["duration"])
                .map(|secs| (secs * 1000.0).round() as u64)
                .or_else(|| s["tags"]["DURATION"].as_str().and_then(tag_duration_ms));
            let frames = (kind == "video")
                .then(|| {
                    number(&s["nb_frames"])
                        .or_else(|| number(&s["nb_read_packets"]))
                        .map(|n| n as u64)
                })
                .flatten();
            Some(StreamExtent {
                kind: kind.to_string(),
                index: s["index"].as_u64().unwrap_or(0) as u32,
                duration_ms,
                frames,
            })
        })
        .collect()
}

/// Measure every video and audio stream of `path`. A video stream whose
/// container does not state a frame count (Matroska/WebM) is counted by
/// reading its packets (demux only, no decode).
pub fn stream_extents(path: &Path) -> Result<Vec<StreamExtent>, CutError> {
    let raw = crate::ffmpeg::ffprobe_json(path)?;
    let mut extents = stream_extents_from_raw(&raw);
    if extents
        .iter()
        .any(|e| e.kind == "video" && e.frames.is_none())
    {
        let counted = crate::ffmpeg::ffprobe_packet_counts(path)?;
        for extent in extents.iter_mut().filter(|e| e.frames.is_none()) {
            if extent.kind == "video" {
                extent.frames = counted
                    .iter()
                    .find(|(index, _)| *index == extent.index)
                    .map(|(_, n)| *n);
            }
        }
    }
    Ok(extents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_short_picture_under_long_sound_is_visible_per_stream() {
        // The truncated-source render of the HQ run: container 7.615 s.
        let raw = json!({
            "format": {"duration": "7.615000"},
            "streams": [
                {"index": 0, "codec_type": "video", "duration": "1.720000", "nb_frames": "43"},
                {"index": 1, "codec_type": "audio", "duration": "7.615000", "nb_frames": "358"},
                {"index": 2, "codec_type": "video", "disposition": {"attached_pic": 1}}
            ]
        });
        let extents = stream_extents_from_raw(&raw);
        assert_eq!(
            extents,
            vec![
                StreamExtent {
                    kind: "video".into(),
                    index: 0,
                    duration_ms: Some(1720),
                    frames: Some(43)
                },
                StreamExtent {
                    kind: "audio".into(),
                    index: 1,
                    duration_ms: Some(7615),
                    frames: None
                },
            ]
        );
    }

    #[test]
    fn webm_durations_come_from_the_stream_tag() {
        let raw = json!({"streams": [
            {"index": 0, "codec_type": "video", "tags": {"DURATION": "00:00:03.000000000"}, "nb_read_packets": "90"},
            {"index": 1, "codec_type": "audio", "tags": {"DURATION": "00:01:03.008000000"}}
        ]});
        let extents = stream_extents_from_raw(&raw);
        assert_eq!(extents[0].duration_ms, Some(3000));
        assert_eq!(extents[0].frames, Some(90));
        assert_eq!(extents[1].duration_ms, Some(63008));
    }
}
