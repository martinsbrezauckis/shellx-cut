//! FCPXML 1.11 serializer — Final Cut Pro (`fcpxml`) and DaVinci Resolve 17+
//! (`resolve`), one code path with a format flag. Shape mirrors
//! examples/minimal.fcpxml.
//!
//! The two interchange constraints this file exists to honor:
//! - `offset` = TIMELINE position, `start` = SOURCE in-point — inverted from
//!   intuition. Both flow from the quantized frames via [`fraction`], which
//!   guarantees every time value is an integer multiple of frameDuration.
//! - asset `duration` = full SOURCE duration, never the edited timeline length.

use crate::error::ExportError;
use crate::model::{file_uri, ExportTimeline};
use crate::quantize::{quantize, Quantized, Timebase, XItem};
use crate::sources::{collect_sources, SourceInfo};
use crate::xml::Xml;

/// Rational-seconds time string: frames -> "N/Ds" (unreduced, exactly what
/// accepted interchange form), 0 -> "0s". Deriving every time value
/// through this function is what guarantees frameDuration alignment.
fn fraction(frames: i64, tb: &Timebase) -> Result<String, ExportError> {
    if frames == 0 {
        Ok("0s".to_string())
    } else {
        let numerator = frames.checked_mul(tb.den).ok_or_else(|| {
            ExportError::BadInput(format!(
                "frame value {frames} overflows FCPXML fraction denominator {}",
                tb.den
            ))
        })?;
        Ok(format!("{numerator}/{}s", tb.num))
    }
}

fn audio_rate_attr(rate: u32) -> String {
    match rate {
        44_100 => "44.1k".to_string(),
        48_000 => "48k".to_string(),
        88_200 => "88.2k".to_string(),
        96_000 => "96k".to_string(),
        176_400 => "176.4k".to_string(),
        192_000 => "192k".to_string(),
        r if r % 1000 == 0 => format!("{}k", r / 1000),
        r if r % 100 == 0 => format!("{:.1}k", r as f64 / 1000.0),
        r => format!("{:.2}k", r as f64 / 1000.0),
    }
}

