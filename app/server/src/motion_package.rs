//! Safe, bounded reads of editable ShellX Motion packages.
//!
//! The connector, tracking bridge, and project-state projection all consume the
//! same package control plane. Keeping those reads here avoids three subtly
//! different path-containment and size-limit implementations.

use cut_core::{error_codes, CutError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MOTION_PACKAGE_SOURCE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Stable, bounded revision for editable Motion package sources. Large rendered
/// assets are intentionally excluded: the authored manifest, motion document,
/// and optional template are the control-plane inputs that decide a rerender.
pub(crate) fn revision(path: &Path) -> Result<String, CutError> {
    let root = package_root(path)?;
    let manifest_path = root.join("manifest.json");
    let manifest_bytes = read_source_file(&manifest_path, "manifest")?;
    let manifest = parse_json(&manifest_bytes, "manifest")?;
    let mut inputs = vec![("manifest.json".to_string(), manifest_bytes)];
    for field in ["motion", "template"] {
        let Some(relative) = manifest.get(field).and_then(Value::as_str) else {
            if field == "motion" {
                return Err(CutError::new(
                    error_codes::INVALID_ARGS,
                    "Motion package manifest has no motion document",
                    "manifest.motion must name a package-relative JSON file",
                ));
            }
            continue;
        };
        let candidate = package_file(&root, relative, field)?;
        inputs.push((relative.to_string(), read_source_file(&candidate, field)?));
    }
    let mut digest = Sha256::new();
    for (name, bytes) in inputs {
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Return the package and authored-motion identities after guarded reads.
pub(crate) fn identity(path: &Path) -> Result<(String, String), CutError> {
    let manifest = manifest(path)?;
    let motion = document_from_manifest(path, &manifest)?;
    let package_id = required_id(&manifest, "manifest", "package")?;
    let motion_id = required_id(&motion, "motion.json", "motion document")?;
    Ok((package_id, motion_id))
}

/// Read the authored Motion document using the same bounded, contained package
/// contract as identity and revision checks.
pub(crate) fn document(path: &Path) -> Result<Value, CutError> {
    let manifest = manifest(path)?;
    document_from_manifest(path, &manifest)
}

pub(crate) fn duration_ms(path: &Path) -> Option<u64> {
    document(path)
        .ok()?
        .get("durationMs")
        .and_then(Value::as_u64)
}

fn manifest(path: &Path) -> Result<Value, CutError> {
    parse_json(
        &read_source_file(&path.join("manifest.json"), "manifest")?,
        "manifest",
    )
}

fn document_from_manifest(path: &Path, manifest: &Value) -> Result<Value, CutError> {
    let relative = manifest
        .get("motion")
        .and_then(Value::as_str)
        .unwrap_or("motion.json");
    let root = package_root(path)?;
    let motion_path = package_file(&root, relative, "motion")?;
    parse_json(
        &read_source_file(&motion_path, "motion document")?,
        "motion document",
    )
}

fn package_root(path: &Path) -> Result<PathBuf, CutError> {
    path.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "Motion package source was not found",
            format!("{}: {error}", path.display()),
        )
    })
}

fn package_file(root: &Path, relative: &str, label: &str) -> Result<PathBuf, CutError> {
    let candidate = root.join(relative).canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("Motion package {label} file was not found"),
            format!("{relative}: {error}"),
        )
    })?;
    if !candidate.starts_with(root) || !candidate.is_file() {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            format!("Motion package {label} path escapes its package"),
            relative,
        ));
    }
    Ok(candidate)
}

fn read_source_file(path: &Path, label: &str) -> Result<Vec<u8>, CutError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MOTION_PACKAGE_SOURCE_MAX_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("Motion package {label} is not a bounded regular file"),
            format!(
                "{} bytes; limit is {}",
                metadata.len(),
                MOTION_PACKAGE_SOURCE_MAX_BYTES
            ),
        ));
    }
    std::fs::read(path).map_err(Into::into)
}

fn parse_json(bytes: &[u8], label: &str) -> Result<Value, CutError> {
    serde_json::from_slice(bytes).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("Motion package {label} is invalid JSON"),
            error.to_string(),
        )
    })
}

fn required_id(value: &Value, file: &str, label: &str) -> Result<String, CutError> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("Motion package {file} has no identity"),
                format!("{label} id must be a non-empty string"),
            )
        })
}

#[cfg(test)]
mod tests;
