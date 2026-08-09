use super::*;
use cut_core::{Actor, OpEffect, OpStatus};
use serde_json::{json, Map};
use std::fs;

fn write_effect_package(root: &Path, extra_keyed_layers: usize) {
    fs::write(
        root.join("manifest.json"),
        r#"{"id":"pkg_effects","motion":"motion.json"}"#,
    )
    .unwrap();
    let mut layers = vec![json!({
        "id": "subject",
        "name": "Green-screen subject",
        "type": "video",
        "keying": {
            "schema": "shellx-motion/chroma-key@1",
            "keyColor": "#00ff00",
            "spillSuppression": 0.72,
            "matte": { "featherPx": 3 },
            "privateUnknown": "must-not-leak"
        },
        "mask": {
            "type": "roto",
            "schema": "shellx-motion/roto-mask@1",
            "frames": [{ "atMs": 0, "vertices": [] }],
            "tracking": { "model": "similarity", "analysisId": "private-track-id" }
        }
    })];
    layers.push(json!({
        "id": "untracked-roto",
        "type": "image",
        "mask": {
            "type": "roto",
            "schema": "shellx-motion/roto-mask@1",
            "frames": [{ "atMs": 0 }, { "atMs": 500 }]
        }
    }));
    layers.push(json!({
        "id": "ignored-text",
        "type": "text",
        "keying": { "schema": "shellx-motion/chroma-key@1", "keyColor": "#ffffff" }
    }));
    for index in 0..extra_keyed_layers {
        layers.push(json!({
            "id": format!("extra-{index}"),
            "name": "x".repeat(240),
            "type": "image",
            "keying": { "schema": "shellx-motion/chroma-key@1", "keyColor": "#123456" }
        }));
    }
    fs::write(
        root.join("motion.json"),
        serde_json::to_vec(&json!({
            "schema": "shellx-motion/motion@1",
            "id": "motion_effects",
            "layers": layers,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn effect_summary_is_typed_bounded_and_redacted() {
    let package = tempfile::tempdir().expect("package");
    write_effect_package(package.path(), 70);
    let summary = effect_summary(package.path());
    assert_eq!(summary["schema"], "shellx-cut/motion-effects-summary@1");
    assert_eq!(summary["keyedLayerCount"], 71);
    assert_eq!(summary["rotoLayerCount"], 2);
    assert_eq!(summary["trackedRotoLayerCount"], 1);
    assert_eq!(summary["truncated"], true);
    assert_eq!(summary["layers"].as_array().unwrap().len(), 64);
    assert_eq!(summary["layers"][0]["keying"]["keyColor"], "#00ff00");
    assert_eq!(summary["layers"][0]["keying"]["matteCleanup"], true);
    assert_eq!(summary["layers"][0]["roto"]["tracked"], true);
    assert_eq!(summary["layers"][0]["roto"]["model"], "similarity");
    let serialized = serde_json::to_string(&summary).unwrap();
    assert!(!serialized.contains("must-not-leak"));
    assert!(!serialized.contains("private-track-id"));
    let longest_name = summary["layers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|layer| layer.get("name").and_then(Value::as_str))
        .map(str::len)
        .max()
        .unwrap();
    assert!(longest_name <= MAX_VISIBLE_LABEL_CHARS);
}

#[test]
fn project_projection_uses_latest_replay_link_and_local_effect_state() {
    let package = tempfile::tempdir().expect("package");
    write_effect_package(package.path(), 0);
    let render = package.path().join("render.mp4");
    let fallback = package.path().join("fallback.mp4");
    let plan = package.path().join("plan.json");
    fs::write(&render, b"render").unwrap();
    fs::write(&fallback, b"fallback").unwrap();
    fs::write(&plan, b"plan").unwrap();
    let revision = crate::motion_package::revision(package.path()).unwrap();
    let link = json!({
        "schema": "shellx-cut/motion-link@1",
        "clipId": "clip-1",
        "sourcePath": package.path(),
        "sourceRevision": revision,
        "sourceRevisionKind": "motion-package",
        "planPath": plan,
        "render": { "path": render },
        "fallbackPath": fallback,
        "state": "linked-current"
    });
    let op = link_op("motion.link.refresh", link);
    let mut project = json!({
        "tracks": [{ "clips": [{ "id": "clip-1" }, { "id": "clip-2" }] }]
    });
    annotate_project_state(&mut project, &[op]);
    let projected = &project["tracks"][0]["clips"][0]["motion_link"];
    assert_eq!(projected["state"], "linked-current");
    assert_eq!(projected["availability"]["source"], true);
    assert_eq!(projected["effects"]["keyedLayerCount"], 1);
    assert_eq!(projected["effects"]["rotoLayerCount"], 2);
    assert!(project["tracks"][0]["clips"][1]
        .get("motion_link")
        .is_none());
    let stale_link = projected.clone();

    fs::write(
        package.path().join("motion.json"),
        r#"{"id":"motion_effects","layers":[]}"#,
    )
    .unwrap();
    annotate_project_state(&mut project, &[link_op("motion.link.refresh", stale_link)]);
    assert_eq!(
        project["tracks"][0]["clips"][0]["motion_link"]["state"],
        "source-dirty"
    );
}

#[test]
fn malformed_document_reports_bounded_unavailable_summary() {
    let package = tempfile::tempdir().expect("package");
    fs::write(
        package.path().join("manifest.json"),
        r#"{"id":"pkg","motion":"motion.json"}"#,
    )
    .unwrap();
    fs::write(package.path().join("motion.json"), b"not-json").unwrap();
    assert_eq!(
        effect_summary(package.path()),
        json!({
            "schema": "shellx-cut/motion-effects-summary@1",
            "available": false,
            "ownership": "motion",
            "editableInCut": false,
            "reason": "unreadable-motion-document",
        })
    );
}

fn link_op(verb: &str, link: Value) -> OpRecord {
    let detail = json!({ "motion_links": [link] })
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    OpRecord {
        op_id: "op_000001".into(),
        ts: OpRecord::now_ts(),
        actor: Actor::system(),
        verb: verb.into(),
        args: json!({}),
        rationale: None,
        effects: vec![OpEffect {
            track: None,
            detail,
        }],
        inverse: None,
        status: OpStatus::Applied,
    }
}
