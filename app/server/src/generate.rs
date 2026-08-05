//! generate.rs - Generate template catalog and normalized Generate IR.

use crate::registry::VerbRegistry;
use cut_core::{error_codes, CutError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub const GENERATE_TEMPLATES_JSON: &str = include_str!("../../../schema/generate_templates.json");
pub const GENERATE_RICH_MOTION_TEMPLATES_JSON: &str =
    include_str!("../../../schema/generate_templates_motion_rich.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerateCatalog {
    pub schema: String,
    pub templates: Vec<GenerateTemplate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerateTemplate {
    pub id: String,
    pub source: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub params: BTreeMap<String, GenerateParam>,
    #[serde(default)]
    pub defaults: Value,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub lowering: GenerateLowering,
    #[serde(default)]
    pub verification: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerateParam {
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "enum")]
    pub allowed: Option<Vec<Value>>,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerateLowering {
    pub verb: String,
    #[serde(default)]
    pub args: Value,
}

impl GenerateCatalog {
    pub fn get(&self, id: &str) -> Option<&GenerateTemplate> {
        self.templates.iter().find(|t| t.id == id)
    }
}

pub fn registry() -> &'static GenerateCatalog {
    static REG: OnceLock<GenerateCatalog> = OnceLock::new();
    REG.get_or_init(|| {
        let mut catalog: GenerateCatalog = serde_json::from_str(GENERATE_TEMPLATES_JSON)
            .expect("schema/generate_templates.json must parse");
        let rich: GenerateCatalog = serde_json::from_str(GENERATE_RICH_MOTION_TEMPLATES_JSON)
            .expect("schema/generate_templates_motion_rich.json must parse");
        assert_eq!(
            catalog.schema, rich.schema,
            "Generate catalog fragments must share one schema"
        );
        catalog.templates.extend(rich.templates);
        validate_catalog_contract(&catalog);
        catalog
    })
}

pub fn referenced_params(args: &Value, out: &mut BTreeSet<String>) {
    match args {
        Value::String(s) => {
            if let Some(inner) = s.strip_prefix("{{").and_then(|x| x.strip_suffix("}}")) {
                let name = inner.trim();
                if !name.is_empty() && !is_generate_special_placeholder(name) {
                    out.insert(name.to_string());
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| referenced_params(v, out)),
        Value::Object(m) => m.values().for_each(|v| referenced_params(v, out)),
        _ => {}
    }
}

pub fn resolve_params(
    template: &GenerateTemplate,
    overrides: &Map<String, Value>,
) -> Result<BTreeMap<String, Value>, CutError> {
    for name in overrides.keys() {
        if !template.params.contains_key(name) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown generate parameter '{name}'"),
                format!("template '{}' does not declare '{name}'", template.id),
            ));
        }
    }

    let mut resolved = BTreeMap::new();
    for (name, param) in &template.params {
        let value = if let Some(v) = overrides.get(name) {
            v.clone()
        } else if let Some(v) = param.default.clone() {
            v
        } else if param.required {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("missing required generate parameter '{name}'"),
                format!("template '{}' requires '{name}'", template.id),
            ));
        } else {
            continue;
        };
        validate_param_value(&template.id, name, param, &value)?;
        resolved.insert(name.clone(), value);
    }
    Ok(resolved)
}

pub fn resolve_duration_ms(template: &GenerateTemplate, params: &BTreeMap<String, Value>) -> u64 {
    let raw = params
        .get("duration_ms")
        .or_else(|| template.defaults.get("duration_ms"));
    raw.and_then(|v| v.as_i64())
        .unwrap_or(3_000)
        .clamp(250, 30_000) as u64
}

pub fn interpolate_args(
    template: &GenerateTemplate,
    params: &BTreeMap<String, Value>,
    range_ms: [u64; 2],
) -> Result<Value, CutError> {
    interpolate_value(&template.id, &template.lowering.args, params, range_ms)
}

