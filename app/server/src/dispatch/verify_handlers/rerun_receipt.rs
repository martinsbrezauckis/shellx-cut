//! Receipt identity, output hashing, and verification-receipt persistence.

use super::*;
use crate::output_paths::write_output_atomic;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// Load the receipt selected by the caller without following a leaf link or
/// accepting a file outside the project's own receipt directory. The embedded
/// id is a second identity fence: a named path cannot stand in for another
/// render's evidence.
#[cfg(test)]
pub(super) fn selected_render_receipt(
    receipts: &Path,
    requested_id: &str,
) -> Result<cut_core::RenderReceipt, CutError> {
    selected_render_receipt_with_path(receipts, requested_id).map(|(_, receipt)| receipt)
}

pub(super) fn selected_render_receipt_with_path(
    receipts: &Path,
    requested_id: &str,
) -> Result<(PathBuf, cut_core::RenderReceipt), CutError> {
    // Persisted retry descriptors bypass the public render_id schema. Reject
    // path syntax before any ID-derived lookup, including Windows UNC paths.
    validate_receipt_id(requested_id)?;
    plain_receipt_dir(receipts)?;
    // Keep an explicitly selected symlink distinguishable from a missing
    // receipt. `resolve_receipt_path` intentionally rejects non-regular leaves
    // early for general verify callers; rerun needs the stronger conflict
    // classification so its caller can explain the unsafe receipt identity.
    let selected = receipts.join(format!("{requested_id}.json"));
    if std::fs::symlink_metadata(&selected).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "selected render receipt is not a local regular file in the receipt directory",
            format!("refusing unsafe receipt path {}", selected.display()),
        )
        .with_suggested_action("render again to create a fresh local render receipt"));
    }
    let path = resolve_receipt_path(receipts, Some(requested_id))?;
    let receipts_dir = receipts.canonicalize().map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the project receipt directory",
            error.to_string(),
        )
    })?;
    let parent = path.parent().and_then(|parent| parent.canonicalize().ok());
    let plain = record_recovery::is_plain_regular_file(&path).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the selected render receipt",
            error.to_string(),
        )
    })?;
    if parent.as_deref() != Some(receipts_dir.as_path()) || !plain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "selected render receipt is not a local regular file in the receipt directory",
            format!("refusing unsafe receipt path {}", path.display()),
        )
        .with_suggested_action("render again to create a fresh local render receipt"));
    }
    let receipt: cut_core::RenderReceipt = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    if receipt.render_id != requested_id {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "selected render receipt id does not match its file identity",
            format!(
                "requested {requested_id}; receipt embeds {}",
                receipt.render_id
            ),
        )
        .with_suggested_action("select the matching render receipt or render again"));
    }
    Ok((path, receipt))
}

pub(super) fn plain_receipt_dir(receipts: &Path) -> Result<(), CutError> {
    let plain = record_recovery::is_plain_dir(receipts).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the project receipt directory",
            error.to_string(),
        )
    })?;
    if !plain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "project receipt directory is not a local directory",
            format!("refusing unsafe receipt directory {}", receipts.display()),
        )
        .with_suggested_action("render again to create receipts in the local project directory"));
    }
    Ok(())
}

pub(super) fn verification_receipt_name(job_id: &str) -> String {
    format!("verify_rerun_{job_id}.json")
}

pub(super) fn write_verification_receipt(
    receipts: &Path,
    name: &str,
    result: &Value,
) -> Result<(), CutError> {
    if name.starts_with("render_") {
        return Err(CutError::new(
            error_codes::IO,
            "could not allocate a separate verification receipt",
            "the render receipt is immutable",
        ));
    }
    plain_receipt_dir(receipts)?;
    let path = receipts.join(name);
    write_output_atomic(
        &path,
        serde_json::to_vec_pretty(result).map_err(CutError::from)?,
    )
}

pub(super) fn profile_from_receipt(
    receipt: &cut_core::RenderReceipt,
) -> Result<cut_perception::FootageProfile, CutError> {
    let Some(entry) = receipt
        .checks
        .iter()
        .find(|check| check.name == cut_core::check_names::FOOTAGE_PROFILE)
    else {
        return Ok(cut_perception::FootageProfile::TalkingHead);
    };
    let Some(active) = entry.details.get("active_profile").and_then(Value::as_str) else {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "render receipt has invalid footage-profile evidence",
            "the footage_profile entry is missing details.active_profile",
        )
        .with_suggested_action("render again to create a complete receipt"));
    };
    active.parse().map_err(|reason: String| {
        CutError::new(
            error_codes::CONFLICT,
            "render receipt has an unknown footage profile",
            reason,
        )
        .with_suggested_action("render again with a supported footage profile")
    })
}

pub(super) fn assert_receipt_hash(path: &Path, expected: &str) -> Result<(), CutError> {
    let actual = full_sha256(path)?;
    if actual == expected {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "rendered output no longer matches its receipt",
        format!("receipt hash {expected}; current artifact hash {actual}"),
    )
    .with_suggested_action("render again before re-running output checks"))
}

pub(super) fn full_sha256(path: &Path) -> Result<String, CutError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub(super) fn duration_matches_receipt(actual_ms: u64, receipt_ms: u64) -> cut_core::CheckResult {
    cut_core::CheckResult {
        name: "duration_matches_receipt".into(),
        pass: actual_ms == receipt_ms,
        details: json!({ "receipt_duration_ms": receipt_ms, "output_duration_ms": actual_ms }),
        evidence: json!({ "measured_by": "ffprobe", "output_duration_ms": actual_ms }),
    }
}
