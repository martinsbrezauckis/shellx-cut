use super::canonical::canonical_json;
use super::contract::{OutputClass, RequestClass, RequestField, RequestSchema};
use super::syntax::*;
use super::ConsumerError;
use serde_json::Value;
use std::collections::BTreeMap;

const REQUEST_SCHEMA: &str = "shellx-motion/connector-request-schema@1";
const IMPORT_PLAN_SCHEMA: &str = "shellx-motion/cut-import-plan@1";
const REQUIRED_OUTPUTS: [&str; 4] = [
    "artifact_handle",
    "cut_import_plan",
    "receipt",
    "rendered_media",
];

pub(super) fn parse_request_schema(value: &Value) -> Result<RequestSchema, ConsumerError> {
    let schema = required_object(
        value,
        &["schema", "id", "maxBytes", "fields"],
        "connector request schema",
    )?;
    required_exact_string(schema, "schema", REQUEST_SCHEMA, "connector request schema")?;
    let id = schema_id(&required_string_from(
        schema,
        "id",
        "connector request schema id",
        192,
    )?)?;
    let max_bytes = integer(
        schema.get("maxBytes").unwrap_or(&Value::Null),
        "connector request max bytes",
        1,
        65_536,
    )? as usize;
    let values = schema
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| ConsumerError::refusal("connector request fields must be an array"))?;
    if values.len() > 16 {
        return Err(ConsumerError::refusal(
            "connector request has too many fields",
        ));
    }
    let mut fields = Vec::with_capacity(values.len());
    for value in values {
        fields.push(parse_request_field(value)?);
    }
    require_sorted(
        fields.iter().map(|field| field.id.clone()).collect(),
        "connector request fields",
    )?;
    Ok(RequestSchema {
        id,
        max_bytes,
        fields,
    })
}

fn parse_request_field(value: &Value) -> Result<RequestField, ConsumerError> {
    let field = value
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("connector request field must be an object"))?;
    if field.keys().any(|key| {
        ![
            "id",
            "type",
            "required",
            "maxLength",
            "minimum",
            "maximum",
            "values",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ConsumerError::refusal(
            "connector request field contains an unsafe authority field",
        ));
    }
    let id = identifier(
        &required_string_from(field, "id", "connector request field id", 128)?,
        "connector request field id",
    )?;
    if unsafe_field_id(&id) {
        return Err(ConsumerError::refusal(
            "connector request field names a path, URL, executable, or callback authority",
        ));
    }
    let required = field
        .get("required")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ConsumerError::refusal("connector request field required must be boolean")
        })?;
    let class = match required_string_from(field, "type", "connector request field type", 32)?
        .as_str()
    {
        "boolean" => RequestClass::Boolean,
        "enum" => RequestClass::Enum(
            string_array(
                field.get("values").unwrap_or(&Value::Null),
                "connector request enum values",
                1,
                16,
                128,
            )?
            .into_iter()
            .collect(),
        ),
        "integer" => RequestClass::Integer {
            minimum: integer(
                field.get("minimum").unwrap_or(&Value::Null),
                "connector request minimum",
                -1_000_000,
                1_000_000,
            )?,
            maximum: integer(
                field.get("maximum").unwrap_or(&Value::Null),
                "connector request maximum",
                -1_000_000,
                1_000_000,
            )?,
        },
        "opaque-reference" => RequestClass::OpaqueReference {
            max_length: integer(
                field.get("maxLength").unwrap_or(&Value::Null),
                "connector request maxLength",
                1,
                1024,
            )? as usize,
        },
        "string" => return Err(ConsumerError::refusal(
            "connector request string fields are not admitted by the Cut opaque-reference baseline",
        )),
        _ => {
            return Err(ConsumerError::refusal(
                "connector request field type is outside the closed safe subset",
            ))
        }
    };
    if let RequestClass::Integer { minimum, maximum } = class {
        if minimum > maximum {
            return Err(ConsumerError::refusal(
                "connector request integer range is invalid",
            ));
        }
        return Ok(RequestField {
            id,
            required,
            class: RequestClass::Integer { minimum, maximum },
        });
    }
    Ok(RequestField {
        id,
        required,
        class,
    })
}

