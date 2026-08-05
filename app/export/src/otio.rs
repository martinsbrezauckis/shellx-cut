//! otio.rs — OpenTimelineIO (.otio) interchange.
//!
//! Role: serialize Cut's timeline to the industry-standard OTIO JSON (ASWF,
//! Apache-2.0) so cuts round-trip with Resolve / Premiere / FCP, and parse it
//! back. OTIO is JSON with a small, STABLE schema, so — like this crate's
//! hand-rolled FCPXML/MLT serializers — we map it directly (no heavy binding).
//!
//! Mapping (Cut ↔ OTIO):
//!   - Timeline.1 → { tracks: Stack.1 { children: [Track.1] } }, rate = fps.
//!   - a video/audio TRACK → Track.1 {kind: "Video"|"Audio"}. Caption tracks are
//!     dropped (OTIO has no native caption track — same as the XML export).
//!   - a MEDIA clip → Clip.1 with source_range = TimeRange(src_in, src_out−src_in)
//!     and media_reference = ExternalReference{target_url=file://…, available_range
//!     = the source's full length when probed}.
//!   - a GAP → Gap.1 with source_range = TimeRange(0, duration).
//!   Times are RationalTime{rate=fps, value=FRAMES} (value = round(ms/1000·fps)).
//!
//! [`parse_otio`] reads the structure back to a flat [`OtioTrack`] list — the basis
//! of the lossless round-trip test (export → parse → equal) and of a future
//! `import.otio` verb. Caller: dispatch.rs (`export.otio`).

use crate::error::ExportError;
use crate::model::{file_stem, file_uri, parse_timeline};
use crate::quantize::{quantize, QTrack, Timebase, XItem};
use serde_json::{json, Value};

/// OTIO frames → ms at `fps` (the inverse, for parse).
fn frames_to_ms(value: f64, rate: f64) -> u64 {
    if rate <= 0.0 {
        return 0;
    }
    (value / rate * 1000.0).round() as u64
}

/// A RationalTime.1 node.
fn rtime(value: i64, rate: f64) -> Value {
    json!({"OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": value})
}

/// A TimeRange.1 node (start + duration in frames).
fn trange(start: i64, dur: i64, rate: f64) -> Value {
    json!({
        "OTIO_SCHEMA": "TimeRange.1",
        "start_time": rtime(start, rate),
        "duration": rtime(dur, rate),
    })
}

/// Serialize a Cut timeline snapshot (the `project.json` Value) to OTIO JSON.
/// `name` titles the timeline. Caption tracks are skipped (returned in the warning
/// path by the caller, mirroring the XML export).
pub fn export_otio(timeline: &Value, name: &str) -> Result<String, ExportError> {
    let tl = parse_timeline(timeline)?;
    let q = quantize(&tl)?;
    let fps = q.tb.fps_f64();

    let mut track_nodes: Vec<Value> = Vec::new();
    for tr in &q.video_tracks {
        track_nodes.push(otio_track(&tl, tr, &q.tb, "Video")?);
    }
    for tr in &q.audio_tracks {
        track_nodes.push(otio_track(&tl, tr, &q.tb, "Audio")?);
    }
    let has_media = track_nodes.iter().any(|track| {
        track
            .get("children")
            .and_then(|v| v.as_array())
            .map(|children| {
                children
                    .iter()
                    .any(|ch| ch.get("OTIO_SCHEMA").and_then(|v| v.as_str()) == Some("Clip.1"))
            })
            .unwrap_or(false)
    });
    if !has_media {
        return Err(ExportError::EmptyTimeline);
    }

    let doc = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": name,
        "global_start_time": rtime(0, fps),
        "metadata": {"shellx_cut": {"fps": fps, "width": tl.settings.width, "height": tl.settings.height}},
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "children": track_nodes,
        },
    });
    serde_json::to_string_pretty(&doc).map_err(|e| ExportError::BadInput(e.to_string()))
}

