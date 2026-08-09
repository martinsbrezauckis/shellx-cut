//! Compiled input-schema validation for the shared verb dispatch boundary.
//!
//! Every transport (REST, CLI, MCP and internal recipe/plugin dispatch) reaches
//! `dispatch`, so validation belongs here rather than in any adapter. Schemas
//! are trusted, embedded build inputs; caller instances are untrusted. The
//! compiled validators perform no network or filesystem resolution.

use crate::registry::VerbSpec;
use cut_core::{error_codes, CutError};
use jsonschema::error::ValidationErrorKind;
use jsonschema::{PatternOptions, Validator};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct CompiledVerbSchemas {
    validators: HashMap<String, Validator>,
}

impl CompiledVerbSchemas {
    pub fn compile(specs: &[VerbSpec]) -> Result<Self, String> {
        let mut validators = HashMap::with_capacity(specs.len());
        for spec in specs {
            let validator = jsonschema::draft7::options()
                .with_pattern_options(PatternOptions::regex())
                .build(&spec.args)
                .map_err(|error| {
                    format!(
                        "verb '{}' input schema did not compile as JSON Schema Draft 7: {error}",
                        spec.name
                    )
                })?;
            if validators.insert(spec.name.clone(), validator).is_some() {
                return Err(format!("duplicate verb schema '{}'", spec.name));
            }
        }
        Ok(Self { validators })
    }

    pub fn validate(&self, spec: &VerbSpec, args: &Value) -> Result<(), CutError> {
        let validator = self
            .validators
            .get(&spec.name)
            .expect("every loaded verb must have one compiled input schema");
        // If a payload both omits required fields and includes a typo, report
        // the typo first. It is the most local corrective action and preserves
        // the prior central guard's useful unknown-key behavior.
        let Some(error) = validator.iter_errors(args).min_by_key(|error| {
            if matches!(
                error.kind(),
                ValidationErrorKind::AdditionalProperties { .. }
            ) {
                0
            } else {
                1
            }
        }) else {
            return Ok(());
        };
        Err(validation_error(&spec.name, &error))
    }
}

fn validation_error(verb: &str, error: &jsonschema::ValidationError<'_>) -> CutError {
    if let Some(detail) = composition_detail(error.kind()) {
        return validation_error(verb, detail);
    }
    let keyword = error.kind().keyword();
    let pointer = error_pointer(error);
    let human_path = pointer_to_human_path(&pointer);
    let expected = expected_constraint(error.kind());
    let path_suffix = if human_path.is_empty() {
        String::new()
    } else {
        format!(" ({human_path})")
    };
    CutError::new(
        error_codes::INVALID_ARGS,
        format!("invalid args for verb '{verb}' at '{pointer}'{path_suffix}: {keyword}"),
        format!("schema keyword '{keyword}' failed: {expected}"),
    )
    .with_suggested_action(format!(
        "correct '{pointer}' to {expected}; GET /api/verbs shows the exact input schema"
    ))
}

fn composition_detail(kind: &ValidationErrorKind) -> Option<&jsonschema::ValidationError<'static>> {
    let context = match kind {
        ValidationErrorKind::AnyOf { context }
        | ValidationErrorKind::OneOfNotValid { context }
        | ValidationErrorKind::OneOfMultipleValid { context } => context,
        _ => return None,
    };
    context
        .iter()
        .flatten()
        .min_by_key(|error| match error.kind() {
            ValidationErrorKind::AdditionalProperties { .. } => 0,
            ValidationErrorKind::Required { .. } => 1,
            ValidationErrorKind::Type { .. } => 2,
            _ => 3,
        })
}

/// Point at the actionable member, not only its containing object. JSON Schema
/// reports `required` and `additionalProperties` at the parent object.
fn error_pointer(error: &jsonschema::ValidationError<'_>) -> String {
    let base = error.instance_path().to_string();
    let member = match error.kind() {
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            unexpected.first().map(|value| bounded_text(value, 80))
        }
        ValidationErrorKind::Required { property } => property.as_str().map(str::to_string),
        _ => None,
    };
    match member {
        Some(member) => format!("{}/{}", base.trim_end_matches('/'), escape_pointer(&member)),
        None if base.is_empty() => "$".to_string(),
        None => base,
    }
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn pointer_to_human_path(pointer: &str) -> String {
    if pointer == "$" {
        return String::new();
    }
    pointer
        .trim_start_matches('/')
        .split('/')
        .map(|part| part.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>()
        .join(".")
}

fn expected_constraint(kind: &ValidationErrorKind) -> String {
    match kind {
        ValidationErrorKind::AdditionalProperties { .. } => {
            "use only declared properties; remove the property at this path".to_string()
        }
        ValidationErrorKind::Required { property } => {
            format!("include required property {}", bounded_json(property))
        }
        ValidationErrorKind::Type { kind } => format!("use the declared type ({kind:?})"),
        ValidationErrorKind::Enum { options } => {
            format!("use one of the declared values {}", bounded_json(options))
        }
        ValidationErrorKind::Constant { expected_value } => {
            format!("use the constant {}", bounded_json(expected_value))
        }
        ValidationErrorKind::Minimum { limit } => {
            format!(
                "use a number greater than or equal to {}",
                bounded_json(limit)
            )
        }
        ValidationErrorKind::Maximum { limit } => {
            format!("use a number less than or equal to {}", bounded_json(limit))
        }
        ValidationErrorKind::ExclusiveMinimum { limit } => {
            format!("use a number greater than {}", bounded_json(limit))
        }
        ValidationErrorKind::ExclusiveMaximum { limit } => {
            format!("use a number less than {}", bounded_json(limit))
        }
        ValidationErrorKind::MinLength { limit } => {
            format!("use a string with at least {limit} characters")
        }
        ValidationErrorKind::MaxLength { limit } => {
            format!("use a string with at most {limit} characters")
        }
        ValidationErrorKind::Pattern { pattern } => {
            format!(
                "use a string matching pattern {}",
                bounded_text(pattern, 120)
            )
        }
        ValidationErrorKind::MinItems { limit } => {
            format!("use an array with at least {limit} items")
        }
        ValidationErrorKind::MaxItems { limit } => {
            format!("use an array with at most {limit} items")
        }
        ValidationErrorKind::MinProperties { limit } => {
            format!("use an object with at least {limit} properties")
        }
        ValidationErrorKind::MaxProperties { limit } => {
            format!("use an object with at most {limit} properties")
        }
        ValidationErrorKind::UniqueItems => "use an array with unique items".to_string(),
        ValidationErrorKind::AnyOf { .. } => {
            "match at least one declared anyOf alternative".to_string()
        }
        ValidationErrorKind::OneOfNotValid { .. } => {
            "match exactly one declared oneOf alternative".to_string()
        }
        ValidationErrorKind::OneOfMultipleValid { .. } => {
            "match only one declared oneOf alternative".to_string()
        }
        other => format!("satisfy the declared '{}' constraint", other.keyword()),
    }
}

fn bounded_json(value: &Value) -> String {
    bounded_text(
        &serde_json::to_string(value).unwrap_or_else(|_| "<constraint>".to_string()),
        180,
    )
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests;
