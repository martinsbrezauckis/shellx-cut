//! captions_in.rs — SRT / WebVTT subtitle IMPORT parser.
//!
//! Role: the inverse of [`crate::srt`] (`export.srt`) — turn an external `.srt`
//! or `.vtt` file into timed cues the `captions.import` verb materializes as
//! caption clips on the `cap1` track, so subtitles ROUND-TRIP (import → edit →
//! re-export). One LENIENT, UNIFIED parser handles both formats: it scans for
//! `-->` timing lines (SRT uses `HH:MM:SS,mmm`, VTT uses `HH:MM:SS.mmm` and may
//! drop the hours), collecting the following non-blank lines as the cue text. It
//! skips SRT index numbers and VTT header/NOTE/STYLE/REGION blocks for free
//! (they carry no `-->`), and strips VTT inline tags (`<...>`).
//!
//! Determinism: pure function of the input text. Callers: dispatch.rs
//! (`captions.import`). No deps beyond std + the crate error type.

use crate::error::ExportError;

/// One imported subtitle cue: an absolute timeline span + its text (which may
/// contain `\n` for multi-line cues).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCue {
    /// Cue start on the timeline, milliseconds.
    pub start_ms: u64,
    /// Cue end on the timeline, milliseconds (always `> start_ms`).
    pub end_ms: u64,
    /// Cue text (lines joined with `\n`; inline tags stripped).
    pub text: String,
}

/// Best-effort format label for the result: `"ass"` for SubStation Alpha (a
/// `[Script Info]`/`[Events]` section or a `Dialogue:` line), `"vtt"` for the
/// WEBVTT signature, else `"srt"`. Drives which parser [`parse`] dispatches to.
pub fn detect_format(content: &str) -> &'static str {
    let head = content.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("[Script Info]")
        || content.contains("\n[Events]")
        || content.contains("\nDialogue:")
        || head.starts_with("Dialogue:")
    {
        "ass"
    } else if head.starts_with("WEBVTT") {
        "vtt"
    } else {
        "srt"
    }
}

/// Strip ASS text down to plain text: drop `{…}` override blocks (incl. `\k`
/// karaoke tags), turn ASS line breaks (`\N` hard / `\n` soft) into newlines and
/// `\h` into a space. Everything else is kept literal.
fn strip_ass_text(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '}') {
                i = i + 1 + rel + 1; // skip the whole {…} override block
                continue;
            }
            // unmatched '{' → keep literal
        }
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'N' | 'n' => {
                    out.push('\n');
                    i += 2;
                    continue;
                }
                'h' => {
                    out.push(' ');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Parse ASS/SSA `Dialogue:` events into cues. The Events `Format:` line fixes 9
/// leading fields (Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect) then
/// Text — which itself may contain commas, so we splitn(10) and take the remainder
/// as Text. Start/End are ASS `H:MM:SS.cc` times ([`parse_ts`] handles them: the
/// 2-digit centiseconds pad to ms). Override tags (`\k`, colours, positions) are
/// stripped, so a karaoke-exported .ass round-trips back to plain caption lines.
fn parse_ass(content: &str) -> Vec<ImportedCue> {
    let mut cues = Vec::new();
    for raw in content.lines() {
        let Some(rest) = raw.trim_start().strip_prefix("Dialogue:") else {
            continue;
        };
        let fields: Vec<&str> = rest.splitn(10, ',').collect();
        if fields.len() < 10 {
            continue;
        }
        let (Some(start), Some(end)) = (parse_ts(fields[1]), parse_ts(fields[2])) else {
            continue;
        };
        let text = strip_ass_text(fields[9]).trim().to_string();
        if end > start && !text.is_empty() {
            cues.push(ImportedCue {
                start_ms: start,
                end_ms: end,
                text,
            });
        }
    }
    cues
}

/// Parse one timestamp token into milliseconds. Accepts `HH:MM:SS,mmm`,
/// `HH:MM:SS.mmm`, and the VTT short form `MM:SS.mmm` (hours omitted). The
/// fractional separator may be `,` or `.`. Returns None on anything malformed.
fn parse_ts(tok: &str) -> Option<u64> {
    let tok = tok.trim();
    // Split off the fractional seconds on the LAST ',' or '.'.
    let (hms, frac) = match tok.rfind([',', '.']) {
        Some(i) => (&tok[..i], &tok[i + 1..]),
        None => (tok, ""),
    };
    // interpret the fractional part as a DECIMAL FRACTION OF A SECOND, not
    // a literal integer count of ms. The standard is 3 digits, but lenient input
    // uses fewer/more: ",5"=0.5s=500ms, ",05"=50ms, ",0500"=50ms. Pad or truncate
    // the digits to 3 (ms precision) before parsing. (The old `ms.parse()` read
    // ",5" as 5ms and ",0500" as 500ms — both wrong.)
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None; // non-digit fractional → malformed
    }
    let mut digits = frac.to_string();
    while digits.len() < 3 {
        digits.push('0');
    }
    digits.truncate(3);
    let ms: u64 = digits.parse().ok()?; // 0..=999 by construction
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, s): (u64, u64, u64) = match parts.as_slice() {
        [h, m, s] => (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?),
        [m, s] => (0, m.parse().ok()?, s.parse().ok()?),
        _ => return None,
    };
    // Bound the components (mirrors the m/s/ms guards): a real subtitle is far
    // under 1000 hours, and this prevents `h * 3600 * 1000` overflowing u64 on a
    // pathological line (which would panic in debug / silently wrap in release).
    if h > 999 || m > 59 || s > 59 {
        return None;
    }
    Some(((h * 3600 + m * 60 + s) * 1000) + ms)
}