fn otio_track(
    tl: &crate::model::ExportTimeline,
    tr: &QTrack,
    tb: &Timebase,
    kind: &str,
) -> Result<Value, ExportError> {
    let fps = tb.fps_f64();
    let mut children: Vec<Value> = Vec::new();
    for item in &tr.items {
        match item {
            XItem::Gap { dur, .. } => {
                children.push(json!({
                    "OTIO_SCHEMA": "Gap.1",
                    "name": "gap",
                    "source_range": trange(0, *dur, fps),
                }));
            }
            XItem::Clip(clip) => {
                let asset =
                    tl.assets
                        .get(&clip.asset)
                        .ok_or_else(|| ExportError::MissingAsset {
                            clip_id: clip.clip_id.clone(),
                            asset: clip.asset.clone(),
                        })?;
                let avail = asset
                    .probe
                    .as_ref()
                    .and_then(|p| p.duration_ms)
                    .map(|d| tb.frames_from_ms(d).map(|frames| trange(0, frames, fps)))
                    .transpose()?;
                let media_ref = json!({
                    "OTIO_SCHEMA": "ExternalReference.1",
                    "target_url": file_uri(&asset.path),
                    "available_range": avail,
                });
                children.push(json!({
                    "OTIO_SCHEMA": "Clip.1",
                    "name": file_stem(&asset.path),
                    "source_range": trange(clip.offset, clip.dur, fps),
                    "media_reference": media_ref,
                    "metadata": {"shellx_cut": {"clip_id": clip.clip_id.clone(), "asset": clip.asset.clone()}},
                }));
            }
        }
    }
    Ok(json!({
        "OTIO_SCHEMA": "Track.1",
        "name": tr.id,
        "kind": kind,
        "children": children,
    }))
}

/// A parsed OTIO clip (the round-trip / import view): clip-local times in MS.
#[derive(Debug, Clone, PartialEq)]
pub struct OtioClip {
    pub name: String,
    pub is_gap: bool,
    /// Source in-point (ms) for a media clip; 0 for a gap.
    pub src_in_ms: u64,
    /// Duration (ms) on the timeline.
    pub dur_ms: u64,
    /// `file://` target for a media clip (empty for a gap).
    pub target_url: String,
}

/// A parsed OTIO track.
#[derive(Debug, Clone, PartialEq)]
pub struct OtioTrack {
    pub name: String,
    /// "video" | "audio" (lowercased OTIO kind).
    pub kind: String,
    pub clips: Vec<OtioClip>,
}

/// Parse OTIO JSON back to a flat track/clip list — the basis of the lossless
/// round-trip test and a future `import.otio`. Tolerant: unknown nodes are skipped.
pub fn parse_otio(s: &str) -> Result<Vec<OtioTrack>, ExportError> {
    let v: Value = serde_json::from_str(s).map_err(|e| ExportError::BadInput(e.to_string()))?;
    let rate_of = |tr: &Value, pointer: &str| -> f64 {
        tr.pointer(pointer)
            .and_then(|x| x.as_f64())
            .or_else(|| {
                v.pointer("/global_start_time/rate")
                    .and_then(|x| x.as_f64())
            })
            .unwrap_or(30.0)
    };
    let tracks = v
        .pointer("/tracks/children")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for tr in &tracks {
        if tr.get("OTIO_SCHEMA").and_then(|x| x.as_str()) != Some("Track.1") {
            continue;
        }
        let kind = tr
            .get("kind")
            .and_then(|x| x.as_str())
            .unwrap_or("Video")
            .to_lowercase();
        let name = tr
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let mut clips = Vec::new();
        for ch in tr
            .get("children")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let schema = ch.get("OTIO_SCHEMA").and_then(|x| x.as_str()).unwrap_or("");
            let start_rate = rate_of(&ch, "/source_range/start_time/rate");
            let duration_rate = rate_of(&ch, "/source_range/duration/rate");
            let start = ch
                .pointer("/source_range/start_time/value")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let dur = ch
                .pointer("/source_range/duration/value")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            match schema {
                "Gap.1" => clips.push(OtioClip {
                    name: "gap".into(),
                    is_gap: true,
                    src_in_ms: 0,
                    dur_ms: frames_to_ms(dur, duration_rate),
                    target_url: String::new(),
                }),
                "Clip.1" | "Clip.2" => clips.push(OtioClip {
                    name: ch
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    is_gap: false,
                    src_in_ms: frames_to_ms(start, start_rate),
                    dur_ms: frames_to_ms(dur, duration_rate),
                    target_url: active_media_target(&ch),
                }),
                _ => {}
            }
        }
        out.push(OtioTrack { name, kind, clips });
    }
    Ok(out)
}

