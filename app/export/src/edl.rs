//! CMX3600 EDL serializer — the classic edit-decision-list interchange that
//! every NLE and color suite reads (Resolve, Premiere, Avid, Final Cut). It
//! consumes the SAME frame-quantized timeline as the XML/MLT serializers, so
//! the cut points line up to the frame across every export format at once.
//!
//! Format choices, made explicit:
//! * NON-DROP FRAME timecodes against the ROUNDED fps — a plain frame counter
//!   (0..fps-1 in the frame field). Exact for non-drop; we never emit
//!   drop-frame (the ';' separated form), so long NTSC timelines drift from
//!   wall-clock by the usual non-drop amount, which is the conventional and
//!   widely-accepted EDL behavior. FCM header states it honestly.
//! * EDL out-points are EXCLUSIVE — one frame past the last frame (the CMX3600
//!   convention), UNLIKE MLT's inclusive in/out. So src_out = offset+dur and
//!   rec_out = start+dur, no -1.
//! * One event per clip: channel `V` for the video track, `A`/`A2`/`A3`… per
//!   audio track in timeline order. Source reel = the file stem sanitised to
//!   ≤8 uppercase alphanumerics (CMX reels are 8 chars); a `* FROM CLIP NAME:`
//!   comment carries the real filename so the target app can relink.
//! * Gaps are left IMPLICIT — the next event's record-in jumps forward, leaving
//!   a hole on the record timeline. That is exactly how an EDL represents black
//!   between events, so no explicit gap construct is emitted.
//!
//! KNOWN FORMAT LIMITS (documented, not bugs): a CMX3600 EDL carries cuts only.
//! Cut's crossfades/transitions, effects, grades, per-clip gains and the
//! caption track are NOT representable and are dropped. The `export.edl` verb
//! surfaces a warning when the project contains any of those.

use crate::error::ExportError;
use crate::model::ExportTimeline;
use crate::quantize::{quantize, Quantized, Timebase, XItem};
use crate::sources::collect_sources;

/// Frames -> "HH:MM:SS:FF" NON-DROP SMPTE against the rounded fps. The frame
/// field counts 0..fps-1; hours wrap at 24 (standard SMPTE).
fn smpte(frames: i64, tb: &Timebase) -> String {
    let fps = tb.rounded().max(1);
    let f = frames.max(0);
    let frame = f % fps;
    let total_secs = f / fps;
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    format!("{h:02}:{m:02}:{s:02}:{frame:02}")
}

/// Sanitise a file stem into a CMX reel id: uppercase ASCII alphanumerics,
/// ≤8 chars, "AX" (the standard "auxiliary" reel) when nothing usable remains.
fn reel(stem: &str) -> String {
    let r: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .take(8)
        .collect();
    if r.is_empty() {
        "AX".to_string()
    } else {
        r
    }
}

/// The basename (filename with extension) of a path, for `* FROM CLIP NAME:`.
fn basename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Audio-track channel label per CMX3600: A, A2, A3, … (1-based; A == A1).
fn audio_chan(idx: usize) -> String {
    if idx == 0 {
        "A".to_string()
    } else {
        format!("A{}", idx + 1)
    }
}

