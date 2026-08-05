use super::contract::{
    bounded_label, safe_id, TrackingAsset, TrackingInventory, TrackingLayer,
    TrackingLifecycleSummary,
};
use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LIFECYCLES: usize = 128;

impl TrackingInventory {
    pub(crate) fn load(package_root: &Path) -> Result<Self, CutError> {
        let root = canonical_package_root(package_root)?;
        let manifest = read_json(&root.join("manifest.json"), "Motion manifest")?;
        let package_id = required_id(&manifest, "id", "manifest.id")?;
        let motion_ref = manifest
            .get("motion")
            .and_then(Value::as_str)
            .unwrap_or("motion.json");
        let motion = read_json(&resolve_package_file(&root, motion_ref)?, "Motion document")?;
        let motion_id = required_id(&motion, "id", "motion.id")?;
        let width = positive_u64(&motion, "width")?;
        let height = positive_u64(&motion, "height")?;
        let duration_ms = positive_u64(&motion, "durationMs")?;
        let fps = motion
            .get("fps")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| invalid("motion.fps must be a positive finite number"))?;
        let declared: BTreeSet<&str> = manifest
            .get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let video_assets = motion
            .get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|asset| tracking_asset(asset, &declared, &root))
            .collect();
        let target_layers = motion
            .get("layers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(tracking_layer)
            .collect();
        Ok(Self {
            package_id,
            motion_id,
            width,
            height,
            duration_ms,
            fps,
            video_assets,
            target_layers,
            analyses: lifecycle_summaries(&root)?,
        })
    }
}

fn tracking_asset(asset: &Value, declared: &BTreeSet<&str>, root: &Path) -> Option<TrackingAsset> {
    let kind = asset
        .get("kind")
        .or_else(|| asset.get("type"))
        .and_then(Value::as_str)?;
    if kind != "video" {
        return None;
    }
    let id = asset.get("id").and_then(Value::as_str)?;
    if !safe_id(id) {
        return None;
    }
    let source_ref = asset
        .get("source")
        .and_then(|source| source.get("path"))
        .and_then(Value::as_str);
    let available = source_ref.is_some_and(|reference| {
        declared.contains(reference) && resolve_package_file(root, reference).is_ok()
    });
    Some(TrackingAsset {
        id: id.to_string(),
        name: bounded_label(asset.get("name").and_then(Value::as_str).unwrap_or(id)),
        available,
    })
}

fn tracking_layer(layer: &Value) -> Option<TrackingLayer> {
    let id = layer.get("id").and_then(Value::as_str)?;
    let kind = layer
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("visual");
    if !safe_id(id) || kind == "audio" {
        return None;
    }
    Some(TrackingLayer {
        id: id.to_string(),
        name: bounded_label(layer.get("name").and_then(Value::as_str).unwrap_or(id)),
        kind: bounded_label(kind),
        tracking_attached: layer.get("x-tracking-stabilization").is_some(),
    })
}

fn lifecycle_summaries(root: &Path) -> Result<Vec<TrackingLifecycleSummary>, CutError> {
    let dir = root.join("analysis").join("tracking");
    let metadata = match std::fs::symlink_metadata(&dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(
            "tracking lifecycle directory must be a real package directory",
        ));
    }
    let mut summaries = Vec::new();
    for entry in std::fs::read_dir(&dir)?.take(MAX_LIFECYCLES + 1) {
        if summaries.len() == MAX_LIFECYCLES {
            return Err(invalid(
                "Motion package contains too many tracking lifecycles",
            ));
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(analysis_id) = name.strip_suffix(".lifecycle.json") else {
            continue;
        };
        if !safe_id(analysis_id) {
            continue;
        }
        let value = read_json(&entry.path(), "tracking lifecycle")?;
        let persisted_id = value
            .get("id")
            .or_else(|| value.get("analysisId"))
            .and_then(Value::as_str);
        if persisted_id != Some(analysis_id) {
            continue;
        }
        let state = value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let asset_id = value
            .get("requestedSource")
            .or_else(|| value.get("source"))
            .and_then(|source| source.get("assetId"))
            .and_then(Value::as_str)
            .filter(|id| safe_id(id))
            .map(str::to_string);
        summaries.push(TrackingLifecycleSummary {
            analysis_id: analysis_id.to_string(),
            state: bounded_label(state),
            asset_id,
        });
    }
    summaries.sort_by(|left, right| left.analysis_id.cmp(&right.analysis_id));
    Ok(summaries)
}

fn canonical_package_root(path: &Path) -> Result<PathBuf, CutError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "linked Motion package was not found",
            error.to_string(),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(
            "linked Motion package root must be a real directory, not a symlink",
        ));
    }
    Ok(path.canonicalize()?)
}

fn resolve_package_file(root: &Path, reference: &str) -> Result<PathBuf, CutError> {
    let joined = root.join(reference);
    let metadata = std::fs::symlink_metadata(&joined)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(
            "Motion package asset must be a regular non-symlink file",
        ));
    }
    let canonical = joined.canonicalize()?;
    if !canonical.starts_with(root) {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            "Motion package path escapes its package root",
            reference,
        ));
    }
    Ok(canonical)
}

fn read_json(path: &Path, label: &str) -> Result<Value, CutError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_JSON_BYTES {
        return Err(invalid(format!(
            "{label} must be a bounded regular non-symlink file"
        )));
    }
    serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} is invalid JSON"),
            error.to_string(),
        )
    })
}

fn required_id(value: &Value, key: &str, label: &str) -> Result<String, CutError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| safe_id(value))
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("{label} must be a safe identifier")))
}

fn positive_u64(value: &Value, key: &str) -> Result<u64, CutError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .ok_or_else(|| invalid(format!("motion.{key} must be a positive integer")))
}

fn invalid(detail: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::INVALID_ARGS,
        "Motion tracking request is invalid",
        detail,
    )
}