pub(super) fn parse_outputs(
    value: &Value,
    require_cut_class: bool,
) -> Result<BTreeMap<String, OutputClass>, ConsumerError> {
    let outputs = value
        .as_array()
        .ok_or_else(|| ConsumerError::refusal("descriptor outputs must be an array"))?;
    if outputs.len() > 8 || (require_cut_class && outputs.len() != REQUIRED_OUTPUTS.len()) {
        return Err(ConsumerError::refusal(
            "descriptor output class is unsupported",
        ));
    }
    let mut parsed = BTreeMap::new();
    for value in outputs {
        let output = required_object(
            value,
            &["role", "mediaKinds", "schemas"],
            "descriptor output",
        )?;
        let role = required_string_from(output, "role", "descriptor output role", 64)?;
        let media_kinds = string_array(
            output.get("mediaKinds").unwrap_or(&Value::Null),
            "descriptor output media kinds",
            1,
            8,
            128,
        )?;
        let schemas = string_array(
            output.get("schemas").unwrap_or(&Value::Null),
            "descriptor output schemas",
            1,
            8,
            192,
        )?;
        let expected = match role.as_str() {
            "artifact_handle" => ("application/json", "shellx-motion/artifact-handle@1"),
            "cut_import_plan" => ("application/json", IMPORT_PLAN_SCHEMA),
            "receipt" => ("application/json", "shellx-motion/receipt@1"),
            "rendered_media" => ("video/mp4", "shellx-motion/artifact-handle-ref@1"),
            _ if require_cut_class => {
                return Err(ConsumerError::refusal(
                    "descriptor declares an unsupported artifact class",
                ))
            }
            _ => {
                if unsafe_field_id(&role) {
                    return Err(ConsumerError::refusal(
                        "descriptor output names an unsafe authority class",
                    ));
                }
                let class = OutputClass {
                    media_kind: media_kinds[0].clone(),
                    schema: schemas[0].clone(),
                };
                if parsed.insert(role, class).is_some() {
                    return Err(ConsumerError::refusal(
                        "descriptor has duplicate output roles",
                    ));
                }
                continue;
            }
        };
        if media_kinds != [expected.0]
            || schemas != [expected.1]
            || parsed
                .insert(
                    role.clone(),
                    OutputClass {
                        media_kind: expected.0.to_owned(),
                        schema: expected.1.to_owned(),
                    },
                )
                .is_some()
        {
            return Err(ConsumerError::refusal(
                "descriptor output does not match the negotiated Cut output class",
            ));
        }
    }
    if require_cut_class
        && parsed.keys().map(String::as_str).collect::<Vec<_>>() != REQUIRED_OUTPUTS
    {
        return Err(ConsumerError::refusal(
            "descriptor output roles are not the supported Cut set",
        ));
    }
    Ok(parsed)
}

pub(super) fn validate_request(
    schema: &RequestSchema,
    request: &Value,
) -> Result<(), ConsumerError> {
    let request = request
        .as_object()
        .ok_or_else(|| ConsumerError::refusal("connector request must be an object"))?;
    if request
        .keys()
        .any(|key| !schema.fields.iter().any(|field| field.id == *key))
    {
        return Err(ConsumerError::refusal(
            "connector request contains an unadvertised field",
        ));
    }
    for field in &schema.fields {
        let value = request.get(&field.id);
        if value.is_none() && field.required {
            return Err(ConsumerError::refusal(
                "connector request omits a required descriptor field",
            ));
        }
        if let Some(value) = value {
            match &field.class {
                RequestClass::Boolean if !value.is_boolean() => {
                    return Err(ConsumerError::refusal(
                        "connector request boolean field is invalid",
                    ))
                }
                RequestClass::Enum(values)
                    if value.as_str().is_none_or(|item| !values.contains(item)) =>
                {
                    return Err(ConsumerError::refusal(
                        "connector request enum field is invalid",
                    ))
                }
                RequestClass::Integer { minimum, maximum }
                    if value
                        .as_i64()
                        .is_none_or(|item| item < *minimum || item > *maximum) =>
                {
                    return Err(ConsumerError::refusal(
                        "connector request integer field is invalid",
                    ))
                }
                RequestClass::OpaqueReference { max_length }
                    if value.as_str().is_none_or(|item| {
                        item.is_empty() || item.len() > *max_length || !opaque_reference(item)
                    }) =>
                {
                    return Err(ConsumerError::refusal(
                        "connector request opaque reference is unsafe",
                    ))
                }
                _ => {}
            }
        }
    }
    if canonical_json(&Value::Object(request.clone())).len() > schema.max_bytes {
        return Err(ConsumerError::refusal(
            "connector request exceeds its descriptor byte limit",
        ));
    }
    Ok(())
}