/// Render the timeline as FCPXML 1.11. `resolve` enables the Resolve deltas
/// Export mapping contract: hard error on audio stream > 0, extra audio layers as
/// duplicate spine passes. With the current single-layer timeline the two
/// variants emit identical documents.
pub fn render(tl: &ExportTimeline, resolve: bool) -> Result<String, ExportError> {
    let q = quantize(tl)?;
    let sources = collect_sources(tl, &q)?;
    if sources.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }

    // Resolve cannot address audio stream indexes > 0 inside an asset —
    // refuse loudly instead of emitting a file Resolve mis-reads.
    if resolve {
        for layer in &q.audio {
            for c in Quantized::clips(layer) {
                if c.stream > 0 {
                    return Err(ExportError::ResolveStream {
                        clip_id: c.clip_id.clone(),
                        stream: c.stream,
                    });
                }
            }
        }
    }

    let tb = q.tb;
    // Spine content: the video track's clips; audio-only timeline falls back
    // to the first audio layer (an asset-clip carries both A and V).
    let main_items: &[XItem] = if q.video.is_empty() {
        q.audio.first().map(|v| v.as_slice()).unwrap_or(&[])
    } else {
        &q.video
    };

    let extra_audio_layers: &[Vec<XItem>] = if q.video.is_empty() {
        q.audio.get(1..).unwrap_or(&[])
    } else {
        q.audio.as_slice()
    };

    // Every standalone audio layer must be emitted. Resolve stacks duplicate
    // offsets onto separate tracks (reversed track order);
    // plain FCP also keeps the audio clips as their own offset asset-clips.
    let mut passes: Vec<(Option<String>, &[XItem])> = Vec::new();
    if resolve {
        for (i, layer) in extra_audio_layers.iter().rev().enumerate() {
            passes.push((Some(format!("-{}", i + 1)), layer.as_slice()));
        }
    } else {
        for (i, layer) in extra_audio_layers.iter().enumerate() {
            passes.push((Some(format!("-{}", i + 1)), layer.as_slice()));
        }
    }
    passes.push((None, main_items));

    // r-id allocation: source i -> format r{2i+1}, asset r{2i+2}.
    let rid_format = |i: usize| format!("r{}", 2 * i + 1);
    let rid_asset = |i: usize| format!("r{}", 2 * i + 2);
    let frame_duration = fraction(1, &tb)?;
    let width = tl.settings.width.to_string();
    let height = tl.settings.height.to_string();

    let mut x = Xml::new();
    x.open("fcpxml", &[("version", "1.11")]);
    x.open("resources", &[]);
    for (i, s) in sources.iter().enumerate() {
        // <format> per source, timeline resolution + frameDuration.
        x.leaf(
            "format",
            &[
                ("id", &rid_format(i)),
                ("name", "FFVideoFormatRateUndefined"),
                ("frameDuration", &frame_duration),
                ("width", &width),
                ("height", &height),
                ("colorSpace", &s.color_space),
            ],
        );
        // <asset>: duration = FULL SOURCE duration; start = embedded SMPTE.
        let duration = fraction(s.duration_frames, &tb)?;
        let start = fraction(s.smpte_start, &tb)?;
        x.open(
            "asset",
            &[
                ("id", &rid_asset(i)),
                ("name", &s.stem),
                ("start", &start),
                ("duration", &duration),
                ("hasVideo", if s.has_video { "1" } else { "0" }),
                ("hasAudio", if s.has_audio { "1" } else { "0" }),
                ("format", &rid_format(i)),
                ("audioSources", "1"),
                ("audioChannels", &s.audio_channels.to_string()),
            ],
        );
        // Relink path: percent-encoded file:// URI.
        x.leaf(
            "media-rep",
            &[("kind", "original-media"), ("src", &file_uri(&s.path))],
        );
        x.close("asset");
    }
    x.close("resources");

    x.open("library", &[]);
    x.open("event", &[("name", "ShellX Cut Media Group")]);
    x.open("project", &[("name", &sources[0].stem)]);
    // audioLayout: mono is NOT accepted — ≤2ch -> stereo, >2 -> surround.
    let max_ch = sources.iter().map(|s| s.audio_channels).max().unwrap_or(2);
    let audio_layout = if max_ch > 2 { "surround" } else { "stereo" };
    let audio_rate = audio_rate_attr(tl.settings.audio_rate);
    x.open(
        "sequence",
        &[
            ("format", "r1"),
            ("tcStart", "0s"),
            ("tcFormat", "NDF"),
            ("audioLayout", audio_layout),
            ("audioRate", &audio_rate),
        ],
    );
    x.open("spine", &[]);
    let write_item = |x: &mut Xml, item: &XItem, lane: Option<&str>| -> Result<(), ExportError> {
        match item {
            XItem::Gap { start, dur } => {
                let offset = fraction(*start, &tb)?;
                let duration = fraction(*dur, &tb)?;
                let mut attrs = vec![("offset", offset), ("duration", duration)];
                if let Some(lane) = lane {
                    attrs.push(("lane", lane.to_string()));
                }
                let refs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                x.leaf("gap", &refs);
            }
            XItem::Clip(c) => {
                let i = source_index(&sources, &c.asset, &c.clip_id)?;
                let s: &SourceInfo = &sources[i];
                let offset = fraction(c.start, &tb)?;
                let duration = fraction(c.dur, &tb)?;
                let source_start = c.offset.checked_add(s.smpte_start).ok_or_else(|| {
                    ExportError::BadInput(format!(
                        "clip '{}' source start overflows FCPXML frame address",
                        c.clip_id
                    ))
                })?;
                let start = fraction(source_start, &tb)?;
                // THE inversion: offset=timeline position, start=source
                // in-point (+ source SMPTE start).
                let mut attrs = vec![
                    ("name", s.stem.clone()),
                    ("ref", rid_asset(i)),
                    ("offset", offset),
                    ("duration", duration),
                    ("start", start),
                    ("tcFormat", "NDF".to_string()),
                ];
                if let Some(lane) = lane {
                    attrs.push(("lane", lane.to_string()));
                }
                let refs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                x.leaf("asset-clip", &refs);
            }
        }
        Ok(())
    };
    for (lane, items) in passes {
        for item in items {
            write_item(&mut x, item, lane.as_deref())?;
        }
    }
    x.close("spine");
    x.close("sequence");
    x.close("project");
    x.close("event");
    x.close("library");
    x.close("fcpxml");
    Ok(x.finish())
}

