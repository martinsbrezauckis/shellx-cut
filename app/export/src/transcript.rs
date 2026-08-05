//! Readable-transcript serializer — `export.transcript`, the script of the
//! FINAL cut for show notes / blog repurposing / an accessibility transcript
//! page. Reads the SAME caption track as the SRT/VTT exporters (so the text is
//! timeline-accurate — it reflects what survived the cut, never the raw
//! recording), but drops timing into PROSE: cues are joined into paragraphs,
//! a new paragraph starting after a natural pause between cues.
//!
//! Two formats: `txt` (plain paragraphs) and `md` (a heading + paragraphs,
//! optionally prefixed with `[mm:ss]` timestamps). No caption track / zero
//! usable cues → actionable error (captions.generate first), same as SRT/VTT.

use crate::error::ExportError;
use crate::model::ExportTimeline;

/// Output format for export.transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    /// Plain UTF-8 paragraphs separated by blank lines.
    Txt,
    /// Markdown: a `# Transcript` heading then paragraphs (optionally
    /// `**[mm:ss]**`-prefixed).
    Md,
}

impl TranscriptFormat {
    /// Parse the verb's string arg. Unknown value → actionable error.
    pub fn from_str(s: &str) -> Result<Self, ExportError> {
        match s {
            "txt" => Ok(Self::Txt),
            "md" => Ok(Self::Md),
            other => Err(ExportError::BadFormat(other.to_string())),
        }
    }

    /// Conventional file extension.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Md => "md",
        }
    }
}

/// A gap longer than this between one cue's end and the next cue's start starts
/// a new paragraph — a natural speaking pause is the paragraph boundary. 2 s is
/// the conventional "sentence/topic breath" used by transcription tools.
const PARAGRAPH_GAP_MS: u64 = 2000;

/// ms → "m:ss" (compact, for a transcript heading prefix — hours are rare for
/// the talking-head/screen-demo wedge; minutes can exceed 59 and that is fine).
fn mmss(ms: u64) -> String {
    let total_s = ms / 1000;
    format!("{}:{:02}", total_s / 60, total_s % 60)
}

/// Render the first caption track as a readable transcript. `timestamps` only
/// applies to `Md` (prefixes each paragraph with its start time). Cues missing
/// text or range are skipped; zero usable cues → `NoCaptions`.
pub fn render(
    tl: &ExportTimeline,
    format: TranscriptFormat,
    timestamps: bool,
) -> Result<String, ExportError> {
    let track = tl
        .tracks
        .iter()
        .find(|t| t.kind == "caption")
        .ok_or(ExportError::NoCaptions)?;

    // Collect usable (text, [start,end]) cues in track order.
    let cues: Vec<(&str, [u64; 2])> = track
        .clips
        .iter()
        .filter_map(|c| match (c.text.as_deref(), c.range_ms) {
            (Some(t), Some(r)) if !t.trim().is_empty() && r[1] > r[0] => Some((t.trim(), r)),
            _ => None,
        })
        .collect();
    if cues.is_empty() {
        return Err(ExportError::NoCaptions);
    }

    // Group cues into paragraphs, breaking on a > PARAGRAPH_GAP_MS pause.
    // Each paragraph: (start_ms, joined text).
    let mut paragraphs: Vec<(u64, String)> = Vec::new();
    let mut cur = String::new();
    let mut cur_start = cues[0].1[0];
    let mut prev_end = cues[0].1[0];
    for (i, (text, [start, end])) in cues.iter().enumerate() {
        if i > 0 && start.saturating_sub(prev_end) > PARAGRAPH_GAP_MS {
            paragraphs.push((cur_start, cur.trim().to_string()));
            cur = String::new();
            cur_start = *start;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(text);
        prev_end = *end;
    }
    if !cur.trim().is_empty() {
        paragraphs.push((cur_start, cur.trim().to_string()));
    }

    let mut out = String::new();
    if format == TranscriptFormat::Md {
        out.push_str("# Transcript\n\n");
    }
    for (i, (start, text)) in paragraphs.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        if format == TranscriptFormat::Md && timestamps {
            out.push_str(&format!("**[{}]** ", mmss(*start)));
        }
        out.push_str(text);
    }
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExportTimeline;

    fn timeline(v: serde_json::Value) -> ExportTimeline {
        serde_json::from_value(v).unwrap()
    }

    /// Two cues within the gap → one paragraph; a > 2 s pause → a new paragraph.
    #[test]
    fn groups_cues_into_paragraphs_on_pause() {
        let tl = timeline(serde_json::json!({
            "settings": {},
            "tracks": [{"kind":"caption","clips":[
                {"text":"Hello and welcome","range_ms":[0,1500]},
                {"text":"to the show.","range_ms":[1600,3000]},
                {"text":"Now the next topic.","range_ms":[6000,8000]}
            ]}]
        }));
        let txt = render(&tl, TranscriptFormat::Txt, false).unwrap();
        // First two cues join (gap 100ms); the 3-second gap starts a new para.
        assert!(
            txt.contains("Hello and welcome to the show."),
            "joined: {txt:?}"
        );
        assert_eq!(
            txt.matches("\n\n").count(),
            1,
            "exactly one paragraph break: {txt:?}"
        );
        assert!(!txt.contains('#'), "txt has no markdown heading");

        // Markdown with timestamps: heading + a [m:ss] on each paragraph.
        let md = render(&tl, TranscriptFormat::Md, true).unwrap();
        assert!(md.starts_with("# Transcript\n\n"), "md heading: {md:?}");
        assert!(md.contains("**[0:00]**"), "first para timestamp: {md:?}");
        assert!(md.contains("**[0:06]**"), "second para timestamp: {md:?}");
    }

    #[test]
    fn no_captions_errors() {
        let tl = timeline(serde_json::json!({"settings": {}, "tracks": []}));
        assert!(render(&tl, TranscriptFormat::Txt, false).is_err());
    }
}
