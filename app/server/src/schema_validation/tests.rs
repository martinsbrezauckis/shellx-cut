use super::*;
use crate::registry::VerbRegistry;
use serde_json::{json, Map};
use std::collections::BTreeSet;

const SUPPORTED_INPUT_KEYWORDS: &[&str] = &[
    "additionalProperties",
    "anyOf",
    "const",
    "default",
    "description",
    "enum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "items",
    "maxItems",
    "maxLength",
    "maximum",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
    "oneOf",
    "pattern",
    "properties",
    "required",
    "type",
    "uniqueItems",
];

fn spec(name: &str, args: Value) -> VerbSpec {
    VerbSpec {
        name: name.to_string(),
        domain: "test".to_string(),
        description: "schema validator fixture".to_string(),
        args,
        result: "{}".to_string(),
        result_schema: None,
    }
}

fn validation_keyword(schema: Value, instance: Value) -> String {
    let spec = spec("test.keyword", schema);
    let compiled = CompiledVerbSchemas::compile(std::slice::from_ref(&spec)).unwrap();
    compiled
        .validate(&spec, &instance)
        .unwrap_err()
        .cause
        .split('\'')
        .nth(1)
        .unwrap()
        .to_string()
}

fn merge_objects(target: &mut Map<String, Value>, value: Value) {
    if let Value::Object(source) = value {
        target.extend(source);
    }
}

fn collect_schema_keywords(schema: &Value, keywords: &mut BTreeSet<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    for (keyword, value) in object {
        keywords.insert(keyword.clone());
        match keyword.as_str() {
            "properties" => {
                for property_schema in value.as_object().into_iter().flat_map(Map::values) {
                    collect_schema_keywords(property_schema, keywords);
                }
            }
            "items" => collect_schema_keywords(value, keywords),
            "oneOf" | "anyOf" | "allOf" => {
                for branch in value.as_array().into_iter().flatten() {
                    collect_schema_keywords(branch, keywords);
                }
            }
            _ => {}
        }
    }
}

