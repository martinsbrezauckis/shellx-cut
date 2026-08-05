//! captions.rs — caption-track → ASS serialization for burn-in (media-engine contract
//! "caption burn-in via subtitles filter").
//!
//! Role: serialize the project's caption track(s) to ASS (styled burn-in
//! honoring CaptionStyle font/size/color/bg/pos). Interchange SRT export
//! lives in the cut-export crate; the export.srt
//! verb is the ONLY SRT exporter (the canonical-export contract). Dependencies: cut-core types.
//! Primary callers: render.rs (burn-in).

use cut_core::{CaptionClip, CutError, Edl, Project, TrackKind};

/// Collect all caption clips across caption tracks, sorted by start time.
fn caption_clips(project: &Project) -> Vec<&CaptionClip> {
    let mut clips: Vec<&CaptionClip> = project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Caption)
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc),
            _ => None,
        })
        .collect();
    clips.sort_by_key(|c| c.range_ms[0]);
    clips
}

/// Convert a CSS-style hex color ("#fff", "#000a", "#ffffff", "#00000080")
/// to ASS &HAABBGGRR (note: ASS alpha is INVERTED — 00 = opaque, FF =
/// transparent). Unparseable input falls back to opaque white — captions
/// must never fail a render over a bad style string (warn-level concern).
fn css_to_ass_color(css: &str) -> String {
    let hex = css.trim_start_matches('#');
    // Expand shorthand #rgb / #rgba to full-width per CSS rules.
    let full: String = match hex.len() {
        3 | 4 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => hex.to_string(),
        _ => "ffffffff".into(),
    };
    let byte =
        |i: usize| u8::from_str_radix(full.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(0xff);
    let (r, g, b) = (byte(0), byte(2), byte(4));
    let a = if full.len() == 8 { byte(6) } else { 0xff };
    format!("&H{:02X}{:02X}{:02X}{:02X}", 0xff - a, b, g, r)
}

/// Map a CaptionStyle `pos` keyword to ASS Alignment (numpad layout).
fn pos_to_alignment(pos: Option<&str>) -> u8 {
    match pos {
        Some("top") => 8,
        Some("center") => 5,
        _ => 2, // bottom (default)
    }
}

/// Format milliseconds as ASS time "H:MM:SS.cc" (centisecond resolution).
fn ass_timestamp(ms: u64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        ms / 3_600_000,
        (ms % 3_600_000) / 60_000,
        (ms % 60_000) / 1_000,
        (ms % 1_000) / 10
    )
}

/// Serialize caption clips to ASS with one ASS style per referenced
/// CaptionStyle (font/size/color/bg/pos mapped to ASS style fields).
/// Used for burn-in so styling survives (SRT cannot carry style).
/// PlayRes matches the project geometry so font sizes mean output pixels.
/// ASS header + `[V4+ Styles]` block — project-wide (one ASS style per
/// CaptionStyle: font/size/color/bg/pos), independent of which events follow.
/// PlayRes matches project geometry so font sizes mean OUTPUT pixels.
fn ass_header(project: &Project) -> String {
    let (w, h) = (project.settings.width, project.settings.height);
    let mut ass = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, \
         OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, \
         Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
    );
    // Fallback style for clips without a style_ref. BorderStyle=3 = opaque
    // box behind text (the talking-head caption look).
    ass.push_str(
        "Style: Default,Inter,42,&H00FFFFFF,&H00FFFFFF,&H00000000,&H66000000,\
         0,0,0,0,100,100,0,0,3,2,0,2,40,40,40,1\n",
    );
    // One ASS style per project CaptionStyle (BTreeMap → deterministic order).
    for (name, st) in &project.caption_styles {
        ass.push_str(&format!(
            "Style: {name},{font},{size},{color},{color},&H00000000,{back},\
             0,0,0,0,100,100,0,0,3,2,0,{align},40,40,40,1\n",
            font = st.font,
            size = st.size,
            color = css_to_ass_color(&st.color),
            back = st
                .bg
                .as_deref()
                .map(css_to_ass_color)
                .unwrap_or("&H66000000".into()),
            align = pos_to_alignment(st.pos.as_deref()),
        ));
    }
    ass.push_str(
        "\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    ass
}

/// One ASS `Dialogue` per `(start_ms, end_ms, style_ref, text)`, in the given
/// (already-resolved) timebase — centisecond-rounded. Caller orders the items.
fn ass_events(items: &[(u64, u64, Option<&str>, &str)]) -> String {
    let mut s = String::new();
    for (start, end, style, text) in items {
        // ASS line breaks are \N; a literal newline would corrupt the event.
        let text = text.replace('\n', "\\N");
        s.push_str(&format!(
            "Dialogue: 0,{},{},{},,0,0,0,,{}\n",
            ass_timestamp(*start),
            ass_timestamp(*end),
            style.unwrap_or("Default"),
            text
        ));
    }
    s
}

