//! SRT (SubRip) serializer — public verb contract `export.srt`, the ONLY caption exporter
//! (the canonical-export contract dropped captions.export_srt; the XML formats exclude captions by
//! design). Millisecond-precision, no frame quantization: SRT is
//! wall-clock, and caption timing should not inherit video frame rounding.

use crate::error::ExportError;
use crate::model::ExportTimeline;

/// ms -> SRT timestamp "HH:MM:SS,mmm" (comma decimal separator per spec).
fn srt_time(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let milli = ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{milli:03}")
}

/// Render the first caption track as an SRT document. Cues are emitted in
/// track order with 1-based indices; clips missing text or range are skipped
/// (a caption without timing is meaningless, not fatal). No caption track or
/// zero usable cues -> actionable error (run captions.generate first).
pub fn render(tl: &ExportTimeline) -> Result<String, ExportError> {
    let track = tl
        .tracks
        .iter()
        .find(|t| t.kind == "caption")
        .ok_or(ExportError::NoCaptions)?;

    let mut out = String::new();
    let mut index = 0u32;
    for clip in &track.clips {
        let (text, [start, end]) = match (clip.text.as_deref(), clip.range_ms) {
            (Some(t), Some(r)) if !t.is_empty() && r[1] > r[0] => (t, r),
            _ => continue,
        };
        index += 1;
        out.push_str(&format!(
            "{index}\n{} --> {}\n{text}\n\n",
            srt_time(start),
            srt_time(end)
        ));
    }
    if index == 0 {
        return Err(ExportError::NoCaptions);
    }
    Ok(out)
}
