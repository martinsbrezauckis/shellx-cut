//! receipt_schema.rs — drift guard between the Rust receipt types and the
//! agent-facing typed contract in schema/receipts.schema.json (receipt contract).
//!
//! The Rust structs (RenderReceipt, CheckResult, FixAction, FixTarget) are the
//! source of truth; receipts.schema.json mirrors their SERIALIZED shape so an
//! agent can reason over a typed contract. This test serializes a representative
//! value of each and asserts every `required` key the schema declares is present
//! in the JSON — so adding/renaming a field in Rust without updating the schema
//! (or vice-versa) fails the build instead of silently lying to the agent.
//!
//! Lightweight by design: a required-keys check, not a full JSON-Schema
//! validator (no extra dep) — enough to catch the drift that matters (a missing
//! contract key) without pulling a validator crate into the workspace.

use cut_core::{check_names, CheckResult, FixAction, FixTarget, RenderReceipt};
use serde_json::Value;

/// Load schema/receipts.schema.json from the repo (relative to cut-core's
/// manifest dir: app/core → app → repo → schema).
fn schema() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schema/receipts.schema.json"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&text).expect("receipts.schema.json must parse")
}

/// Required keys for a $def.
fn required(schema: &Value, def: &str) -> Vec<String> {
    schema["$defs"][def]["required"]
        .as_array()
        .unwrap_or_else(|| panic!("$defs.{def}.required must be an array"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

/// Assert every required key is present in the serialized value.
fn assert_has_required(value: &Value, schema: &Value, def: &str) {
    let obj = value
        .as_object()
        .expect("serialized value must be an object");
    for key in required(schema, def) {
        assert!(
            obj.contains_key(&key),
            "{def}: serialized value missing required key '{key}'"
        );
    }
}

#[test]
fn rust_receipt_types_match_typed_schema_contract() {
    let s = schema();

    // A FixTarget with all coords (skip_serializing_if drops Nones, so populate).
    let target = FixTarget {
        clip_id: Some("c3".into()),
        at_ms: Some(1200),
        op_id: Some("op_000042".into()),
    };
    let tv = serde_json::to_value(&target).unwrap();
    for key in ["clip_id", "at_ms", "op_id"] {
        assert!(
            tv.get(key).is_some(),
            "FixTarget should serialize {key} when Some"
        );
    }

    let fix = FixAction {
        check: check_names::LUFS.into(),
        fix_verb: "render.final".into(),
        fix_args: serde_json::json!({"normalize_loudness": -16}),
        targets: vec![target],
        measured: serde_json::json!({"integrated_lufs": -22.0}),
        rationale: "loudness off target".into(),
        auto_fixable: true,
    };
    assert_has_required(&serde_json::to_value(&fix).unwrap(), &s, "FixAction");

    let check = CheckResult {
        name: check_names::LUFS.into(),
        pass: false,
        details: serde_json::json!({"target_lufs": -16.0}),
        evidence: serde_json::json!({"integrated_lufs": -22.0}),
    };
    assert_has_required(&serde_json::to_value(&check).unwrap(), &s, "CheckResult");

    let mut receipt = RenderReceipt {
        render_id: "render_001".into(),
        ts: "2026-06-16T00:00:00.000Z".into(),
        output_path: "/o.mp4".into(),
        output_hash: "sha256:x".into(),
        duration_ms: 1000,
        preset: "standard".into(),
        at_op: "op_000001".into(),
        checks: vec![check],
        pass: false,
        judge: None,
        fix_actions: vec![],
    };
    receipt.compute_pass();
    let rv = serde_json::to_value(&receipt).unwrap();
    assert_has_required(&rv, &s, "RenderReceipt");
    // The computed fix_actions are present (a failing lufs check → one action).
    assert!(
        rv["fix_actions"].as_array().map(|a| a.len()).unwrap_or(0) >= 1,
        "a failing receipt must serialize a non-empty fix_actions list"
    );
    // And each fix_action conforms to the FixAction contract.
    assert_has_required(&rv["fix_actions"][0], &s, "FixAction");
}
