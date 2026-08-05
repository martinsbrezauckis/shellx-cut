//! Replay-backed, machine-local projection of linked Motion clips.
//!
//! Link receipts stay durable in the operation log. Availability and authored
//! effect summaries are deliberately transient because package paths belong to
//! the current workstation, not the portable Cut project cache.

use cut_core::OpRecord;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_VISIBLE_EFFECT_LAYERS: usize = 64;
const MAX_VISIBLE_LABEL_CHARS: usize = 160;

pub(super) fn annotate_project_state(project: &mut Value, ops: &[OpRecord]) {
    let links = clip_links(ops);
    if links.is_empty() {
        return;
    }
    let Some(tracks) = project.get_mut("tracks").and_then(Value::as_array_mut) else {
        return;
    };
    for clip in tracks
        .iter_mut()
        .filter_map(|track| track.get_mut("clips").and_then(Value::as_array_mut))
        .flatten()
    {
        let Some(clip_id) = clip.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(link) = links.get(clip_id) else {
            continue;
        };
        if let Some(object) = clip.as_object_mut() {
            object.insert("motion_link".into(), revalidate_link(link.clone()));
        }
    }
}

fn clip_links(ops: &[OpRecord]) -> BTreeMap<String, Value> {
    let mut links = BTreeMap::new();
    for op in ops.iter().filter(|op| is_link_verb(&op.verb)) {
        for detail in op.effects.iter().map(|effect| &effect.detail) {
            let Some(items) = detail.get("motion_links").and_then(Value::as_array) else {
                continue;
            };
            for item in items.iter().filter(|item| {
                item.get("schema").and_then(Value::as_str) == Some("shellx-cut/motion-link@1")
            }) {
                if let Some(clip_id) = item.get("clipId").and_then(Value::as_str) {
                    links.insert(clip_id.to_string(), item.clone());
                }
            }
        }
    }
    links
}

fn is_link_verb(verb: &str) -> bool {
    matches!(
        verb,
        "motion.apply_import"
            | "motion.link.update"
            | "motion.link.refresh"
            | "motion.link.relink"
            | "motion.link.tracking.request"
            | "motion.link.tracking.apply"
            | "motion.link.tracking.detach"
    )
}

fn revalidate_link(mut link: Value) -> Value {
    let source_path = path_field(&link, "sourcePath");
    let plan_path = path_field(&link, "planPath");
    let render_path = link
        .pointer("/render/path")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let fallback_path = path_field(&link, "fallbackPath");
    let source_available = source_path
        .as_ref()
        .is_some_and(|path| path.is_dir() && path.join("manifest.json").is_file());
    let plan_available = is_file(&plan_path);
    let render_available = is_file(&render_path);
    let fallback_available = is_file(&fallback_path);
    let source_dirty = source_is_dirty(&link, source_path.as_deref(), plan_path.as_deref());
    let state = link_state(
        &link,
        source_available,
        source_dirty,
        render_available,
        fallback_available,
    );
    if let Some(object) = link.as_object_mut() {
        object.insert("state".into(), json!(state));
        object.insert(
            "availability".into(),
            json!({
                "source": source_available,
                "plan": plan_available,
                "render": render_available,
                "fallback": fallback_available,
                "canRefresh": source_available,
                "canRelink": true,
                "canEditInMotion": source_available && crate::motion_bridge::canvas_available(),
            }),
        );
        if source_available {
            object.insert(
                "effects".into(),
                effect_summary(source_path.as_deref().expect("available source has path")),
            );
        }
    }
    link
}

fn source_is_dirty(link: &Value, source: Option<&Path>, plan: Option<&Path>) -> bool {
    let expected = link.get("sourceRevision").and_then(Value::as_str);
    let actual = match link
        .get("sourceRevisionKind")
        .and_then(Value::as_str)
        .unwrap_or("cut-import-plan")
    {
        "cut-import-plan" => plan.and_then(hash_small_plan),
        "motion-package" => source.and_then(|path| crate::motion_package::revision(path).ok()),
        _ => None,
    };
    actual
        .zip(expected)
        .is_some_and(|(actual, expected)| actual != expected)
}

