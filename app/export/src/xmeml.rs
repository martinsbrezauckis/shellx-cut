//! FCP7 XML (`xmeml` v5) serializer — Premiere Pro dialect.
//! Shape mirrors examples/minimal-premiere.xml. Import-only target: Premiere
//! round-trips this format lossily by design.
//!
//! The interchange constraints this file honors:
//! - "Exploded" stereo: one PAIR of `<track>`s per stereo audio layer with the
//!   exploded attrs; MONO IS NEVER EXPLODED (a mono source's second exploded
//!   track would read a non-existent channel and play silent on one side).
//! - Emit-file-def-once: the full `<file>` body appears only under the FIRST
//!   clipitem in document order referencing that source; later refs are bare
//!   `<file id="file-K"></file>`.
//! - The blank `<duration></duration>` inside the file def stays — DaVinci
//!   Resolve requires the tag's presence; Premiere ignores it.
//! - Deterministic clipitem ids drive the A/V link wiring: video 1..V, then
//!   one block of ids per audio plan track.

use std::collections::HashSet;

use crate::error::ExportError;
use crate::model::ExportTimeline;
use crate::quantize::{quantize, Quantized, Timebase, XClip};
use crate::sources::{collect_sources, SourceInfo};
use crate::xml::Xml;

/// xmeml rate encoding: integer timebase + NTSC flag.
/// 30000/1001 -> (30, TRUE); integer fps -> (fps, FALSE).
fn xmeml_rate(tb: &Timebase) -> (i64, bool) {
    if tb.is_ntsc() {
        ((tb.num as f64 / 1000.0).ceil() as i64, true)
    } else {
        (tb.num, false)
    }
}

/// One emitted audio `<track>` (an "exploded" channel of a timeline layer).
struct PlanTrack<'a> {
    clips: Vec<&'a XClip>,
    exploded_index: u32,
    exploded_total: u32,
    premiere_track_type: &'static str,   // "Stereo" | "Mono"
    premiere_channel_type: &'static str, // "stereo" | "mono"
    /// `<outputchannelindex>` value; only emitted when video exists.
    output_channel: u32,
    /// 1-based index into the source's flattened channel list.
    sourcetrack_index: u32,
    /// clipitem id of this track's first clipitem (link arithmetic).
    first_clip_id: usize,
}

