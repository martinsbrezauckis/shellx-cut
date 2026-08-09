//! Independent, path-free comparison of an optional current Motion package
//! against the immutable lineage carried by an attested rendered artifact.

use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::path::{Component, Path};

#[path = "motion_package_lineage_io.rs"]
mod io;
use io::{canonical_root_file, read_hashed_file};

const MANIFEST_MAX_BYTES: u64 = 4 * 1024 * 1024;
const MOTION_MAX_BYTES: u64 = 64 * 1024 * 1024;
const SOURCE_MAX_BYTES: u64 = 64 * 1024 * 1024;
const RECEIPT_MAX_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) async fn current_package_lineage(
    package_dir: Option<&Path>,
    expected: Option<&Value>,
) -> Value {
    let Some(expected) = expected else {
        return unavailable("artifact-lineage-unavailable");
    };
    let Some(package_dir) = package_dir else {
        return unavailable("package-dir-not-provided");
    };
    let Ok(lineage) = derive(package_dir).await else {
        return unavailable("package-unreadable");
    };
    let changed_fields = changed_fields(expected, &lineage);
    serde_json::json!({
        "schema": "shellx-cut/current-motion-package-lineage@1",
        "status": if changed_fields.is_empty() { "exact" } else { "changed" },
        "lineage": lineage,
        "changedFields": changed_fields,
        "reason": Value::Null,
    })
}

fn unavailable(reason: &str) -> Value {
    serde_json::json!({
        "schema": "shellx-cut/current-motion-package-lineage@1",
        "status": "unavailable",
        "lineage": Value::Null,
        "changedFields": [],
        "reason": reason,
    })
}

fn changed_fields(expected: &Value, current: &Value) -> Vec<&'static str> {
    [
        "manifestSha256",
        "motionSha256",
        "adapterId",
        "sourceSha256",
        "normalizedSourceSha256",
        "loweringReceiptSha256",
    ]
    .into_iter()
    .filter(|field| expected.get(*field) != current.get(*field))
    .collect()
}

/// Derive the same bounded package-owned hash projection emitted by the current
/// Motion SDK. Failure is converted to `unavailable` by the caller because this
/// comparison cannot invalidate an already attested artifact.
async fn derive(package_dir: &Path) -> Result<Value, CutError> {
    let root = tokio::fs::canonicalize(package_dir)
        .await
        .map_err(|error| lineage_error("canonicalize current Motion package", error.to_string()))?;
    let root_metadata = tokio::fs::metadata(&root)
        .await
        .map_err(|error| lineage_error("inspect current Motion package", error.to_string()))?;
    if !root_metadata.is_dir() {
        return Err(lineage_error(
            "current Motion package root is not a directory",
            "select the package directory containing manifest.json",
        ));
    }

    let manifest_path =
        canonical_root_file(&root, "manifest.json", "current package manifest").await?;
    let manifest_file = read_hashed_file(
        &manifest_path,
        MANIFEST_MAX_BYTES,
        MANIFEST_MAX_BYTES as usize,
        "current package manifest",
    )
    .await?;
    let manifest: Value = serde_json::from_slice(&manifest_file.bytes).map_err(|error| {
        lineage_error(
            "current Motion package manifest is invalid JSON",
            error.to_string(),
        )
    })?;
    require_string(&manifest, "schema", "shellx-motion/package-manifest@1")?;
    required_string(&manifest, "id")?;

    let motion_relative =
        required_package_relative(&manifest, "motion", "current package Motion document")?;
    let motion_path =
        canonical_root_file(&root, &motion_relative, "current package Motion document").await?;
    let motion_file = read_hashed_file(
        &motion_path,
        MOTION_MAX_BYTES,
        MOTION_MAX_BYTES as usize,
        "current package Motion document",
    )
    .await?;
    let motion: Value = serde_json::from_slice(&motion_file.bytes).map_err(|error| {
        lineage_error(
            "current Motion package document is invalid JSON",
            error.to_string(),
        )
    })?;
    require_string(&motion, "schema", "shellx-motion/motion@1")?;
    required_string(&motion, "id")?;

    let mut lineage = serde_json::json!({
        "schema": "shellx-motion/package-render-lineage@1",
        "manifestSha256": manifest_file.sha256,
        "motionSha256": motion_file.sha256,
    });
    let adapter = manifest
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("adapter"))
        .filter(|adapter| adapter.is_object());
    if adapter
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        != Some("adapter.gltf")
    {
        return Ok(lineage);
    }
    let adapter = adapter.expect("glTF adapter was checked");
    let source_relative = required_package_relative(adapter, "source", "glTF preserved source")?;
    let normalized_relative =
        required_package_relative(adapter, "loweringSource", "glTF normalized source")?;
    let receipt_relative =
        required_package_relative(adapter, "loweringReceipt", "glTF lowering receipt")?;
    let declared_source = required_sha256(adapter, "sourceSha256")?;
    let declared_normalized = required_sha256(adapter, "loweringSourceSha256")?;
    let source_path =
        canonical_root_file(&root, &source_relative, "current package glTF source").await?;
    let normalized_path = canonical_root_file(
        &root,
        &normalized_relative,
        "current package normalized glTF source",
    )
    .await?;
    let receipt_path = canonical_root_file(
        &root,
        &receipt_relative,
        "current package glTF lowering receipt",
    )
    .await?;
    let (source_file, normalized_file, receipt_file) = tokio::try_join!(
        read_hashed_file(
            &source_path,
            SOURCE_MAX_BYTES,
            0,
            "current package glTF source"
        ),
        read_hashed_file(
            &normalized_path,
            SOURCE_MAX_BYTES,
            0,
            "current package normalized glTF source"
        ),
        read_hashed_file(
            &receipt_path,
            RECEIPT_MAX_BYTES,
            RECEIPT_MAX_BYTES as usize,
            "current package glTF lowering receipt"
        ),
    )?;
    if source_file.sha256 != declared_source || normalized_file.sha256 != declared_normalized {
        return Err(lineage_error(
            "current Motion package glTF hashes do not match manifest provenance",
            "regenerate the package from its preserved source",
        ));
    }
    let receipt: Value = serde_json::from_slice(&receipt_file.bytes).map_err(|error| {
        lineage_error(
            "current Motion package glTF receipt is invalid JSON",
            error.to_string(),
        )
    })?;
    require_string(&receipt, "schema", "shellx-motion/receipt@1")?;
    require_string(&receipt, "operation", "adapter.lower")?;
    require_string(&receipt, "lane", "adapter")?;
    required_success_status(&receipt, "status")?;

    lineage["adapterId"] = Value::from("adapter.gltf");
    lineage["sourceSha256"] = Value::from(source_file.sha256);
    lineage["normalizedSourceSha256"] = Value::from(normalized_file.sha256);
    lineage["loweringReceiptSha256"] = Value::from(receipt_file.sha256);
    Ok(lineage)
}