fn hash_small_plan(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .filter(|bytes| bytes.len() as u64 <= 4 * 1024 * 1024)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

fn link_state(
    link: &Value,
    source_available: bool,
    source_dirty: bool,
    render_available: bool,
    fallback_available: bool,
) -> String {
    if !source_available {
        return "missing-source".into();
    }
    if source_dirty {
        return "source-dirty".into();
    }
    if !render_available && !fallback_available {
        return "render-error".into();
    }
    link.get("state")
        .and_then(Value::as_str)
        .filter(|state| {
            !matches!(
                *state,
                "missing-source" | "source-dirty" | "render-error" | "rendering" | "relinking"
            )
        })
        .unwrap_or("linked-current")
        .to_string()
}

fn effect_summary(package: &Path) -> Value {
    let Ok(document) = crate::motion_package::document(package) else {
        return json!({
            "schema": "shellx-cut/motion-effects-summary@1",
            "available": false,
            "ownership": "motion",
            "editableInCut": false,
            "reason": "unreadable-motion-document",
        });
    };
    let layers = document
        .get("layers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut keyed = 0_u64;
    let mut roto = 0_u64;
    let mut tracked = 0_u64;
    let mut affected = 0_u64;
    let mut visible = Vec::new();
    for layer in layers {
        let keying = valid_keying(layer);
        let mask = valid_roto(layer);
        if keying.is_none() && mask.is_none() {
            continue;
        }
        affected += 1;
        keyed += u64::from(keying.is_some());
        roto += u64::from(mask.is_some());
        tracked += u64::from(
            mask.and_then(|value| value.get("tracking"))
                .and_then(Value::as_object)
                .is_some(),
        );
        if visible.len() < MAX_VISIBLE_EFFECT_LAYERS {
            visible.push(visible_effect_layer(layer, keying, mask));
        }
    }
    json!({
        "schema": "shellx-cut/motion-effects-summary@1",
        "available": true,
        "ownership": "motion",
        "editableInCut": false,
        "keyedLayerCount": keyed,
        "rotoLayerCount": roto,
        "trackedRotoLayerCount": tracked,
        "truncated": affected as usize > visible.len(),
        "layers": visible,
    })
}

fn valid_keying(layer: &Value) -> Option<&Value> {
    let value = layer.get("keying")?;
    (matches!(
        layer.get("type").and_then(Value::as_str),
        Some("image" | "video")
    ) && value.get("schema").and_then(Value::as_str) == Some("shellx-motion/chroma-key@1"))
    .then_some(value)
}

fn valid_roto(layer: &Value) -> Option<&Value> {
    let value = layer.get("mask")?;
    (matches!(
        layer.get("type").and_then(Value::as_str),
        Some("image" | "video")
    ) && value.get("type").and_then(Value::as_str) == Some("roto")
        && value.get("schema").and_then(Value::as_str) == Some("shellx-motion/roto-mask@1"))
    .then_some(value)
}

fn visible_effect_layer(layer: &Value, keying: Option<&Value>, mask: Option<&Value>) -> Value {
    let mut summary = serde_json::Map::new();
    summary.insert("id".into(), json!(short_text(layer.get("id"))));
    summary.insert("name".into(), json!(short_text(layer.get("name"))));
    summary.insert("type".into(), json!(short_text(layer.get("type"))));
    if let Some(keying) = keying {
        summary.insert(
            "keying".into(),
            json!({
                "keyColor": supported_color(keying.get("keyColor")),
                "spillSuppression": unit_number(keying.get("spillSuppression")),
                "matteCleanup": keying.get("matte").and_then(Value::as_object).is_some_and(|value| !value.is_empty()),
            }),
        );
    }
    if let Some(mask) = mask {
        let tracking = mask.get("tracking").and_then(Value::as_object);
        summary.insert(
            "roto".into(),
            json!({
                "frameCount": mask.get("frames").and_then(Value::as_array).map_or(0, Vec::len),
                "tracked": tracking.is_some(),
                "model": tracking.and_then(|value| value.get("model")).and_then(Value::as_str).filter(|value| matches!(*value, "translation" | "similarity" | "homography")),
            }),
        );
    }
    Value::Object(summary)
}

fn short_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.chars().take(MAX_VISIBLE_LABEL_CHARS).collect())
}

fn supported_color(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|value| {
        value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn unit_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| (0.0..=1.0).contains(value))
}

fn path_field(link: &Value, name: &str) -> Option<PathBuf> {
    link.get(name).and_then(Value::as_str).map(PathBuf::from)
}

fn is_file(path: &Option<PathBuf>) -> bool {
    path.as_ref().is_some_and(|path| path.is_file())
}

#[cfg(test)]
mod tests;