fn validate_param_value(
    template_id: &str,
    name: &str,
    param: &GenerateParam,
    value: &Value,
) -> Result<(), CutError> {
    let type_ok = match param.ty.as_str() {
        "string" => value.as_str().is_some(),
        "integer" => value.as_i64().is_some(),
        "number" => value.as_f64().is_some_and(f64::is_finite),
        "boolean" => value.as_bool().is_some(),
        "color" => value.as_str().map(is_generate_color).unwrap_or(false),
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unsupported generate parameter type '{other}'"),
                format!("template '{template_id}' parameter '{name}' uses an unsupported type"),
            ))
        }
    };
    if !type_ok {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid value for generate parameter '{name}'"),
            format!(
                "template '{template_id}' expects '{name}' to be type {}",
                param.ty
            ),
        ));
    }
    if let Some(allowed) = &param.allowed {
        if !allowed.iter().any(|v| v == value) {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid enum value for generate parameter '{name}'"),
                format!("template '{template_id}' parameter '{name}' must be one of {allowed:?}"),
            ));
        }
    }
    if matches!(param.ty.as_str(), "integer" | "number") {
        let number = value.as_f64().expect("numeric type was validated");
        if param.minimum.is_some_and(|minimum| number < minimum)
            || param.maximum.is_some_and(|maximum| number > maximum)
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("generate parameter '{name}' is outside its allowed range"),
                format!(
                    "template '{template_id}' expects '{name}' between {:?} and {:?}",
                    param.minimum, param.maximum
                ),
            ));
        }
    }
    Ok(())
}

fn is_generate_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn placeholder_name(s: &str) -> Option<&str> {
    s.strip_prefix("{{")
        .and_then(|x| x.strip_suffix("}}"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn is_generate_special_placeholder(name: &str) -> bool {
    matches!(
        name,
        "range_ms" | "half_duration_ms" | "quarter_duration_ms"
    )
}

fn interpolate_value(
    template_id: &str,
    value: &Value,
    params: &BTreeMap<String, Value>,
    range_ms: [u64; 2],
) -> Result<Value, CutError> {
    match value {
        Value::String(s) => {
            if let Some(name) = placeholder_name(s) {
                if name == "range_ms" {
                    return Ok(json!(range_ms));
                }
                if name == "half_duration_ms" {
                    let duration = range_ms[1].saturating_sub(range_ms[0]).max(2);
                    return Ok(json!(duration.div_ceil(2)));
                }
                if name == "quarter_duration_ms" {
                    let duration = range_ms[1].saturating_sub(range_ms[0]).max(4);
                    return Ok(json!(duration.div_ceil(4)));
                }
                return params.get(name).cloned().ok_or_else(|| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        format!("generate template '{template_id}' references unresolved parameter '{name}'"),
                        "template params must resolve before lowering args are materialized",
                    )
                });
            }
            if s.contains("{{") || s.contains("}}") {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("generate template '{template_id}' uses an embedded placeholder"),
                    format!(
                        "embedded placeholder strings are not supported in template preview: '{s}'"
                    ),
                ));
            }
            Ok(value.clone())
        }
        Value::Array(values) => values
            .iter()
            .map(|v| interpolate_value(template_id, v, params, range_ms))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(
                    k.clone(),
                    interpolate_value(template_id, v, params, range_ms)?,
                );
            }
            Ok(Value::Object(out))
        }
        _ => Ok(value.clone()),
    }
}

