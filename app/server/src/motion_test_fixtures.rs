//! Shared Motion connector fixtures and assertions for server tests.

use serde_json::{json, Value};

pub(crate) fn linked_effect_motion_document() -> &'static [u8] {
    br##"{"schema":"shellx-motion/motion@1","id":"motion-lower-third","durationMs":1500,"fps":30,"width":1280,"height":720,"layers":[{"id":"subject","name":"Hero subject","type":"video","keying":{"schema":"shellx-motion/chroma-key@1","keyColor":"#00ff00","spillSuppression":0.72,"matte":{"featherPx":3}},"mask":{"type":"roto","schema":"shellx-motion/roto-mask@1","frames":[{"atMs":0,"vertices":[]}],"tracking":{"model":"similarity","analysisId":"private-track-id"}}}]}"##
}

pub(crate) fn assert_linked_effect_summary(clip: &Value) {
    let effects = &clip["motion_link"]["effects"];
    assert_eq!(
        effects["schema"],
        json!("shellx-cut/motion-effects-summary@1")
    );
    assert_eq!(effects["keyedLayerCount"], json!(1));
    assert_eq!(effects["rotoLayerCount"], json!(1));
    assert_eq!(effects["trackedRotoLayerCount"], json!(1));
    assert_eq!(effects["layers"][0]["keying"]["matteCleanup"], json!(true));
    assert_eq!(effects["layers"][0]["roto"]["model"], json!("similarity"));
    assert!(!effects.to_string().contains("private-track-id"));
}