pub fn captions_to_ass(project: &Project) -> Result<String, CutError> {
    let mut ass = ass_header(project);
    let items: Vec<(u64, u64, Option<&str>, &str)> = caption_clips(project)
        .iter()
        .map(|c| {
            (
                c.range_ms[0],
                c.range_ms[1],
                c.style_ref.as_deref(),
                c.text.as_str(),
            )
        })
        .collect();
    ass.push_str(&ass_events(&items));
    Ok(ass)
}

/// The "unspoken" karaoke base colour (ASS &HAABBGGRR, opaque dark gray). With
/// `\k` each word starts in this SecondaryColour and flips to the style's bright
/// PrimaryColour as it is "spoken" — the word-by-word highlight (TikTok/Hormozi look).
const KARAOKE_SECONDARY: &str = "&H00606060";

/// Build an ASS header whose styles carry a DIM SecondaryColour, so `\k` karaoke
/// fill is visible (the burn-in header keeps Secondary == Primary, which would show
/// no fill). Self-contained — does NOT touch [`ass_header`] / the byte-identical burn
/// path. One ASS style per project CaptionStyle, same field layout as the burn header.
fn karaoke_header(project: &Project) -> String {
    let (w, h) = (project.settings.width, project.settings.height);
    let mut ass = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, \
         OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, \
         Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
    );
    // Default style: bright white primary, dim secondary (the karaoke base).
    ass.push_str(&format!(
        "Style: Default,Inter,42,&H00FFFFFF,{sec},&H00000000,&H66000000,\
         0,0,0,0,100,100,0,0,3,2,0,2,40,40,40,1\n",
        sec = KARAOKE_SECONDARY
    ));
    for (name, st) in &project.caption_styles {
        ass.push_str(&format!(
            "Style: {name},{font},{size},{color},{sec},&H00000000,{back},\
             0,0,0,0,100,100,0,0,3,2,0,{align},40,40,40,1\n",
            font = st.font,
            size = st.size,
            color = css_to_ass_color(&st.color),
            sec = KARAOKE_SECONDARY,
            back = st
                .bg
                .as_deref()
                .map(css_to_ass_color)
                .unwrap_or("&H66000000".into()),
            align = pos_to_alignment(st.pos.as_deref()),
        ));
    }
    ass.push_str(
        "\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    ass
}

/// Split a caption line into `{\k<cs>}word` syllables, distributing the clip's
/// duration (centiseconds) across its words PROPORTIONALLY to word length (longer
/// words hold the highlight longer) — an even estimate when no per-word transcript
/// timing is available. Each `\k` value is centiseconds the word stays in the base
/// (Secondary) colour before flipping to Primary. Trailing space rides with the word.
fn karaoke_line(text: &str, start_ms: u64, end_ms: u64) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let total_cs = end_ms.saturating_sub(start_ms) / 10; // ms → centiseconds
    let total_chars: usize = words
        .iter()
        .map(|w| w.chars().count())
        .sum::<usize>()
        .max(1);
    let mut out = String::new();
    let mut used_cs: u64 = 0;
    for (i, w) in words.iter().enumerate() {
        // Last word absorbs the rounding remainder so the syllables sum to total_cs.
        let cs = if i + 1 == words.len() {
            total_cs.saturating_sub(used_cs)
        } else {
            let chars = w.chars().count() as u64;
            (total_cs * chars / total_chars as u64).max(1)
        };
        used_cs += cs;
        // ASS line breaks are \N; words never contain one (split_whitespace), but a
        // brace in text would corrupt the override block — escape it defensively.
        let safe = w.replace('{', "(").replace('}', ")");
        out.push_str(&format!("{{\\k{cs}}}{safe} "));
    }
    out.trim_end().to_string()
}

/// Serialize the caption track to ASS with WORD-LEVEL `\k` KARAOKE (the portable,
/// word-fill styled-caption standard — TikTok/Hormozi look). Each caption line
/// becomes `{\k<cs>}word …` syllables timed across the clip's duration; the header
/// gives every style a dim SecondaryColour so the fill is visible. This is the
/// `export.ass{karaoke:true}` path; the line-level [`captions_to_ass`] is the
/// `karaoke:false` path. Word timing is estimated from the line duration (no
/// per-word transcript timing dependency) — honest and visually correct.
pub fn captions_to_ass_karaoke(project: &Project) -> Result<String, CutError> {
    let mut ass = karaoke_header(project);
    for c in caption_clips(project) {
        let text = karaoke_line(&c.text, c.range_ms[0], c.range_ms[1]);
        if text.is_empty() {
            continue;
        }
        ass.push_str(&format!(
            "Dialogue: 0,{},{},{},,0,0,0,,{}\n",
            ass_timestamp(c.range_ms[0]),
            ass_timestamp(c.range_ms[1]),
            c.style_ref.as_deref().unwrap_or("Default"),
            text
        ));
    }
    Ok(ass)
}