fn active_media_target(clip: &Value) -> String {
    if let Some(target) = clip
        .pointer("/media_reference/target_url")
        .and_then(Value::as_str)
    {
        return target.to_string();
    }
    let key = clip
        .get("active_media_reference_key")
        .and_then(Value::as_str)
        .unwrap_or("DEFAULT_MEDIA");
    clip.get("media_references")
        .and_then(Value::as_object)
        .and_then(|references| references.get(key))
        .and_then(|reference| reference.get("target_url"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2-track timeline (video clip + gap + clip; audio clip) round-trips through
    /// OTIO LOSSLESSLY: export → parse reproduces the track/clip structure + times.
    #[test]
    fn otio_round_trip_is_lossless() {
        let timeline = json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30.0, "audio_rate": 48000},
            "assets": {
                "a1": {"path": "/m/clip1.mp4", "probe": {"duration_ms": 10000}},
                "a2": {"path": "/m/voice.wav", "probe": {"duration_ms": 8000}}
            },
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "c1", "asset": "a1", "src_in_ms": 1000, "src_out_ms": 3000},
                    {"kind": "gap", "duration_ms": 500},
                    {"id": "c2", "asset": "a1", "src_in_ms": 4000, "src_out_ms": 6000}
                ]},
                {"id": "a1t", "kind": "audio", "clips": [
                    {"id": "c3", "asset": "a2", "src_in_ms": 0, "src_out_ms": 8000}
                ]},
                {"id": "cap", "kind": "caption", "clips": [
                    {"text": "hi", "range_ms": [0, 1000]}
                ]}
            ]
        });
        let otio = export_otio(&timeline, "test").expect("export");
        // It's valid JSON with the OTIO schema marker.
        assert!(otio.contains("\"OTIO_SCHEMA\": \"Timeline.1\""));
        let parsed = parse_otio(&otio).expect("parse");
        // Caption track dropped → 2 tracks.
        assert_eq!(parsed.len(), 2, "video + audio tracks (caption dropped)");
        let v = &parsed[0];
        assert_eq!(v.kind, "video");
        assert_eq!(v.clips.len(), 3);
        // Clip 1: src_in 1000ms, duration 2000ms, the right media.
        assert!(!v.clips[0].is_gap);
        assert_eq!(v.clips[0].src_in_ms, 1000);
        assert_eq!(v.clips[0].dur_ms, 2000);
        assert!(v.clips[0].target_url.ends_with("/m/clip1.mp4"));
        // Gap: 500ms.
        assert!(v.clips[1].is_gap);
        assert_eq!(v.clips[1].dur_ms, 500);
        // Clip 2.
        assert_eq!(v.clips[2].src_in_ms, 4000);
        assert_eq!(v.clips[2].dur_ms, 2000);
        // Audio track.
        assert_eq!(parsed[1].kind, "audio");
        assert_eq!(parsed[1].clips[0].dur_ms, 8000);
        assert!(parsed[1].clips[0].target_url.ends_with("/m/voice.wav"));
    }

    #[test]
    fn parse_otio_uses_duration_rate_independently_from_start_rate() {
        let otio = json!({
            "OTIO_SCHEMA": "Timeline.1",
            "global_start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 0.0},
            "tracks": {
                "OTIO_SCHEMA": "Stack.1",
                "children": [{
                    "OTIO_SCHEMA": "Track.1",
                    "name": "v1",
                    "kind": "Video",
                    "children": [{
                        "OTIO_SCHEMA": "Clip.1",
                        "name": "mixed-rate",
                        "source_range": {
                            "OTIO_SCHEMA": "TimeRange.1",
                            "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": 48.0},
                            "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 48.0, "value": 48.0}
                        },
                        "media_reference": {"target_url": "file:///m/a.mov"}
                    }]
                }]
            }
        })
        .to_string();

        let parsed = parse_otio(&otio).expect("parse mixed-rate OTIO");
        let clip = &parsed[0].clips[0];
        assert_eq!(clip.src_in_ms, 2000, "start_time uses 24 fps");
        assert_eq!(clip.dur_ms, 1000, "duration uses its own 48 fps rate");
    }

    #[test]
    fn parse_current_clip_schema_uses_active_media_reference() {
        let otio = json!({
            "OTIO_SCHEMA":"Timeline.1",
            "global_start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":24.0,"value":0},
            "tracks":{"OTIO_SCHEMA":"Stack.1","children":[{
                "OTIO_SCHEMA":"Track.1","name":"v1","kind":"Video","children":[{
                    "OTIO_SCHEMA":"Clip.2","name":"current",
                    "source_range":{
                        "OTIO_SCHEMA":"TimeRange.1",
                        "start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":24.0,"value":24.0},
                        "duration":{"OTIO_SCHEMA":"RationalTime.1","rate":24.0,"value":48.0}
                    },
                    "media_references":{
                        "proxy":{"OTIO_SCHEMA":"ExternalReference.1","target_url":"proxy.mov"},
                        "DEFAULT_MEDIA":{"OTIO_SCHEMA":"ExternalReference.1","target_url":"source.mov"}
                    },
                    "active_media_reference_key":"DEFAULT_MEDIA"
                }]}
            ]}
        })
        .to_string();
        let parsed = parse_otio(&otio).unwrap();
        assert_eq!(parsed[0].clips[0].src_in_ms, 1000);
        assert_eq!(parsed[0].clips[0].dur_ms, 2000);
        assert_eq!(parsed[0].clips[0].target_url, "source.mov");
    }

    #[test]
    fn otio_uses_shared_quantized_duration_for_ragged_ms_ranges() {
        let timeline = json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30.0, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/ragged.mp4", "probe": {"duration_ms": 10000}}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 15, "src_out_ms": 50}
            ]}]
        });
        let otio = export_otio(&timeline, "ragged").expect("export");
        let parsed = parse_otio(&otio).expect("parse");
        assert_eq!(parsed[0].clips[0].src_in_ms, 0);
        assert_eq!(
            parsed[0].clips[0].dur_ms, 67,
            "2 frames at 30fps, matching quantize(out)-quantize(in)"
        );
    }

    #[test]
    fn otio_rejects_bad_fps_and_empty_timelines_like_other_exports() {
        let bad_fps = json!({
            "settings": {"fps": 25.5},
            "assets": {"a1": {"path": "/m/a.mp4"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}
            ]}]
        });
        assert!(matches!(
            export_otio(&bad_fps, "bad").unwrap_err(),
            ExportError::BadFps(_)
        ));

        let empty = json!({
            "settings": {"fps": 30},
            "assets": {},
            "tracks": []
        });
        assert!(matches!(
            export_otio(&empty, "empty").unwrap_err(),
            ExportError::EmptyTimeline
        ));
    }

    #[test]
    fn otio_exports_all_video_tracks() {
        let timeline = json!({
            "settings": {"width": 1920, "height": 1080, "fps": 30.0, "audio_rate": 48000},
            "assets": {
                "a1": {"path": "/m/base.mp4", "probe": {"duration_ms": 10000}},
                "a2": {"path": "/m/overlay.mp4", "probe": {"duration_ms": 10000}}
            },
            "tracks": [
                {"id": "v1", "kind": "video", "clips": [
                    {"id": "base", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}
                ]},
                {"id": "v2", "kind": "video", "clips": [
                    {"id": "overlay", "asset": "a2", "src_in_ms": 0, "src_out_ms": 500}
                ]}
            ]
        });
        let otio = export_otio(&timeline, "layers").expect("export");
        let parsed = parse_otio(&otio).expect("parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "v1");
        assert_eq!(parsed[1].name, "v2");
        assert!(parsed[1].clips[0].target_url.ends_with("/m/overlay.mp4"));
    }
}
