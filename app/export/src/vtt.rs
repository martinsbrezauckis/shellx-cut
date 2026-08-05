//! WebVTT serializer — public contract `export.vtt`, the HTML5 `<track>` caption standard
//! for web-published video. Same cue model as the SRT exporter (ms precision,
//! no frame quantization — captions are wall-clock), differing only in the
//! mandatory `WEBVTT` header and the `.` decimal separator on timestamps.

use crate::error::ExportError;
use crate::model::ExportTimeline;

/// ms -> WebVTT timestamp "HH:MM:SS.mmm" (period decimal separator per spec).
fn vtt_time(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let milli = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{milli:03}")
}

/// Render the first caption track as a WebVTT document. Cues emitted in track
/// order with 1-based indices; clips missing text or range are skipped. No
/// caption track or zero usable cues -> actionable error (captions.generate first).
pub fn render(tl: &ExportTimeline) -> Result<String, ExportError> {
    let track = tl
        .tracks
        .iter()
        .find(|t| t.kind == "caption")
        .ok_or(ExportError::NoCaptions)?;

    let mut out = String::from("WEBVTT\n\n");
    let mut index = 0u32;
    for clip in &track.clips {
        let (text, [start, end]) = match (clip.text.as_deref(), clip.range_ms) {
            (Some(t), Some(r)) if !t.is_empty() && r[1] > r[0] => (t, r),
            _ => continue,
        };
        index += 1;
        out.push_str(&format!(
            "{index}\n{} --> {}\n{text}\n\n",
            vtt_time(start),
            vtt_time(end)
        ));
    }
    if index == 0 {
        return Err(ExportError::NoCaptions);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExportTimeline;

    fn timeline(v: serde_json::Value) -> ExportTimeline {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn renders_webvtt_with_header_and_period_separator() {
        let tl = timeline(serde_json::json!({
            "settings": {},
            "tracks": [{"kind":"caption","clips":[{"text":"Hello world","range_ms":[100,1400]}]}]
        }));
        let out = render(&tl).unwrap();
        assert!(
            out.starts_with("WEBVTT\n\n"),
            "must start with the WEBVTT header"
        );
        assert!(
            out.contains("00:00:00.100 --> 00:00:01.400"),
            "period separator: {out}"
        );
        assert!(out.contains("Hello world"));
    }

    #[test]
    fn no_captions_errors() {
        let tl = timeline(serde_json::json!({"settings": {}, "tracks": []}));
        assert!(render(&tl).is_err());
    }
}