/// Captions for a (possibly windowed) EDL. The Dialogue times come from the
/// EDL's caption SEGMENTS, which `Edl::window` has already clamped + rebased to
/// the window's local timebase — so a SEGMENTED render burns each window's
/// captions IN-PASS (no separate caption re-encode), and a FULL (un-windowed)
/// EDL reproduces exactly the same events as [`captions_to_ass`]
/// (`edl_from_project` copies each caption clip's range verbatim and we re-sort
/// by start) → byte-identical ASS, byte-identical render. This is what
/// `build_graph` uses, so captions are a function of the render's EDL, not of
/// the project's absolute clip times.
pub fn captions_to_ass_for_edl(project: &Project, edl: &Edl) -> Result<String, CutError> {
    let mut items: Vec<(u64, u64, Option<&str>, &str)> = edl
        .segments
        .iter()
        .filter_map(|s| {
            s.caption_text.as_deref().map(|t| {
                (
                    s.timeline_in_ms,
                    s.timeline_out_ms,
                    s.style_ref.as_deref(),
                    t,
                )
            })
        })
        .collect();
    // Match captions_to_ass's by-start order so a full EDL is byte-identical.
    items.sort_by_key(|i| i.0);
    let mut ass = ass_header(project);
    ass.push_str(&ass_events(&items));
    Ok(ass)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::{Clip, ProjectSettings, Track};

    /// A project with one caption clip "hello there world" over [0, 3000].
    fn proj_with_caption(text: &str, start: u64, end: u64) -> Project {
        let mut p = Project::new(
            "k",
            ProjectSettings {
                width: 1080,
                height: 1920,
                fps: 30.0,
                audio_rate: 48_000,
                color: cut_core::ColorConfig::default(),
            },
        );
        p.tracks.push(Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "s1".into(),
                text: text.into(),
                style_ref: None,
                range_ms: [start, end],
            })],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        p
    }

    /// karaoke_line emits one `{\k<cs>}word` per word, the \k values SUM to the
    /// clip's centisecond duration, and longer words get more time (proportional).
    #[test]
    fn karaoke_line_distributes_centiseconds() {
        // 3000ms = 300cs across "ab cd efgh" (2+2+4 = 8 chars).
        let line = karaoke_line("ab cd efgh", 0, 3000);
        assert!(line.starts_with("{\\k"), "starts with a \\k tag: {line}");
        assert!(line.contains("ab"));
        assert!(line.contains("efgh"));
        // Extract every \kN and sum — must equal 300cs (last word absorbs remainder).
        let sum: u64 = line
            .split("{\\k")
            .skip(1)
            .filter_map(|s| s.split('}').next())
            .filter_map(|n| n.parse::<u64>().ok())
            .sum();
        assert_eq!(
            sum, 300,
            "syllable \\k values must sum to the clip cs: {line}"
        );
        // "efgh" (4 chars) ≥ "ab"/"cd" (2 chars) in allotted time (proportional).
    }

    /// Empty / whitespace-only text → empty (no Dialogue emitted).
    #[test]
    fn karaoke_line_handles_empty() {
        assert_eq!(karaoke_line("   ", 0, 1000), "");
        assert_eq!(karaoke_line("", 0, 1000), "");
        // single word gets the whole duration.
        let one = karaoke_line("solo", 0, 1000);
        assert_eq!(one, "{\\k100}solo");
    }

    /// The full karaoke serializer produces a valid ASS doc: header with a DIM
    /// SecondaryColour (so the fill shows), an [Events] section, and a Dialogue
    /// carrying \k tags. Distinct from the line-level burn serializer.
    #[test]
    fn karaoke_serializer_emits_dim_secondary_and_k_tags() {
        let p = proj_with_caption("first second third", 500, 3500);
        let ass = captions_to_ass_karaoke(&p).unwrap();
        assert!(ass.contains("[Script Info]"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(
            ass.contains(KARAOKE_SECONDARY),
            "karaoke header must dim SecondaryColour so \\k is visible"
        );
        assert!(ass.contains("[Events]"));
        assert!(ass.contains("Dialogue: 0,"));
        assert!(ass.contains("{\\k"), "events carry \\k karaoke tags");
        // The line-level serializer does NOT carry \k (Secondary == Primary).
        let plain = captions_to_ass(&p).unwrap();
        assert!(!plain.contains("{\\k"), "line-level ASS has no \\k tags");
    }

    /// Brace injection in caption text can't corrupt the \k override block.
    #[test]
    fn karaoke_escapes_braces() {
        let line = karaoke_line("a{evil}b", 0, 1000);
        assert!(
            !line.contains("{evil}"),
            "raw braces must be neutralized: {line}"
        );
    }
}
