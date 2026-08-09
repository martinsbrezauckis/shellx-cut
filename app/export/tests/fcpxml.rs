//! FCPXML 1.11 structural tests: render the shared export fixture and
//! compare key fields against examples/minimal.fcpxml (parse both — never
//! byte equality). Well-formedness is proven by the roxmltree parse itself.

mod common;

use common::*;
use cut_export::{export_xml, ExportError, XmlFormat};
use roxmltree::Document;
use serde_json::json;

#[test]
fn fcpxml_matches_known_good_example() {
    let ours_text = export_xml(&scenario(), XmlFormat::Fcpxml).expect("render fcpxml");
    let ours = Document::parse(&ours_text).expect("our output must be well-formed XML");
    let ex_text = example("minimal.fcpxml");
    let ex = Document::parse(&ex_text).unwrap();

    // Root version.
    assert_eq!(
        elems(&ours, "fcpxml")[0].attribute("version"),
        elems(&ex, "fcpxml")[0].attribute("version")
    );

    // <format>: frameDuration multiples rule starts here (gotcha — every time
    // value must be a multiple of this).
    let (of, ef) = (elems(&ours, "format"), elems(&ex, "format"));
    assert_eq!(of.len(), 1);
    assert_eq!(ef.len(), 1);
    assert_attrs_eq(
        of[0],
        ef[0],
        &[
            "id",
            "name",
            "frameDuration",
            "width",
            "height",
            "colorSpace",
        ],
        "format",
    );

    // <asset>: duration = FULL SOURCE duration (1800/30s for the 60s source),
    // not the 10s timeline — the classic source-duration regression.
    let (oa, ea) = (elems(&ours, "asset"), elems(&ex, "asset"));
    assert_eq!(oa.len(), 1);
    assert_attrs_eq(
        oa[0],
        ea[0],
        &[
            "id",
            "name",
            "start",
            "duration",
            "hasVideo",
            "hasAudio",
            "format",
            "audioSources",
            "audioChannels",
        ],
        "asset",
    );
    assert_eq!(
        oa[0].attribute("duration"),
        Some("1800/30s"),
        "asset duration must be SOURCE duration"
    );

    // <media-rep>: percent-encoded file:// URI — the relink path.
    let (om, em) = (elems(&ours, "media-rep"), elems(&ex, "media-rep"));
    assert_attrs_eq(om[0], em[0], &["kind", "src"], "media-rep");

    // <sequence> attrs (audioLayout stereo, audioRate 48k, NDF).
    let (os, es) = (elems(&ours, "sequence"), elems(&ex, "sequence"));
    assert_attrs_eq(
        os[0],
        es[0],
        &["format", "tcStart", "tcFormat", "audioLayout", "audioRate"],
        "sequence",
    );

    // Spine clips: offset = TIMELINE position, start = SOURCE in-point
    // (the source-to-timeline inversion), all multiples of 1/30s by construction. The
    // example has two video clips; ShellX also emits the scenario's separate
    // audio track, so each expected timing tuple appears twice.
    let (oc, ec) = (elems(&ours, "asset-clip"), elems(&ex, "asset-clip"));
    assert_eq!(oc.len(), ec.len() * 2, "video + standalone audio clips");
    let tuple = |n: roxmltree::Node<'_, '_>| -> Vec<String> {
        ["name", "ref", "offset", "duration", "start", "tcFormat"]
            .iter()
            .map(|attr| n.attribute(*attr).unwrap_or("").to_string())
            .collect()
    };
    for e in &ec {
        let expected = tuple(*e);
        let count = oc.iter().filter(|o| tuple(**o) == expected).count();
        assert_eq!(count, 2, "expected video+audio copies of {expected:?}");
    }
    // Second clip is the load-bearing one: timeline 5s, source 10s.
    assert!(
        oc.iter()
            .any(|c| c.attribute("offset") == Some("150/30s")
                && c.attribute("start") == Some("300/30s")),
        "second clip timing missing"
    );

    // No gap elements in the butt-joined scenario.
    assert!(elems(&ours, "gap").is_empty());

    // Caption track must NOT leak into FCPXML (ships via export.srt).
    assert!(!ours_text.contains("Hello world"));
}

#[test]
fn resolve_variant_identical_for_single_audio_layer() {
    // With the current single-layer timeline, fcpxml and Resolve
    // share one code path and emit identical documents.
    let a = export_xml(&scenario(), XmlFormat::Fcpxml).unwrap();
    let b = export_xml(&scenario(), XmlFormat::Resolve).unwrap();
    assert_eq!(a, b);
}

#[test]
fn resolve_rejects_audio_stream_above_zero() {
    // Hard-error rule: never emit a file Resolve mis-reads.
    let mut tl = scenario();
    tl["tracks"][1]["clips"][0]["stream"] = serde_json::json!(1);
    let err = export_xml(&tl, XmlFormat::Resolve).unwrap_err();
    assert!(
        matches!(err, ExportError::ResolveStream { .. }),
        "got {err:?}"
    );
    // Plain FCP export still fine.
    export_xml(&tl, XmlFormat::Fcpxml).unwrap();
}

#[test]
fn fcpxml_keeps_standalone_audio_tracks_alongside_video() {
    let mut tl = scenario();
    tl["assets"]["music"] = json!({
        "path": "/home/user/media/music.wav",
        "hash": "sha256:music",
        "probe": {
            "duration_ms": 10000,
            "has_video": false,
            "has_audio": true,
            "audio_channels": 2,
            "sample_rate": 48000
        }
    });
    tl["tracks"].as_array_mut().unwrap().push(json!({
        "id": "music",
        "kind": "audio",
        "clips": [
            {"id": "m1", "asset": "music", "src_in_ms": 0, "src_out_ms": 10000}
        ]
    }));

    let text = export_xml(&tl, XmlFormat::Fcpxml).expect("render fcpxml");
    let doc = Document::parse(&text).expect("our output must be well-formed XML");
    let clips = elems(&doc, "asset-clip");

    assert!(
        clips.iter().any(|c| c.attribute("name") == Some("music")),
        "standalone audio track should emit an asset-clip, got {text}"
    );
    assert!(
        clips
            .iter()
            .any(|c| { c.attribute("name") == Some("music") && c.attribute("lane").is_some() }),
        "standalone audio track should be connected on a lane, got {text}"
    );
}
