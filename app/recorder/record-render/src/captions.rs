//! captions.rs — caption burn-in from ShellX Cut's word-span transcript.
//!
//! Consumes Cut's transcript schema — `{words:[{word,start_ms,end_ms,...}]}` from
//! `receipts/<asset>.words.json`, produced by Parakeet-TDT / onnx-asr (NOT whisper;
//! schema is engine-agnostic). We do NO transcription here — Cut owns STT. We just
//! group words into short on-screen lines and look up the active one per frame.
//! Best-effort: any read/parse error yields zero lines (captions never crash a render).

use serde::Deserialize;
use std::io::Read;

const MAX_TRANSCRIPT_JSON_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Deserialize)]
struct WordSpan {
    word: String,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Deserialize)]
struct Transcript {
    words: Vec<WordSpan>,
}

/// One on-screen caption line with its time span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Load a word-span transcript and group it into caption lines, breaking on
/// `max_chars` or a long pause (> 900 ms). Returns [] on any error.
pub fn load_lines(path: &str, max_chars: usize) -> Vec<CaptionLine> {
    load_lines_with_limit(path, max_chars, MAX_TRANSCRIPT_JSON_BYTES)
}

fn load_lines_with_limit(path: &str, max_chars: usize, max_bytes: u64) -> Vec<CaptionLine> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("warning: captions requested but transcript '{path}' could not be read ({e}) — rendering without captions");
            return Vec::new();
        }
    };
    let mut bytes = Vec::new();
    if let Err(e) = file.take(max_bytes + 1).read_to_end(&mut bytes) {
        // Best-effort: don't crash the render, but DON'T fail silently either —
        // captions were requested, so surface that they were skipped.
        eprintln!("warning: captions requested but transcript '{path}' could not be read ({e}) — rendering without captions");
        return Vec::new();
    }
    if bytes.len() as u64 > max_bytes {
        eprintln!(
            "warning: captions transcript '{path}' exceeds the {} MiB limit — rendering without captions",
            max_bytes / (1024 * 1024)
        );
        return Vec::new();
    }
    let t = match serde_json::from_slice::<Transcript>(&bytes) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warning: captions transcript '{path}' is not valid word-span JSON ({e}) — rendering without captions");
            return Vec::new();
        }
    };
    group(&t.words, max_chars)
}

fn group(words: &[WordSpan], max_chars: usize) -> Vec<CaptionLine> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut start = 0u64;
    let mut last_end = 0u64;
    for w in words {
        let gap = w.start_ms.saturating_sub(last_end);
        let would = if cur.is_empty() {
            w.word.len()
        } else {
            cur.len() + 1 + w.word.len()
        };
        if !cur.is_empty() && (would > max_chars || gap > 900) {
            lines.push(CaptionLine {
                start_ms: start,
                end_ms: last_end,
                text: std::mem::take(&mut cur),
            });
        }
        if cur.is_empty() {
            start = w.start_ms;
        } else {
            cur.push(' ');
        }
        cur.push_str(&w.word);
        last_end = w.end_ms;
    }
    if !cur.is_empty() {
        lines.push(CaptionLine {
            start_ms: start,
            end_ms: last_end,
            text: cur,
        });
    }
    lines
}

/// The caption line active at `t_ms`, if any.
pub fn active(lines: &[CaptionLine], t_ms: u64) -> Option<&str> {
    lines
        .iter()
        .find(|l| t_ms >= l.start_ms && t_ms < l.end_ms)
        .map(|l| l.text.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(word: &str, s: u64, e: u64) -> WordSpan {
        WordSpan {
            word: word.into(),
            start_ms: s,
            end_ms: e,
        }
    }

    #[test]
    fn groups_by_chars_and_pause() {
        let words = vec![
            w("create", 0, 300),
            w("polished", 350, 700),
            w("demos", 750, 1000),
            // long pause → new line
            w("without", 3000, 3300),
            w("editing", 3350, 3700),
        ];
        let lines = group(&words, 24);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "create polished demos");
        assert_eq!(lines[1].text, "without editing");
        assert_eq!(active(&lines, 500), Some("create polished demos"));
        assert_eq!(active(&lines, 1500), None); // in the pause
        assert_eq!(active(&lines, 3400), Some("without editing"));
    }

    #[test]
    fn bad_path_is_empty_not_fatal() {
        assert!(load_lines("/no/such/file.json", 30).is_empty());
    }

    #[test]
    fn oversized_transcript_is_empty_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("words.json");
        std::fs::write(&path, b"12345").unwrap();

        assert!(load_lines_with_limit(path.to_str().unwrap(), 30, 4).is_empty());
    }
}