fn validate_catalog_contract(catalog: &GenerateCatalog) {
    assert_eq!(
        catalog.schema, "shellx-cut/generate-templates/1",
        "schema/generate_templates.json has an unexpected schema"
    );
    let verbs = VerbRegistry::load();
    let mut ids = BTreeSet::new();
    for template in &catalog.templates {
        assert!(
            ids.insert(template.id.clone()),
            "duplicate generate template id {}",
            template.id
        );
        assert!(
            verbs.get(&template.lowering.verb).is_some(),
            "generate template {} lowers to unknown verb {}",
            template.id,
            template.lowering.verb
        );
        for (name, param) in &template.params {
            assert!(
                matches!(
                    param.ty.as_str(),
                    "string" | "integer" | "number" | "boolean" | "color"
                ),
                "generate template {} parameter {} uses unsupported type {}",
                template.id,
                name,
                param.ty
            );
            assert!(
                param.minimum.is_none_or(f64::is_finite),
                "{}:{name} minimum must be finite",
                template.id
            );
            assert!(
                param.maximum.is_none_or(f64::is_finite),
                "{}:{name} maximum must be finite",
                template.id
            );
            assert!(
                param.step.is_none_or(|step| step.is_finite() && step > 0.0),
                "{}:{name} step must be positive",
                template.id
            );
            assert!(
                !matches!((param.minimum, param.maximum), (Some(min), Some(max)) if min > max),
                "{}:{name} minimum must not exceed maximum",
                template.id
            );
        }
        let mut refs = BTreeSet::new();
        referenced_params(&template.lowering.args, &mut refs);
        for name in refs {
            assert!(
                template.params.contains_key(&name),
                "generate template {} references undeclared param {}",
                template.id,
                name
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn generate_catalog_parses_and_has_v1_templates() {
        let reg = registry();
        assert_eq!(reg.schema, "shellx-cut/generate-templates/1");
        assert!(
            reg.templates.len() >= 8,
            "Built-in templates cover titles, captions, shapes, social cards, batch, and record handoff"
        );
        assert!(reg.get("builtin.lower-third.clean").is_some());
        assert!(reg.get("builtin.motion.lower-third").is_some());
        assert!(reg.get("builtin.caption.kinetic-yellow").is_some());
        assert!(reg.get("builtin.batch.quote-card").is_some());
    }

    #[test]
    fn generate_catalog_exposes_motion_template_to_cut_template() {
        let template = registry()
            .get("builtin.motion.lower-third")
            .expect("Motion lower-third template should be discoverable in Generate");
        assert_eq!(template.kind, "motion");
        assert_eq!(template.lowering.verb, "motion.template_to_cut");
        assert!(
            template.capabilities.iter().any(|cap| cap == "preview")
                && template.capabilities.iter().any(|cap| cap == "insert")
                && template
                    .capabilities
                    .iter()
                    .any(|cap| cap == "rendered_media"),
            "Motion template should advertise preview, insert, and rendered_media capabilities"
        );
        assert!(template.params.contains_key("title"));
        assert!(template.params.contains_key("subtitle"));
        assert!(template.params.contains_key("accentColor"));
    }

    #[test]
    fn generate_catalog_exposes_motion_scripted_video_template() {
        let template = registry()
            .get("builtin.motion.scripted-video")
            .expect("Motion scripted-video template should be discoverable in Generate");
        assert_eq!(template.kind, "motion");
        assert_eq!(template.lowering.verb, "motion.script_to_cut");
        assert!(
            template.capabilities.iter().any(|cap| cap == "preview")
                && template.capabilities.iter().any(|cap| cap == "insert")
                && template
                    .capabilities
                    .iter()
                    .any(|cap| cap == "scripted_video")
                && template
                    .capabilities
                    .iter()
                    .any(|cap| cap == "rendered_media"),
            "Motion scripted-video template should advertise preview, insert, scripted_video, and rendered_media capabilities"
        );
        assert_eq!(
            template
                .lowering
                .args
                .get("script")
                .and_then(|script| script.get("schema"))
                .and_then(|schema| schema.as_str()),
            Some("shellx-motion/scripted-video@1")
        );
        let frames = template
            .lowering
            .args
            .get("script")
            .and_then(|script| script.get("frames"))
            .and_then(|frames| frames.as_array())
            .expect("Motion scripted-video template should lower to scripted frames");
        assert!(
            frames.len() >= 2,
            "Motion scripted-video Generate inserts need at least two frames for Motion's real-render uniqueness gate"
        );
        let mut requested = Map::new();
        requested.insert("title".to_string(), json!("Generate in Cut"));
        requested.insert("duration_ms".to_string(), json!(3000));
        let params = resolve_params(template, &requested).expect("scripted-video params resolve");
        let args = interpolate_args(template, &params, [0, 3000])
            .expect("scripted-video lowering args interpolate");
        let resolved_frames = args
            .get("script")
            .and_then(|script| script.get("frames"))
            .and_then(|frames| frames.as_array())
            .expect("interpolated scripted-video frames");
        assert_eq!(resolved_frames.len(), 4);
        for frame in resolved_frames {
            assert_eq!(frame["durationMs"], json!(750));
        }
        assert_eq!(args["script"]["width"], json!(1920));
        assert_eq!(args["script"]["height"], json!(1080));
        assert!(template.params.contains_key("title"));
        assert!(template.params.contains_key("body"));
        assert!(template.params.contains_key("caption"));
        assert!(template.params.contains_key("width"));
        assert!(template.params.contains_key("height"));
    }

    #[test]
    fn generate_catalog_ids_are_unique_and_params_are_referenced_safely() {
        let reg = registry();
        let mut ids = BTreeSet::new();
        for t in &reg.templates {
            assert!(ids.insert(t.id.clone()), "duplicate template id {}", t.id);
            let mut refs = BTreeSet::new();
            referenced_params(&t.lowering.args, &mut refs);
            for name in refs {
                assert!(
                    t.params.contains_key(&name),
                    "template {} references undeclared param {}",
                    t.id,
                    name
                );
            }
        }
    }

    #[test]
    fn generate_catalog_lowering_verbs_exist() {
        let verbs = VerbRegistry::load();
        for t in &registry().templates {
            assert!(
                verbs.get(&t.lowering.verb).is_some(),
                "template {} lowers to unknown verb {}",
                t.id,
                t.lowering.verb
            );
        }
    }

    #[test]
    fn generate_params_resolve_defaults_and_interpolate_range() {
        let t = registry().get("builtin.lower-third.clean").unwrap();
        let overrides =
            serde_json::from_value::<Map<String, Value>>(json!({"name":"Ada"})).unwrap();
        let params = resolve_params(t, &overrides).expect("params resolve");
        assert_eq!(params["name"], json!("Ada"));
        assert_eq!(params["accent"], json!("#FFD24A"));
        assert_eq!(resolve_duration_ms(t, &params), 4_000);
        let args = interpolate_args(t, &params, [0, 4_000]).expect("args interpolate");
        assert_eq!(args["text"], json!("Ada"));
        assert_eq!(args["accent"], json!("#FFD24A"));
        assert_eq!(args["range_ms"], json!([0, 4_000]));
    }

    #[test]
    fn generate_params_reject_missing_required_before_interpolation() {
        let t = registry().get("builtin.lower-third.clean").unwrap();
        let err = resolve_params(t, &Map::new()).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains("name"));
    }

    #[test]
    fn generate_params_reject_unknown_bad_color_and_bad_enum() {
        let t = registry().get("builtin.lower-third.clean").unwrap();
        let unknown =
            serde_json::from_value::<Map<String, Value>>(json!({"name":"Ada","bogus":true}))
                .unwrap();
        let err = resolve_params(t, &unknown).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains("bogus"));

        let bad_color =
            serde_json::from_value::<Map<String, Value>>(json!({"name":"Ada","accent":"gold"}))
                .unwrap();
        let err = resolve_params(t, &bad_color).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.cause.contains("color"));

        let t = registry().get("builtin.caption.kinetic-yellow").unwrap();
        let bad_enum =
            serde_json::from_value::<Map<String, Value>>(json!({"position":"left"})).unwrap();
        let err = resolve_params(t, &bad_enum).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.message.contains("position"));
    }
}