fn source_index(
    sources: &[SourceInfo],
    asset_id: &str,
    clip_id: &str,
) -> Result<usize, ExportError> {
    sources
        .iter()
        .position(|s| s.asset_id == asset_id)
        .ok_or_else(|| ExportError::MissingAsset {
            clip_id: clip_id.to_string(),
            asset: asset_id.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_timeline;
    use serde_json::json;

    fn av_plus_voice_timeline() -> ExportTimeline {
        parse_timeline(&json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30, "audio_rate": 48000},
            "assets": {
                "v": {"path": "/m/picture.mp4", "probe": {"duration_ms": 10000, "has_video": true, "has_audio": true}},
                "voice": {"path": "/m/voice.wav", "probe": {"duration_ms": 5000, "has_video": false, "has_audio": true}}
            },
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "vc", "asset": "v", "src_in_ms": 0, "src_out_ms": 2000}
                ]},
                {"id": "a1", "kind": "audio", "clips": [
                    {"id": "ac", "asset": "voice", "src_in_ms": 0, "src_out_ms": 5000}
                ]}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn fcpxml_keeps_standalone_audio_tracks_alongside_video() {
        let plain = render(&av_plus_voice_timeline(), false).expect("plain fcpxml");
        assert!(plain.contains("name=\"picture\""), "{plain}");
        assert!(plain.matches("name=\"voice\"").count() >= 2, "{plain}");
        assert!(
            plain.contains("duration=\"150/30s\""),
            "audio clip duration should be emitted, not just an orphan resource: {plain}"
        );
        assert!(
            plain.contains("name=\"voice\"")
                && plain.contains("lane=\"-1\"")
                && plain.contains("offset=\"0s\""),
            "standalone audio should be connected on its own lane: {plain}"
        );

        let resolve = render(&av_plus_voice_timeline(), true).expect("resolve fcpxml");
        assert!(resolve.matches("name=\"voice\"").count() >= 2, "{resolve}");
        assert!(resolve.contains("duration=\"150/30s\""), "{resolve}");
        assert!(
            resolve.contains("name=\"voice\"") && resolve.contains("lane=\"-1\""),
            "Resolve standalone audio should not be a flat unlabeled spine sibling: {resolve}"
        );
    }

    #[test]
    fn fcpxml_audio_rate_reflects_project_sample_rate() {
        let mut tl = av_plus_voice_timeline();
        tl.settings.audio_rate = 96_000;
        let xml = render(&tl, false).expect("fcpxml");
        assert!(xml.contains("audioRate=\"96k\""), "{xml}");

        tl.settings.audio_rate = 32_000;
        let xml = render(&tl, false).expect("fcpxml");
        assert!(xml.contains("audioRate=\"32k\""), "{xml}");
    }

    #[test]
    fn source_lookup_returns_error_instead_of_panicking() {
        let sources = vec![SourceInfo {
            asset_id: "known".to_string(),
            path: "/m/known.mp4".to_string(),
            stem: "known".to_string(),
            has_video: true,
            has_audio: true,
            audio_channels: 2,
            duration_frames: 30,
            smpte_start: 0,
            color_space: "1-1-1 (Rec. 709)".to_string(),
        }];
        let err = source_index(&sources, "missing", "c1").unwrap_err();
        assert!(
            matches!(err, ExportError::MissingAsset { .. }),
            "missing source lookup should be an export error: {err:?}"
        );
    }
}