fn required_package_relative(value: &Value, field: &str, label: &str) -> Result<String, CutError> {
    let raw = required_string(value, field)?;
    let path = Path::new(&raw);
    if raw.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(lineage_error(
            format!("{label} path is not canonical and package-relative"),
            raw,
        ));
    }
    Ok(raw)
}

fn required_sha256(value: &Value, field: &str) -> Result<String, CutError> {
    let hash = required_string(value, field)?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(lineage_error(
            format!("current Motion package {field} is not a lowercase sha256"),
            hash,
        ));
    }
    Ok(hash)
}

fn required_string(value: &Value, field: &str) -> Result<String, CutError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            lineage_error(
                format!("current Motion package is missing '{field}'"),
                "expected a non-empty string",
            )
        })
}

fn require_string(value: &Value, field: &str, expected: &str) -> Result<(), CutError> {
    let actual = required_string(value, field)?;
    if actual != expected {
        return Err(lineage_error(
            format!("current Motion package field '{field}' does not match"),
            format!("expected '{expected}', got '{actual}'"),
        ));
    }
    Ok(())
}

fn required_success_status(value: &Value, field: &str) -> Result<String, CutError> {
    let status = required_string(value, field)?;
    if status != "passed" && status != "warning" {
        return Err(lineage_error(
            "current Motion package receipt is not successful",
            status,
        ));
    }
    Ok(status)
}

fn lineage_error(message: impl Into<String>, cause: impl Into<String>) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, cause)
        .with_suggested_action("Select the unchanged Motion package used for this render")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unavailable_reports_do_not_claim_legacy_or_missing_packages_match() {
        let legacy = current_package_lineage(None, None).await;
        assert_eq!(legacy["status"], serde_json::json!("unavailable"));
        assert_eq!(
            legacy["reason"],
            serde_json::json!("artifact-lineage-unavailable")
        );
        let expected = serde_json::json!({
            "schema":"shellx-motion/package-render-lineage@1",
            "manifestSha256":"a".repeat(64),
            "motionSha256":"b".repeat(64),
        });
        let omitted = current_package_lineage(None, Some(&expected)).await;
        assert_eq!(omitted["status"], serde_json::json!("unavailable"));
        assert_eq!(
            omitted["reason"],
            serde_json::json!("package-dir-not-provided")
        );
    }

    #[test]
    fn changed_fields_are_stable_for_missing_gltf_provenance() {
        let expected = serde_json::json!({
            "manifestSha256":"a",
            "motionSha256":"b",
            "adapterId":"adapter.gltf",
            "sourceSha256":"c",
            "normalizedSourceSha256":"d",
            "loweringReceiptSha256":"e",
        });
        let current = serde_json::json!({"manifestSha256":"a", "motionSha256":"b"});
        assert_eq!(
            changed_fields(&expected, &current),
            [
                "adapterId",
                "sourceSha256",
                "normalizedSourceSha256",
                "loweringReceiptSha256",
            ]
        );
    }
}