/// Produce the smallest deterministic instance that satisfies the
/// structural constraints used by the committed registry. This is a
/// contract probe, not user-facing sample data: handler-level semantic
/// requirements deliberately remain outside JSON Schema.
fn minimal_instance(schema: &Value) -> Value {
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(value) = schema.get("default") {
        return value.clone();
    }
    if let Some(value) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return value.clone();
    }

    let schema_type = match schema.get("type") {
        Some(Value::String(value)) => Some(value.as_str()),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .find(|value| *value != "null"),
        _ if schema.get("properties").is_some()
            || schema.get("required").is_some()
            || schema.get("minProperties").is_some() =>
        {
            Some("object")
        }
        _ if schema.get("items").is_some() || schema.get("minItems").is_some() => Some("array"),
        _ if schema.get("minimum").is_some()
            || schema.get("maximum").is_some()
            || schema.get("exclusiveMinimum").is_some()
            || schema.get("exclusiveMaximum").is_some() =>
        {
            Some("number")
        }
        _ if schema.get("pattern").is_some()
            || schema.get("minLength").is_some()
            || schema.get("maxLength").is_some() =>
        {
            Some("string")
        }
        _ => None,
    };

    let mut instance = match schema_type {
        Some("object") => {
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut object = Map::new();
            for name in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let property_schema = properties.get(name).unwrap_or(&Value::Null);
                object.insert(name.to_string(), minimal_instance(property_schema));
            }
            let min_properties = schema
                .get("minProperties")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            for (name, property_schema) in &properties {
                if object.len() >= min_properties {
                    break;
                }
                object
                    .entry(name.clone())
                    .or_insert_with(|| minimal_instance(property_schema));
            }
            Value::Object(object)
        }
        Some("array") => {
            let item_schema = schema.get("items").unwrap_or(&Value::Null);
            let count = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            Value::Array((0..count).map(|_| minimal_instance(item_schema)).collect())
        }
        Some("integer") => {
            let mut value = schema.get("minimum").and_then(Value::as_i64).unwrap_or(0);
            if let Some(exclusive) = schema.get("exclusiveMinimum").and_then(Value::as_i64) {
                value = value.max(exclusive.saturating_add(1));
            }
            if let Some(exclusive) = schema.get("exclusiveMaximum").and_then(Value::as_i64) {
                value = value.min(exclusive.saturating_sub(1));
            }
            if let Some(maximum) = schema.get("maximum").and_then(Value::as_i64) {
                value = value.min(maximum);
            }
            json!(value)
        }
        Some("number") => {
            let mut value = schema.get("minimum").and_then(Value::as_f64).unwrap_or(0.0);
            if let Some(exclusive) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
                value = value.max(exclusive + 1.0);
            }
            if let Some(exclusive) = schema.get("exclusiveMaximum").and_then(Value::as_f64) {
                value = value.min(exclusive - 1.0);
            }
            if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
                value = value.min(maximum);
            }
            json!(value)
        }
        Some("string") => {
            let pattern = schema.get("pattern").and_then(Value::as_str);
            let value = match pattern {
                Some("^#(?:[0-9A-Fa-f]{3}|[0-9A-Fa-f]{4}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$") => {
                    "#000"
                }
                Some("^[0-9]+:[0-9]+$") => "1:1",
                Some("^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$") => "x",
                Some("^[A-Za-z0-9._:-]+$") => "cut:job-1",
                Some("^sha256:[0-9a-f]{64}$") => {
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                }
                Some(other) => {
                    panic!("add a deterministic sample for committed pattern {other}")
                }
                None => "x",
            };
            let min_length = schema.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
            if value.chars().count() >= min_length {
                json!(value)
            } else {
                json!("x".repeat(min_length))
            }
        }
        Some("boolean") => json!(false),
        Some("null") => Value::Null,
        Some(other) => panic!("unsupported committed schema type {other}"),
        None => Value::Null,
    };

    if let Value::Object(object) = &mut instance {
        if let Some(branches) = schema.get("oneOf").and_then(Value::as_array) {
            merge_objects(object, minimal_instance(&branches[0]));
        }
        if let Some(branches) = schema.get("anyOf").and_then(Value::as_array) {
            merge_objects(object, minimal_instance(&branches[0]));
        }
        if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
            for branch in branches {
                merge_objects(object, minimal_instance(branch));
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, value) in object.iter_mut() {
                if value.is_null() {
                    if let Some(property_schema) = properties.get(name) {
                        *value = minimal_instance(property_schema);
                    }
                }
            }
        }
    } else if let Some(branches) = schema.get("oneOf").and_then(Value::as_array) {
        instance = minimal_instance(&branches[0]);
    } else if let Some(branches) = schema.get("anyOf").and_then(Value::as_array) {
        instance = minimal_instance(&branches[0]);
    } else if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        instance = minimal_instance(&branches[0]);
    }
    instance
}

#[test]
fn enforces_every_keyword_used_by_the_committed_registry() {
    let registry = VerbRegistry::load();
    let mut committed_keywords = BTreeSet::new();
    for spec in &registry.verbs {
        collect_schema_keywords(&spec.args, &mut committed_keywords);
    }
    assert_eq!(
        committed_keywords,
        SUPPORTED_INPUT_KEYWORDS
            .iter()
            .map(|keyword| keyword.to_string())
            .collect(),
        "a schema keyword changed; add an explicit validator/error/generator test before accepting it"
    );

    let cases = [
        (json!({"type":"object"}), json!([]), "type"),
        (
            json!({"type":"object","required":["x"]}),
            json!({}),
            "required",
        ),
        (
            json!({
                "type":"object",
                "additionalProperties":false,
                "properties":{"known":{"type":"integer"}}
            }),
            json!({"x":1}),
            "additionalProperties",
        ),
        (json!({"enum":["a","b"]}), json!("c"), "enum"),
        (json!({"const":"a"}), json!("b"), "const"),
        (json!({"minimum":1}), json!(0), "minimum"),
        (json!({"maximum":1}), json!(2), "maximum"),
        (json!({"exclusiveMinimum":1}), json!(1), "exclusiveMinimum"),
        (json!({"exclusiveMaximum":1}), json!(1), "exclusiveMaximum"),
        (json!({"minLength":2}), json!("a"), "minLength"),
        (json!({"maxLength":1}), json!("ab"), "maxLength"),
        (json!({"pattern":"^a+$"}), json!("b"), "pattern"),
        (json!({"minItems":2}), json!([1]), "minItems"),
        (json!({"maxItems":1}), json!([1, 2]), "maxItems"),
        (json!({"uniqueItems":true}), json!([1, 1]), "uniqueItems"),
        (json!({"minProperties":1}), json!({}), "minProperties"),
    ];
    for (schema, instance, expected) in cases {
        assert_eq!(
            validation_keyword(schema, instance),
            expected,
            "keyword fixture {expected}"
        );
    }
}

