//! Shared export test harness (talk.mp4, 1920x1080,
//! 30fps, stereo 48kHz, 60s source, keep [0-5s)+[10-15s) butt-joined) plus
//! roxmltree helpers for structural comparison against the known-good files
//! in app/export/tests/fixtures/nle-xml/. Comparison is field-based, never
//! byte equality.
// Each test binary uses a different helper subset — dead_code is expected.
#![allow(dead_code)]

use roxmltree::{Document, Node};
use serde_json::{json, Value};

/// The shared scenario as timeline/op-log contract timeline JSON. Path matches the example
/// files so relink references compare equal.
pub fn scenario() -> Value {
    json!({
        "schema": "shellx-cut/1",
        "settings": {"width": 1920, "height": 1080, "fps": 30, "audio_rate": 48000},
        "assets": {
            "a1": {
                "path": "/home/user/media/talk.mp4",
                "hash": "sha256:test",
                "probe": {
                    "duration_ms": 60000,
                    "width": 1920,
                    "height": 1080,
                    "has_video": true,
                    "has_audio": true,
                    "audio_channels": 2,
                    "sample_rate": 48000
                }
            }
        },
        "tracks": [
            {"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 5000, "effects": [], "gain_db": 0},
                {"id": "c2", "asset": "a1", "src_in_ms": 10000, "src_out_ms": 15000, "effects": [], "gain_db": 0}
            ]},
            {"id": "a1t", "kind": "audio", "clips": [
                {"id": "c3", "asset": "a1", "src_in_ms": 0, "src_out_ms": 5000},
                {"id": "c4", "asset": "a1", "src_in_ms": 10000, "src_out_ms": 15000}
            ]},
            {"id": "cap1", "kind": "caption", "clips": [
                {"id": "s1", "text": "Hello world", "style_ref": "brand1", "range_ms": [0, 1200]},
                {"id": "s2", "text": "Second line", "style_ref": "brand1", "range_ms": [1500, 2600]}
            ]}
        ]
    })
}

/// Load a known-good example document's text.
pub fn example(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/nle-xml/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// All elements with the given tag, in document order.
pub fn elems<'a, 'input>(doc: &'a Document<'input>, tag: &str) -> Vec<Node<'a, 'input>> {
    doc.descendants()
        .filter(|n| n.is_element() && n.has_tag_name(tag))
        .collect()
}

/// Text of the first direct child element with the given tag ("" if absent).
pub fn child_text<'a>(n: Node<'a, 'a>, tag: &str) -> String {
    n.children()
        .find(|c| c.is_element() && c.has_tag_name(tag))
        .and_then(|c| c.text())
        .unwrap_or("")
        .to_string()
}

/// MLT `<property name=...>` text under a node ("" if absent).
pub fn prop<'a>(n: Node<'a, 'a>, name: &str) -> String {
    n.children()
        .find(|c| c.is_element() && c.has_tag_name("property") && c.attribute("name") == Some(name))
        .and_then(|c| c.text())
        .unwrap_or("")
        .to_string()
}

/// Assert the listed attributes are equal between two elements.
pub fn assert_attrs_eq(ours: Node, example: Node, attrs: &[&str], what: &str) {
    for a in attrs {
        assert_eq!(
            ours.attribute(*a),
            example.attribute(*a),
            "{what}: attribute '{a}' differs"
        );
    }
}
