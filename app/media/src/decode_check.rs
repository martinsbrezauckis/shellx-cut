//! How much of a source can really be decoded.
//!
//! ffprobe reads a file's index, not its picture. A file cut off part-way (a
//! download or copy that stopped, with the index at the front) still reports
//! its full length, so it imports, places and renders as if it were whole, and
//! the export ends early. The editing proxy decodes the whole source anyway;
//! its own stream lengths are the decodable length. Comparing them with what
//! the source claims costs one ffprobe of the small proxy.

use crate::probe::StreamExtent;
use serde_json::{json, Value};

/// Shortfall below which a difference is ordinary container rounding: the
/// larger of 250 ms and three source frames.
fn tolerance_ms(fps: Option<f64>) -> u64 {
    let frames = fps
        .filter(|f| f.is_finite() && *f > 0.0)
        .map(|f| (3000.0 / f).ceil() as u64)
        .unwrap_or(0);
    frames.max(250)
}

fn secs(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

/// What the source claims for its first video and first audio stream (stream
/// duration, else the container duration), from a stored asset probe.
fn claimed(probe: &Value, kind: &str) -> Option<u64> {
    let container = probe.get("duration_ms").and_then(Value::as_u64);
    let raw = probe.get("raw").cloned().unwrap_or(Value::Null);
    let extents = crate::probe::stream_extents_from_raw(&raw);
    extents
        .iter()
        .find(|e| e.kind == kind)
        .map(|e| e.duration_ms.or(container))
        .unwrap_or(None)
}

/// Compare the source probe (`asset.probe`) with the measured streams of its
/// proxy. Returns the record stored as `asset.probe.decode_check`:
/// `{checked_by:"proxy", claimed_picture_ms, decodable_picture_ms,
/// claimed_sound_ms?, decodable_sound_ms?, warning?}`. `warning` is set only
/// when the picture or the sound decodes noticeably shorter than claimed; it is
/// a sentence a person reads.
pub fn decode_check(source_probe: &Value, proxy_streams: &[StreamExtent]) -> Value {
    let fps = source_probe.get("fps").and_then(Value::as_f64);
    let tol = tolerance_ms(fps);
    let picture = |kind: &str| {
        proxy_streams
            .iter()
            .find(|e| e.kind == kind)
            .and_then(|e| e.duration_ms)
    };
    let claimed_picture = claimed(source_probe, "video");
    let claimed_sound = claimed(source_probe, "audio");
    let decoded_picture = picture("video");
    let decoded_sound = picture("audio");
    let short = |claimed: Option<u64>, decoded: Option<u64>| match (claimed, decoded) {
        (Some(c), Some(d)) => c > d.saturating_add(tol),
        // The source claims a stream the proxy could not produce at all.
        (Some(c), None) => c > tol,
        _ => false,
    };
    let picture_short = short(claimed_picture, decoded_picture);
    let sound_short = short(claimed_sound, decoded_sound);
    let mut record = json!({
        "checked_by": "proxy",
        "claimed_picture_ms": claimed_picture,
        "decodable_picture_ms": decoded_picture,
        "tolerance_ms": tol,
    });
    if claimed_sound.is_some() {
        record["claimed_sound_ms"] = json!(claimed_sound);
        record["decodable_sound_ms"] = json!(decoded_sound);
    }
    let what = match (picture_short, sound_short) {
        (true, true) => Some((
            "picture and sound",
            decoded_picture.unwrap_or(0).max(decoded_sound.unwrap_or(0)),
            claimed_picture.unwrap_or(0).max(claimed_sound.unwrap_or(0)),
        )),
        (true, false) => Some((
            "picture",
            decoded_picture.unwrap_or(0),
            claimed_picture.unwrap_or(0),
        )),
        (false, true) => Some((
            "sound",
            decoded_sound.unwrap_or(0),
            claimed_sound.unwrap_or(0),
        )),
        (false, false) => None,
    };
    // Damage INSIDE the file: the proxy reaches the end, but frames are
    // missing (zeroed or corrupt data in the middle). The frame count shows it.
    let claimed_frames =
        crate::probe::stream_extents_from_raw(source_probe.get("raw").unwrap_or(&Value::Null))
            .into_iter()
            .find(|e| e.kind == "video")
            .and_then(|e| e.frames);
    let decoded_frames = proxy_streams
        .iter()
        .find(|e| e.kind == "video")
        .and_then(|e| e.frames);
    if claimed_frames.is_some() {
        record["claimed_frames"] = json!(claimed_frames);
        record["decodable_frames"] = json!(decoded_frames);
    }
    let frames_missing = match (claimed_frames, decoded_frames) {
        (Some(c), Some(d)) if c > d => {
            let missing = c - d;
            // Ordinary end-of-file rounding is a frame or two.
            (missing > 2 && missing * 200 > c).then_some((missing, c))
        }
        _ => None,
    };
    if what.is_none() {
        if let Some((missing, total)) = frames_missing {
            record["warning"] = json!(format!(
                "{missing} of {total} frames of picture cannot be read: the file is damaged inside. \
                 An export shows a frozen or skipped picture there. Import an undamaged copy of the \
                 file if you have one."
            ));
            record["code"] = json!("decode_gap");
        }
    }
    if let Some((what, decoded, claimed)) = what {
        record["warning"] = json!(format!(
            "Only the first {} of {} of {what} can be read: the file is cut off or damaged. \
             An export will stop early where it is missing. Import a complete copy of the file \
             if you have one.",
            secs(decoded),
            secs(claimed)
        ));
        record["code"] = json!("decode_short");
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(duration_ms: u64, video_s: &str, audio_s: &str) -> Value {
        json!({
            "kind": "video", "duration_ms": duration_ms, "fps": 25.0,
            "raw": {"streams": [
                {"index": 0, "codec_type": "video", "duration": video_s},
                {"index": 1, "codec_type": "audio", "duration": audio_s}
            ]}
        })
    }

    fn proxy(video_ms: Option<u64>, audio_ms: Option<u64>) -> Vec<StreamExtent> {
        let mut v = Vec::new();
        if let Some(d) = video_ms {
            v.push(StreamExtent {
                kind: "video".into(),
                index: 0,
                duration_ms: Some(d),
                frames: None,
            });
        }
        if let Some(d) = audio_ms {
            v.push(StreamExtent {
                kind: "audio".into(),
                index: 1,
                duration_ms: Some(d),
                frames: None,
            });
        }
        v
    }

    #[test]
    fn a_cut_off_file_warns_with_the_readable_length() {
        // The box experiment: 8.0 s claimed, the proxy got 2.0 s / 1.92 s.
        let record = decode_check(
            &source(8000, "8.000000", "8.000000"),
            &proxy(Some(2000), Some(1920)),
        );
        assert_eq!(record["code"], "decode_short");
        let warning = record["warning"].as_str().unwrap();
        assert!(
            warning.starts_with("Only the first 2.0 s of 8.0 s of picture and sound can be read"),
            "{warning}"
        );
        assert_eq!(record["decodable_picture_ms"], 2000);
        assert_eq!(record["claimed_picture_ms"], 8000);
    }

    #[test]
    fn a_whole_file_has_no_warning() {
        let record = decode_check(
            &source(8000, "8.000000", "8.000000"),
            &proxy(Some(8000), Some(8000)),
        );
        assert!(record.get("warning").is_none(), "{record}");
        // Ordinary rounding (a frame or two, AAC padding) is not a warning.
        let record = decode_check(
            &source(8000, "8.000000", "8.021333"),
            &proxy(Some(7960), Some(8000)),
        );
        assert!(record.get("warning").is_none(), "{record}");
    }

    #[test]
    fn damage_inside_the_file_is_caught_by_the_frame_count() {
        // The HQ Linux run: 4 MB zeroed in the middle of the 780-frame phone
        // clip; the proxy reached the end with 721 frames.
        let mut src = source(13_313, "13.312500", "13.311792");
        src["raw"]["streams"][0]["nb_frames"] = json!("780");
        let mut proxy = proxy(Some(13_317), Some(13_313));
        proxy[0].frames = Some(721);
        let record = decode_check(&src, &proxy);
        assert_eq!(record["code"], "decode_gap", "{record}");
        assert!(record["warning"]
            .as_str()
            .unwrap()
            .starts_with("59 of 780 frames of picture cannot be read"));
        // One frame short is rounding, not damage.
        proxy[0].frames = Some(779);
        assert!(decode_check(&src, &proxy).get("warning").is_none());
    }

    #[test]
    fn only_the_short_stream_is_named() {
        let record = decode_check(
            &source(8000, "8.000000", "8.000000"),
            &proxy(Some(8000), Some(3000)),
        );
        let warning = record["warning"].as_str().unwrap();
        assert!(warning.contains("3.0 s of 8.0 s of sound"), "{warning}");
    }
}