#[test]
fn enforces_nested_items_and_composition() {
    let nested = spec(
        "test.nested",
        json!({
            "type":"object",
            "required":["rows"],
            "additionalProperties":false,
            "properties":{
                "rows":{"type":"array","items":{
                    "type":"object",
                    "required":["id"],
                    "additionalProperties":false,
                    "properties":{"id":{"type":"integer"}}
                }}
            }
        }),
    );
    let compiled = CompiledVerbSchemas::compile(std::slice::from_ref(&nested)).unwrap();
    let error = compiled
        .validate(&nested, &json!({"rows":[{"id":1},{"extra":true}]}))
        .unwrap_err();
    assert!(error.message.contains("/rows/1/extra"), "{error:?}");

    for (schema, invalid, valid) in [
        (
            json!({"oneOf":[{"type":"string"},{"type":"integer"}]}),
            json!(true),
            json!(1),
        ),
        (
            json!({"anyOf":[{"type":"string"},{"minimum":2}]}),
            json!(1),
            json!("ok"),
        ),
        (
            json!({"allOf":[{"type":"integer"},{"minimum":2}]}),
            json!(1),
            json!(2),
        ),
    ] {
        let spec = spec("test.composition", schema);
        let compiled = CompiledVerbSchemas::compile(std::slice::from_ref(&spec)).unwrap();
        assert!(compiled.validate(&spec, &invalid).is_err());
        assert!(compiled.validate(&spec, &valid).is_ok());
    }
}

#[test]
fn error_contract_is_bounded_and_does_not_echo_values() {
    let spec = spec(
        "ui.open",
        json!({
            "type":"object",
            "required":["panel"],
            "additionalProperties":false,
            "properties":{"panel":{"type":"string","enum":["assets","library"]}}
        }),
    );
    let compiled = CompiledVerbSchemas::compile(std::slice::from_ref(&spec)).unwrap();
    let required = compiled.validate(&spec, &json!({})).unwrap_err();
    assert_eq!(required.code, error_codes::INVALID_ARGS);
    assert!(required.message.contains("ui.open"));
    assert!(required.message.contains("/panel"));
    assert!(required.message.contains("required"));
    assert!(required
        .suggested_action
        .unwrap()
        .contains("GET /api/verbs"));

    let secret = "do-not-echo-this-sensitive-value";
    let wrong_enum = compiled
        .validate(&spec, &json!({"panel":secret}))
        .unwrap_err();
    assert!(!wrong_enum.message.contains(secret));
    assert!(!wrong_enum.cause.contains(secret));
    assert!(!wrong_enum.suggested_action.unwrap().contains(secret));

    let oversized_key = "secret".repeat(2_000);
    let unknown = compiled
        .validate(
            &spec,
            &json!({"panel":"assets", oversized_key.clone():true}),
        )
        .unwrap_err();
    assert!(!unknown.message.contains(&oversized_key));
    assert!(unknown.message.chars().count() < 300);
    assert!(unknown.cause.chars().count() < 300);
    assert!(unknown.suggested_action.unwrap().chars().count() < 400);
}

#[test]
fn generated_minimal_examples_pass_every_committed_schema() {
    let registry = VerbRegistry::load();
    for spec in &registry.verbs {
        let args = minimal_instance(&spec.args);
        registry
            .validate_args(spec, &args)
            .unwrap_or_else(|error| panic!("{} rejected generated {args}: {error:?}", spec.name));
    }
}