/// Render the timeline as xmeml v5, Premiere dialect.
pub fn render(tl: &ExportTimeline) -> Result<String, ExportError> {
    let q = quantize(tl)?;
    let sources = collect_sources(tl, &q)?;
    if sources.is_empty() {
        return Err(ExportError::EmptyTimeline);
    }
    let (timebase, ntsc) = xmeml_rate(&q.tb);
    let tb_s = timebase.to_string();
    let ntsc_s = if ntsc { "TRUE" } else { "FALSE" };

    let video_layers: Vec<Vec<&XClip>> = q
        .video_tracks
        .iter()
        .map(|track| Quantized::clips(&track.items))
        .collect();
    let all_video_clip_count: usize = video_layers.iter().map(Vec::len).sum();
    let has_video = all_video_clip_count > 0;

    // Build the audio plan: stereo (≥2ch) layer -> 2 exploded tracks,
    // mono -> 1 track. channelOffset accumulates source channels across
    // layers so sourcetrack indexes stay unique.
    let src_channels = |asset_id: &str| -> u32 {
        sources
            .iter()
            .find(|s| s.asset_id == asset_id)
            .map(|s| s.audio_channels)
            .unwrap_or(2)
    };
    let mut plan: Vec<PlanTrack> = Vec::new();
    let mut next_id = all_video_clip_count + 1; // video ids first; audio blocks follow
    let mut channel_offset: u32 = 0;
    for layer in &q.audio {
        let clips = Quantized::clips(layer);
        if clips.is_empty() {
            continue;
        }
        let ch = src_channels(&clips[0].asset);
        let exploded: u32 = if ch >= 2 { 2 } else { 1 };
        for e in 0..exploded {
            plan.push(PlanTrack {
                clips: clips.clone(),
                exploded_index: e,
                exploded_total: exploded,
                premiere_track_type: if exploded == 2 { "Stereo" } else { "Mono" },
                premiere_channel_type: if exploded == 2 { "stereo" } else { "mono" },
                output_channel: e + 1,
                sourcetrack_index: channel_offset + e + 1,
                first_clip_id: next_id,
            });
            next_id += clips.len();
        }
        channel_offset += ch;
    }

    let file_id = |asset_id: &str| -> String {
        let k = sources.iter().position(|s| s.asset_id == asset_id).unwrap() + 1;
        format!("file-{k}")
    };
    let mut emitted_files: HashSet<String> = HashSet::new();

    let project_name = sources[0].stem.clone();
    let total = q.total_frames.to_string();

    let mut x = Xml::new();
    x.open("xmeml", &[("version", "5")]);
    // explodedTracks attr present for the Premiere dialect.
    x.open("sequence", &[("explodedTracks", "true")]);
    x.text_el("name", &[], &project_name);
    x.text_el("duration", &[], &total);
    rate_block(&mut x, &tb_s, ntsc_s);
    x.open("media", &[]);

    // ---- video section (omitted entirely for audio-only timelines) ----
    if has_video {
        x.open("video", &[]);
        x.open("format", &[]);
        x.open("samplecharacteristics", &[]);
        x.text_el("width", &[], &tl.settings.width.to_string());
        x.text_el("height", &[], &tl.settings.height.to_string());
        x.text_el("pixelaspectratio", &[], "square");
        rate_block(&mut x, &tb_s, ntsc_s);
        x.close("samplecharacteristics");
        x.close("format");
        let mut video_id = 1usize;
        for (track_i, clips) in video_layers.iter().enumerate() {
            if clips.is_empty() {
                continue;
            }
            x.open("track", &[]);
            for (j, c) in clips.iter().enumerate() {
                let id = format!("clipitem-{video_id}");
                let source = source_of(&sources, &c.asset, &c.clip_id)?;
                x.open("clipitem", &[("id", &id)]);
                clipitem_times(&mut x, &source.stem, c);
                emit_file(
                    &mut x,
                    tl,
                    &q.tb,
                    &tb_s,
                    ntsc_s,
                    source,
                    &file_id(&c.asset),
                    &mut emitted_files,
                );
                x.text_el("compositemode", &[], "normal");
                // Link wiring: primary video gets the A/V links. Overlay
                // video tracks keep their own video self-link but have no sibling
                // audio link to avoid claiming unrelated clips are coupled.
                link(&mut x, &id, "video", track_i + 1, j + 1);
                if track_i == 0 {
                    for (k, p) in plan.iter().enumerate() {
                        if j < p.clips.len() {
                            link(
                                &mut x,
                                &format!("clipitem-{}", p.first_clip_id + j),
                                "audio",
                                k + 1,
                                j + 1,
                            );
                        }
                    }
                }
                x.close("clipitem");
                video_id += 1;
            }
            x.close("track");
        }
        x.close("video");
    }

    // ---- audio section ----
    x.open("audio", &[]);
    x.text_el("numOutputChannels", &[], "2");
    x.open("format", &[]);
    x.open("samplecharacteristics", &[]);
    x.text_el("depth", &[], "16");
    x.text_el("samplerate", &[], &tl.settings.audio_rate.to_string());
    x.close("samplecharacteristics");
    x.close("format");
    for p in &plan {
        // Exploded attrs only meaningful with the Premiere dialect; mono
        // tracks carry totalExplodedTrackCount="1" (never a second track).
        let cur = p.exploded_index.to_string();
        let tot = p.exploded_total.to_string();
        x.open(
            "track",
            &[
                ("currentExplodedTrackIndex", &cur),
                ("totalExplodedTrackCount", &tot),
                ("premiereTrackType", p.premiere_track_type),
            ],
        );
        if has_video {
            x.text_el("outputchannelindex", &[], &p.output_channel.to_string());
        }
        for (j, c) in p.clips.iter().enumerate() {
            let id = format!("clipitem-{}", p.first_clip_id + j);
            let source = source_of(&sources, &c.asset, &c.clip_id)?;
            x.open(
                "clipitem",
                &[
                    ("id", &id),
                    ("premiereChannelType", p.premiere_channel_type),
                ],
            );
            clipitem_times(&mut x, &source.stem, c);
            emit_file(
                &mut x,
                tl,
                &q.tb,
                &tb_s,
                ntsc_s,
                source,
                &file_id(&c.asset),
                &mut emitted_files,
            );
            x.open("sourcetrack", &[]);
            x.text_el("mediatype", &[], "audio");
            x.text_el("trackindex", &[], &p.sourcetrack_index.to_string());
            x.close("sourcetrack");
            // Clip color, cosmetic — kept for byte-compat with the proven shape.
            x.open("labels", &[]);
            x.text_el("label2", &[], "Iris");
            x.close("labels");
            x.close("clipitem");
        }
        x.close("track");
    }
    x.close("audio");

    x.close("media");
    x.close("sequence");
    x.close("xmeml");
    Ok(x.finish())
}

/// `<rate>` block — emitted identically at every site; mismatched rates are
/// the classic xmeml import failure.
fn rate_block(x: &mut Xml, timebase: &str, ntsc: &str) {
    x.open("rate", &[]);
    x.text_el("timebase", &[], timebase);
    x.text_el("ntsc", &[], ntsc);
    x.close("rate");
}

/// Common clipitem header: name/enabled + end-exclusive frame ranges —
/// start/end = timeline placement, in/out = source range.
fn clipitem_times(x: &mut Xml, name: &str, c: &XClip) {
    x.text_el("name", &[], name);
    x.text_el("enabled", &[], "TRUE");
    x.text_el("start", &[], &c.start.to_string());
    x.text_el("end", &[], &(c.start + c.dur).to_string());
    x.text_el("in", &[], &c.offset.to_string());
    x.text_el("out", &[], &(c.offset + c.dur).to_string());
}

