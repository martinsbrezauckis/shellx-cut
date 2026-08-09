//! Mutation regressions for schema-owned generated behavior metadata.

use super::{VerbRegistry, VERBS_JSON};

fn mutable_schema() -> serde_json::Value {
    serde_json::from_str(VERBS_JSON).expect("embedded schema is valid JSON")
}

fn verb_mut<'a>(schema: &'a mut serde_json::Value, name: &str) -> &'a mut serde_json::Value {
    schema["verbs"]
        .as_array_mut()
        .expect("verbs array")
        .iter_mut()
        .find(|verb| verb["name"] == name)
        .unwrap_or_else(|| panic!("missing fixture verb {name}"))
}

#[test]
fn missing_behavior_field_fails_closed_before_a_verb_can_run() {
    let mut schema = mutable_schema();
    verb_mut(&mut schema, "edit.add_marker")["behavior"]
        .as_object_mut()
        .expect("behavior object")
        .remove("agent_chat");

    let error = VerbRegistry::try_load_source(&schema.to_string())
        .expect_err("a verb without total behavior metadata must be rejected");
    assert!(error.contains("missing field `agent_chat`"), "{error}");
}

#[test]
fn stale_behavior_field_fails_against_the_generated_contract() {
    let mut schema = mutable_schema();
    verb_mut(&mut schema, "project.open")["behavior"]["mutation_class"] = serde_json::json!("read");

    let error = VerbRegistry::try_load_source(&schema.to_string())
        .expect_err("a source/generated behavior mismatch must be rejected");
    assert!(
        error.contains("project.open") && error.contains("differs from generated core contract"),
        "{error}"
    );
}
