//! MLT XML serializer — Shotcut (stretch format). Shape mirrors
//! examples/minimal-shotcut.mlt with ONE deliberate, documented deviation:
//!
//! MLT `in`/`out` are inclusive frame positions.
//! some serializers emit `out = timecode(offset+dur)` — one frame past the last
//! frame; Shotcut tolerates it but every clip plays 1 frame long. Per the
//! We emit the LAST-FRAME time (`offset+dur-1`) on all
//! inclusive `in`/`out` positions. `length` properties stay full durations.
//! The structural tests assert this exact 1-frame relationship vs the example.
//!
//! Element order matters: producers/chains before the playlists that
//! reference them, tractor last.

use crate::error::ExportError;
use crate::model::ExportTimeline;
use crate::quantize::{quantize, Timebase, XItem};
use crate::sources::collect_sources;
use crate::xml::Xml;

/// Frames -> wall-clock timecode "HH:MM:SS.mmm" against the profile fps
/// (note '.' before ms, not ':').
fn tc(frames: i64, tb: &Timebase) -> String {
    let secs = frames.max(0) as f64 * tb.den as f64 / tb.num as f64;
    let h = (secs / 3600.0).floor() as i64;
    let m = ((secs % 3600.0) / 60.0).floor() as i64;
    let s = secs - (h * 3600 + m * 60) as f64;
    format!("{h:02}:{m:02}:{s:06.3}")
}

/// Integer gcd for the profile display-aspect reduction (1920/1080 -> 16/9).
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// `<property name="...">value</property>` shorthand.
fn prop(x: &mut Xml, name: &str, value: &str) {
    x.text_el("property", &[("name", name)], value);
}