/// `<link>` element (order: linkclipref, mediatype, trackindex, clipindex).
fn link(x: &mut Xml, clipref: &str, mediatype: &str, trackindex: usize, clipindex: usize) {
    x.open("link", &[]);
    x.text_el("linkclipref", &[], clipref);
    x.text_el("mediatype", &[], mediatype);
    x.text_el("trackindex", &[], &trackindex.to_string());
    x.text_el("clipindex", &[], &clipindex.to_string());
    x.close("link");
}

/// `<file>`: full definition the first time a source is referenced in
/// document order, bare `<file id></file>` afterwards (emit-once rule).
fn emit_file(
    x: &mut Xml,
    tl: &ExportTimeline,
    tb: &Timebase,
    tb_s: &str,
    ntsc_s: &str,
    s: &SourceInfo,
    fid: &str,
    emitted: &mut HashSet<String>,
) {
    if !emitted.insert(s.asset_id.clone()) {
        x.text_el("file", &[("id", fid)], "");
        return;
    }
    x.open("file", &[("id", fid)]);
    x.text_el("name", &[], &s.stem);
    // Bare absolute path, forward slashes — NOT a file:// URI; this is
    // the field-proven relink shape for both Premiere and Resolve.
    x.text_el("pathurl", &[], &s.path);
    x.open("timecode", &[]);
    x.text_el("string", &[], &smpte_string(s.smpte_start, tb.rounded()));
    x.text_el("displayformat", &[], "NDF");
    rate_block(x, tb_s, ntsc_s);
    x.close("timecode");
    rate_block(x, tb_s, ntsc_s);
    // Blank duration: Resolve requires the tag's PRESENCE — keep empty.
    x.text_el("duration", &[], "");
    x.open("media", &[]);
    if s.has_video {
        x.open("video", &[]);
        x.open("samplecharacteristics", &[]);
        rate_block(x, tb_s, ntsc_s);
        x.text_el("width", &[], &tl.settings.width.to_string());
        x.text_el("height", &[], &tl.settings.height.to_string());
        x.text_el("pixelaspectratio", &[], "square");
        x.close("samplecharacteristics");
        x.close("video");
    }
    if s.has_audio {
        x.open("audio", &[]);
        x.open("samplecharacteristics", &[]);
        x.text_el("depth", &[], "16");
        x.text_el("samplerate", &[], &tl.settings.audio_rate.to_string());
        x.close("samplecharacteristics");
        x.text_el("channelcount", &[], &s.audio_channels.to_string());
        x.close("audio");
    }
    x.close("media");
    x.close("file");
}

/// Look up a clip's source facts (quantize already validated asset ids).
fn source_of<'a>(
    sources: &'a [SourceInfo],
    asset_id: &str,
    clip_id: &str,
) -> Result<&'a SourceInfo, ExportError> {
    sources
        .iter()
        .find(|s| s.asset_id == asset_id)
        .ok_or_else(|| ExportError::MissingAsset {
            clip_id: clip_id.to_string(),
            asset: asset_id.to_string(),
        })
}

/// Frames -> SMPTE "HH:MM:SS:FF" at the integer timebase (file timecode block).
fn smpte_string(frames: i64, fps: i64) -> String {
    let fps = fps.max(1);
    let ff = frames % fps;
    let secs = frames / fps;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60,
        ff
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_timeline;
    use serde_json::json;

    #[test]
    fn source_lookup_reports_export_error_instead_of_panicking() {
        let err = source_of(&[], "missing_asset", "clip_1").unwrap_err();
        assert!(matches!(
            err,
            ExportError::MissingAsset {
                clip_id,
                asset
            } if clip_id == "clip_1" && asset == "missing_asset"
        ));
    }

    #[test]
    fn premiere_xmeml_emits_additional_video_tracks() {
        let tl = parse_timeline(&json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30, "audio_rate": 48000},
            "assets": {
                "base": {"path": "/m/base.mp4", "probe": {"duration_ms": 10000}},
                "overlay": {"path": "/m/overlay.mp4", "probe": {"duration_ms": 10000}}
            },
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "base_c", "asset": "base", "src_in_ms": 0, "src_out_ms": 1000}
                ]},
                {"id": "v2", "kind": "video", "clips": [
                    {"id": "overlay_c", "asset": "overlay", "src_in_ms": 0, "src_out_ms": 500}
                ]}
            ]
        }))
        .unwrap();
        let xml = render(&tl).expect("xmeml");
        assert!(xml.contains("<name>base</name>"), "{xml}");
        assert!(xml.contains("<name>overlay</name>"), "{xml}");
        assert!(
            xml.matches("<track>").count() >= 2,
            "both video tracks should be emitted: {xml}"
        );
    }
}