/// Strip angle-bracket tags (`<v Bob>`, `<c.yellow>`, inline `<00:00:01.000>`)
/// from a subtitle text line, keeping the visible characters.
///
/// L1 fix: only strip a WELL-FORMED `<…>` (a `<` with a matching `>` ahead). A
/// bare `<` used as a less-than — common in SRT dialogue like `5 < 3` — has no
/// closing `>` and is kept LITERAL. The old depth-counter ate everything after
/// any `<`, silently dropping the rest of such a line.
fn strip_tags(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '>') {
                i = i + 1 + rel + 1; // skip the whole `<…>` tag (past the `>`)
                continue;
            }
            // no closing `>` → a literal less-than; fall through and keep it.
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// True if `line` is a valid cue TIMING line (`<ts> --> <ts>` with both sides
/// parseable). Used to detect the start of the next cue even when a blank-line
/// separator is missing.
fn is_timing_line(line: &str) -> bool {
    match line.split_once("-->") {
        Some((l, r)) => {
            l.split_whitespace().last().and_then(parse_ts).is_some()
                && r.split_whitespace().next().and_then(parse_ts).is_some()
        }
        None => false,
    }
}

/// Parse SRT or VTT content into cues (the format is auto-handled). Errors only
/// when ZERO usable cues are found (an empty / non-subtitle file).
pub fn parse(content: &str) -> Result<Vec<ImportedCue>, ExportError> {
    // Normalize newlines + strip a BOM so the line scan is uniform.
    let norm = content
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    // ASS/SSA has no `-->` timing lines — dispatch to the Dialogue parser.
    if detect_format(&norm) == "ass" {
        let cues = parse_ass(&norm);
        if cues.is_empty() {
            return Err(ExportError::BadSubtitle(
                "no ASS Dialogue events found (expected lines like 'Dialogue: 0,0:00:01.00,0:00:03.00,…')"
                    .into(),
            ));
        }
        return Ok(cues);
    }
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut cues = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((left, right)) = line.split_once("-->") {
            // The token ADJACENT to `-->` on each side is the timestamp (VTT cue
            // settings trail the end time and are ignored).
            let start = left.split_whitespace().last().and_then(parse_ts);
            let end = right.split_whitespace().next().and_then(parse_ts);
            if let (Some(start), Some(end)) = (start, end) {
                i += 1;
                let mut text_lines = Vec::new();
                while i < lines.len() && !lines[i].trim().is_empty() {
                    // stop at the START of the next cue even WITHOUT a blank
                    // separator (lenient: separator-less SRT exists in the wild).
                    // The next cue begins at either its timing line, or an SRT index
                    // line (pure digits) immediately followed by a timing line —
                    // without this, that cue's index+timing+text are swallowed into
                    // THIS cue's text and the next cue is lost entirely.
                    if is_timing_line(lines[i]) {
                        break;
                    }
                    let t = lines[i].trim();
                    if !t.is_empty()
                        && t.bytes().all(|b| b.is_ascii_digit())
                        && i + 1 < lines.len()
                        && is_timing_line(lines[i + 1])
                    {
                        break;
                    }
                    text_lines.push(strip_tags(lines[i]));
                    i += 1;
                }
                let text = text_lines.join("\n").trim().to_string();
                if end > start && !text.is_empty() {
                    cues.push(ImportedCue {
                        start_ms: start,
                        end_ms: end,
                        text,
                    });
                }
                continue;
            }
        }
        i += 1;
    }
    if cues.is_empty() {
        return Err(ExportError::BadSubtitle(
            "no cues found (expected SRT/VTT lines like '00:00:01,000 --> 00:00:03,000')".into(),
        ));
    }
    Ok(cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_srt() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nHello world\n\n2\n00:00:04,000 --> 00:00:06,000\nSecond line\nwrapped\n\n";
        let cues = parse(srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(
            cues[0],
            ImportedCue {
                start_ms: 1000,
                end_ms: 3500,
                text: "Hello world".into()
            }
        );
        assert_eq!(cues[1].text, "Second line\nwrapped");
        assert_eq!(cues[1].start_ms, 4000);
        assert_eq!(detect_format(srt), "srt");
    }

    #[test]
    fn parse_vtt_with_tags_and_short_ts() {
        let vtt = "WEBVTT\n\nNOTE this is a comment\n\ncue-1\n00:01.000 --> 00:03.000 align:start\n<v Bob>Hi <c.yellow>there</c>\n\n00:00:05.000 --> 00:00:07.000\nplain\n";
        assert_eq!(detect_format(vtt), "vtt");
        let cues = parse(vtt).unwrap();
        assert_eq!(cues.len(), 2);
        // MM:SS.mmm short form (no hours) → 1s..3s; tags stripped; settings ignored.
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 3000);
        // All <...> stripped — incl. the <v Bob> voice tag (Bob is tag metadata,
        // not inline display text), so only the visible "Hi there" remains.
        assert_eq!(cues[0].text, "Hi there");
        assert_eq!(cues[1].text, "plain");
    }

    #[test]
    fn parse_ts_forms() {
        assert_eq!(parse_ts("00:00:01,000"), Some(1000));
        assert_eq!(parse_ts("01:02:03.250"), Some(3_723_250));
        assert_eq!(parse_ts("02:05.500"), Some(125_500)); // MM:SS.mmm
        assert_eq!(parse_ts("00:00:00,000"), Some(0));
        // sub-3-digit and 4+-digit fractional seconds are DECIMAL
        // fractions, not literal ms. ",5"=500ms, ",50"=500ms, ",05"=50ms,
        // ",0500"=50ms (truncated to ms precision), ",500"=500ms (unchanged).
        assert_eq!(parse_ts("00:00:01,5"), Some(1500));
        assert_eq!(parse_ts("00:00:01,50"), Some(1500));
        assert_eq!(parse_ts("00:00:01,05"), Some(1050));
        assert_eq!(parse_ts("00:00:01,0500"), Some(1050));
        assert_eq!(parse_ts("00:00:01,500"), Some(1500));
        assert_eq!(parse_ts("00:00:01"), Some(1000)); // no fractional → .000
        assert_eq!(parse_ts("00:00:01,abc"), None); // non-digit fractional
        assert_eq!(parse_ts("garbage"), None);
        assert_eq!(parse_ts("00:99:00,000"), None); // minutes out of range
                                                    // a pathological hour count must return None, NOT overflow
                                                    // (debug panic) / wrap (release). 999h is the bound; beyond → None.
        assert_eq!(parse_ts("5124095576031:00:00,000"), None);
        assert_eq!(parse_ts("1000:00:00,000"), None);
        assert!(parse_ts("999:00:00,000").is_some()); // at the bound, still valid
    }

    #[test]
    fn empty_or_junk_errors() {
        assert!(parse("").is_err());
        assert!(parse("just some text\nno timings here\n").is_err());
        // A cue with end <= start is dropped (here the only cue) → error.
        assert!(parse("1\n00:00:03,000 --> 00:00:01,000\nbackwards\n").is_err());
    }

    #[test]
    fn parse_missing_blank_separators() {
        // an SRT with NO blank lines between cues must still yield all
        // cues — the old loop swallowed cue 2's index+timing+text into cue 1's
        // text and lost cue 2 entirely.
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nFirst\n2\n00:00:03,000 --> 00:00:04,000\nSecond\n3\n00:00:05,000 --> 00:00:06,000\nThird\n";
        let cues = parse(srt).unwrap();
        assert_eq!(cues.len(), 3, "all 3 separator-less cues recovered");
        assert_eq!(cues[0].text, "First");
        assert_eq!(cues[1].text, "Second");
        assert_eq!(cues[2].text, "Third");
        assert_eq!(cues[1].start_ms, 3000);
        // separator-less with NO index lines (back-to-back timing+text) too.
        let noidx = "00:00:01,000 --> 00:00:02,000\nA\n00:00:03,000 --> 00:00:04,000\nB\n";
        let c2 = parse(noidx).unwrap();
        assert_eq!(c2.len(), 2);
        assert_eq!(c2[0].text, "A");
        assert_eq!(c2[1].text, "B");
    }

    #[test]
    fn strip_tags_keeps_bare_less_than() {
        // L1 fix: a bare `<` (less-than) with no closing `>` is LITERAL, not a tag.
        assert_eq!(strip_tags("5 < 3 ok"), "5 < 3 ok");
        assert_eq!(strip_tags("if a<b then c"), "if a<b then c");
        // well-formed tags still stripped.
        assert_eq!(strip_tags("<v Bob>Hi <c.yellow>there</c>"), "Hi there");
        assert_eq!(strip_tags("plain"), "plain");
        // a real SRT cue whose dialogue contains `<` keeps its full text.
        let srt = "1\n00:00:01,000 --> 00:00:02,000\n5 < 3 is false\n";
        assert_eq!(parse(srt).unwrap()[0].text, "5 < 3 is false");
    }

    #[test]
    fn skips_index_only_and_blank_noise() {
        // Leading blank lines + index numbers are ignored (no '-->').
        let srt = "\n\n1\n00:00:00,500 --> 00:00:02,000\nA\n\n";
        let cues = parse(srt).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "A");
        assert_eq!(cues[0].start_ms, 500);
    }

    /// ASS import: detect the format, parse Dialogue Start/End (centiseconds), and
    /// STRIP override tags incl. `\k` karaoke + comma-bearing Text — so a karaoke
    /// .ass round-trips to plain caption lines.
    #[test]
    fn parse_ass_dialogue_with_karaoke_and_commas() {
        let ass = "[Script Info]\nScriptType: v4.00+\n\n[V4+ Styles]\n\
                   Format: Name, Fontname\nStyle: Default,Inter\n\n[Events]\n\
                   Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                   Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,{\\k50}hello {\\k50}world\n\
                   Dialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,one, two, three\n";
        assert_eq!(detect_format(ass), "ass");
        let cues = parse(ass).unwrap();
        assert_eq!(cues.len(), 2, "two Dialogue events");
        // \k tags stripped; words kept.
        assert_eq!(cues[0].text, "hello world");
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 3500, "0:00:03.50 = 3500ms (cc→ms)");
        // Text with commas survives (splitn(10) keeps the remainder).
        assert_eq!(cues[1].text, "one, two, three");
    }

    /// \N hard line break in ASS becomes a newline (matches SRT/VTT multi-line).
    #[test]
    fn parse_ass_line_break() {
        let ass = "[Events]\nDialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,top\\Nbottom\n";
        let cues = parse(ass).unwrap();
        assert_eq!(cues[0].text, "top\nbottom");
    }

    /// A non-subtitle ASS-ish file with no Dialogue events errors cleanly.
    #[test]
    fn parse_ass_without_events_errors() {
        let ass = "[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nStyle: Default\n";
        assert!(parse(ass).is_err());
    }
}