/// Render the timeline as Shotcut-flavored MLT XML. An avformat chain carries
/// video AND audio together, so the single video track covers the
/// talking-head A/V case; audio-only timelines use the first audio layer.
pub fn render(tl: &ExportTimeline) -> Result<String, ExportError> {
    let q = quantize(tl)?;
    let sources = collect_sources(tl, &q)?;
    if sources.is_empty() || q.total_frames <= 0 {
        return Err(ExportError::EmptyTimeline);
    }
    let tb = q.tb;
    let mut tracks: Vec<(String, &[XItem])> = Vec::new();
    if q.video.is_empty() {
        if let Some(first_audio) = q.audio.first() {
            tracks.push(("A1".to_string(), first_audio.as_slice()));
            for (i, layer) in q.audio.iter().enumerate().skip(1) {
                tracks.push((format!("A{}", i + 1), layer.as_slice()));
            }
        }
    } else {
        tracks.push(("V1".to_string(), q.video.as_slice()));
        for (i, layer) in q.audio.iter().enumerate() {
            tracks.push((format!("A{}", i + 1), layer.as_slice()));
        }
    }
    let source = |asset_id: &str| sources.iter().find(|s| s.asset_id == asset_id).unwrap();

    let total_tc = tc(q.total_frames, &tb); // duration semantics: no -1
    let total_last = tc(q.total_frames - 1, &tb); // inclusive position: -1
    let g = gcd(tl.settings.width, tl.settings.height);

    let mut x = Xml::new();
    // LC_NUMERIC="C" guards float parsing under non-C locales — keep it.
    x.open(
        "mlt",
        &[
            ("LC_NUMERIC", "C"),
            ("version", "7.33.0"),
            ("title", "Shotcut version 25.10.31"),
            ("producer", "main_bin"),
        ],
    );
    x.leaf(
        "profile",
        &[
            ("description", "automatic"),
            ("width", &tl.settings.width.to_string()),
            ("height", &tl.settings.height.to_string()),
            ("progressive", "1"),
            ("sample_aspect_num", "1"),
            ("sample_aspect_den", "1"),
            ("display_aspect_num", &(tl.settings.width / g).to_string()),
            ("display_aspect_den", &(tl.settings.height / g).to_string()),
            ("frame_rate_num", &tb.num.to_string()),
            ("frame_rate_den", &tb.den.to_string()),
            ("colorspace", "709"),
        ],
    );
    // Shotcut project-bin scaffolding — melt plays the file without these,
    // Shotcut's UI needs them to open it as a project.
    x.open("playlist", &[("id", "main_bin")]);
    prop(&mut x, "xml_retain", "1");
    x.close("playlist");

    // Black background producer spanning the whole timeline (Shotcut shape).
    x.open("producer", &[("id", "bg")]);
    prop(&mut x, "length", &total_tc);
    prop(&mut x, "eof", "pause");
    prop(&mut x, "resource", "#000000");
    prop(&mut x, "mlt_service", "color");
    prop(&mut x, "mlt_image_format", "rgba");
    prop(&mut x, "aspect_ratio", "1");
    x.close("producer");
    x.open("playlist", &[("id", "background")]);
    // The bare "1" text child is part of the proven interchange shape.
    x.text_el(
        "entry",
        &[
            ("producer", "bg"),
            ("in", &tc(0, &tb)),
            ("out", &total_last),
        ],
        "1",
    );
    x.close("playlist");

    // One <chain> per CLIP (not per source file — chains repeat the path).
    let mut filter_n = 0usize;
    let mut chain_n = 0usize;
    let mut chain_refs: Vec<Vec<Option<String>>> = Vec::new();
    for (_track_name, items) in &tracks {
        let mut refs = Vec::new();
        for item in *items {
            if let XItem::Clip(c) = item {
                let chain_id = format!("chain{chain_n}");
                chain_n += 1;
                refs.push(Some(chain_id.clone()));
                let s = source(&c.asset);
                let src_out = c.offset + c.dur;
                x.open(
                    "chain",
                    &[("id", &chain_id), ("out", &tc(src_out - 1, &tb))],
                );
                // length = frames available from the source (duration semantics).
                prop(&mut x, "length", &tc(src_out, &tb));
                prop(&mut x, "eof", "pause");
                prop(&mut x, "resource", &s.path); // absolute path — relative resolves against the XML's dir
                prop(&mut x, "mlt_service", "avformat");
                prop(&mut x, "caption", &s.stem);
                if c.gain_db != 0.0 {
                    // The only format where we export per-clip gain.
                    filter_n += 1;
                    x.open("filter", &[("id", &format!("filter{filter_n}"))]);
                    prop(&mut x, "mlt_service", "volume");
                    prop(&mut x, "level", &format!("{:.2} dB", c.gain_db));
                    x.close("filter");
                }
                x.close("chain");
            } else {
                refs.push(None);
            }
        }
        chain_refs.push(refs);
    }

    // The cut lists: entry in/out are SOURCE-time inclusive positions; timeline
    // position = cumulative playlist order. Gap -> <blank>. Separate audio
    // layers become separate playlists/tracks instead of disappearing.
    for (track_idx, ((track_name, items), refs)) in tracks.iter().zip(chain_refs.iter()).enumerate()
    {
        let playlist_id = format!("playlist{track_idx}");
        x.open("playlist", &[("id", &playlist_id)]);
        if track_idx == 0 && !q.video.is_empty() {
            prop(&mut x, "shotcut:video", "1");
        }
        prop(&mut x, "shotcut:name", track_name);
        for (item, chain_ref) in items.iter().zip(refs.iter()) {
            match item {
                XItem::Clip(c) => x.leaf(
                    "entry",
                    &[
                        ("producer", chain_ref.as_deref().unwrap_or("")),
                        ("in", &tc(c.offset, &tb)),
                        ("out", &tc(c.offset + c.dur - 1, &tb)),
                    ],
                ),
                XItem::Gap { dur, .. } => x.leaf("blank", &[("length", &tc(*dur, &tb))]),
            }
        }
        x.close("playlist");
    }

    x.open(
        "tractor",
        &[
            ("id", "tractor0"),
            ("in", &tc(0, &tb)),
            ("out", &total_last),
        ],
    );
    prop(&mut x, "shotcut", "1");
    prop(&mut x, "shotcut:projectAudioChannels", "2");
    x.leaf("track", &[("producer", "background")]);
    for track_idx in 0..tracks.len() {
        x.leaf("track", &[("producer", &format!("playlist{track_idx}"))]);
    }
    x.close("tractor");
    x.close("mlt");
    Ok(x.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_timeline;
    use serde_json::json;

    #[test]
    fn mlt_emits_audio_layers_alongside_video() {
        let tl = parse_timeline(&json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30, "audio_rate": 48000},
            "assets": {
                "v": {"path": "/m/picture.mp4", "probe": {"duration_ms": 10000}},
                "a": {"path": "/m/music.wav", "probe": {"duration_ms": 10000, "has_video": false, "has_audio": true}}
            },
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "vc", "asset": "v", "src_in_ms": 0, "src_out_ms": 1000}
                ]},
                {"id": "a1", "kind": "audio", "clips": [
                    {"id": "ac", "asset": "a", "src_in_ms": 0, "src_out_ms": 2000}
                ]}
            ]
        }))
        .unwrap();
        let xml = render(&tl).expect("mlt");
        assert!(
            xml.contains("<property name=\"shotcut:name\">V1</property>"),
            "{xml}"
        );
        assert!(
            xml.contains("<property name=\"shotcut:name\">A1</property>"),
            "{xml}"
        );
        assert!(
            xml.contains("<property name=\"resource\">/m/music.wav</property>"),
            "{xml}"
        );
        assert!(xml.contains("track producer=\"playlist1\""), "{xml}");
    }
}