/// Render the timeline as a CMX3600 EDL. `title` names the sequence (TITLE:).
pub fn render(tl: &ExportTimeline, title: &str) -> Result<String, ExportError> {
    let q = quantize(tl)?;
    let sources = collect_sources(tl, &q)?;
    if sources.is_empty() || q.total_frames <= 0 {
        return Err(ExportError::EmptyTimeline);
    }
    let tb = q.tb;
    // (reel-stem, basename) for an asset, with safe fallbacks to the asset id.
    let names = |asset_id: &str| -> (String, String) {
        sources
            .iter()
            .find(|s| s.asset_id == asset_id)
            .map(|s| (s.stem.clone(), basename(&s.path)))
            .unwrap_or_else(|| (asset_id.to_string(), asset_id.to_string()))
    };

    let mut out = String::new();
    let safe_title: String = title.chars().filter(|c| !c.is_control()).take(70).collect();
    out.push_str(&format!(
        "TITLE: {}\n",
        if safe_title.trim().is_empty() {
            "ShellX Cut"
        } else {
            safe_title.trim()
        }
    ));
    out.push_str("FCM: NON-DROP FRAME\n\n");

    let mut event = 1u32;
    // One pass per channel: the video track first, then each audio track.
    // (Quantized keeps the first video track + every audio layer, in order.)
    let mut channels: Vec<(&str, String)> = Vec::new();
    let video_chan = "V".to_string();
    if !q.video.is_empty() {
        channels.push(("video", video_chan));
    }
    for (i, _) in q.audio.iter().enumerate() {
        channels.push(("audio", audio_chan(i)));
    }

    let mut audio_idx = 0usize;
    for (kind, chan) in &channels {
        let items: &[XItem] = if *kind == "video" {
            &q.video
        } else {
            let t = &q.audio[audio_idx];
            audio_idx += 1;
            t
        };
        for c in Quantized::clips(items) {
            let (stem, file) = names(&c.asset);
            let src_in = smpte(c.offset, &tb);
            let src_out = smpte(c.offset + c.dur, &tb); // EXCLUSIVE
            let rec_in = smpte(c.start, &tb);
            let rec_out = smpte(c.start + c.dur, &tb); // EXCLUSIVE
                                                       // Classic CMX3600 column layout: NNN  REEL(8) CHAN(4) C  TCs.
            out.push_str(&format!(
                "{event:03}  {reel:<8} {chan:<4} C        {src_in} {src_out} {rec_in} {rec_out}\n",
                reel = reel(&stem),
                chan = chan,
            ));
            out.push_str(&format!("* FROM CLIP NAME: {file}\n"));
            event += 1;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_timeline;
    use serde_json::json;

    /// Two video clips + one audio clip at 30fps; assert header, event count,
    /// channels, exclusive out-points and the running record timecodes.
    fn sample() -> ExportTimeline {
        parse_timeline(&json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30, "audio_rate": 48000},
            "assets": {"a1": {"path": "/media/Interview_Take1.mp4"}},
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "c1", "asset": "a1", "src_in_ms": 0,    "src_out_ms": 2000},
                    {"id": "c2", "asset": "a1", "src_in_ms": 5000, "src_out_ms": 6000}
                ]},
                {"id": "a1t", "kind": "audio", "clips": [
                    {"id": "c3", "asset": "a1", "src_in_ms": 0, "src_out_ms": 3000}
                ]}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn header_events_channels_and_exclusive_outpoints() {
        let edl = render(&sample(), "My Cut").unwrap();
        assert!(
            edl.starts_with("TITLE: My Cut\nFCM: NON-DROP FRAME\n\n"),
            "{edl}"
        );
        // 2 video + 1 audio = 3 events, numbered 001..003.
        assert!(edl.contains("\n001  "), "{edl}");
        assert!(edl.contains("\n002  "), "{edl}");
        assert!(edl.contains("\n003  "), "{edl}");
        assert!(!edl.contains("\n004  "));
        // Reel = sanitised stem (≤8 uppercase alnum) of Interview_Take1.
        assert!(edl.contains("INTERVIE"), "reel from stem: {edl}");
        // Video on V, audio on A.
        assert!(edl.contains(" V    C "), "video channel: {edl}");
        assert!(edl.contains(" A    C "), "audio channel: {edl}");
        // First clip: src 0..2000ms = 0..60f, rec 0..60f (exclusive out = 2s).
        assert!(
            edl.contains("00:00:00:00 00:00:02:00 00:00:00:00 00:00:02:00"),
            "{edl}"
        );
        // Second video clip records right after the first (running sum): rec_in
        // = 60f = 00:00:02:00, and its source is 5000..6000ms = 00:00:05:00..06:00.
        assert!(
            edl.contains("00:00:05:00 00:00:06:00 00:00:02:00 00:00:03:00"),
            "{edl}"
        );
        // Relink comment carries the real filename (with extension).
        assert!(
            edl.contains("* FROM CLIP NAME: Interview_Take1.mp4"),
            "{edl}"
        );
    }

    #[test]
    fn empty_timeline_is_rejected() {
        let tl = parse_timeline(&json!({
            "settings": {"fps": 30},
            "assets": {},
            "tracks": []
        }))
        .unwrap();
        assert!(matches!(render(&tl, "x"), Err(ExportError::EmptyTimeline)));
    }
}
