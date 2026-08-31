//! Immutable input admission and retry descriptors for `verify.rerun`.

use super::*;
use crate::dispatch::run_blocking;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

pub(super) async fn prepare_verify_rerun(
    state: &AppState,
    render_id: &str,
) -> Result<PreparedVerifyRerun, CutError> {
    let (project_dir, receipts, project_revision) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let receipts = store.receipts_dir();
        let project_revision = store
            .log
            .read_all()?
            .last()
            .map(|operation| operation.op_id.clone())
            .unwrap_or_else(|| "op_000000".into());
        (store.dir.clone(), receipts, project_revision)
    };
    let (receipt_path, receipt) = selected_render_receipt_with_path(&receipts, render_id)?;
    let output = rerun_output::fenced_output_for_receipt(&project_dir, &receipt.output_path, None)?;
    let expected_hash = receipt.output_hash.clone();
    let profile = profile_from_receipt(&receipt)?;

    // Refuse an already-stale artifact before allocating a durable job.
    let preflight_output = output.clone();
    let preflight_hash = expected_hash.clone();
    run_blocking("verify.rerun artifact identity", move || {
        assert_receipt_hash(&preflight_output, &preflight_hash)
    })
    .await?;

    Ok(PreparedVerifyRerun {
        project_dir,
        receipts,
        receipt_path,
        receipt,
        output,
        profile,
        project_revision,
    })
}

/// Build the durable retry recipe from the immutable receipt and exact artifact
/// bytes. The descriptor stays engine-private in persisted job records.
pub(super) async fn retry_descriptor_async(
    prepared: &PreparedVerifyRerun,
) -> Result<crate::jobs::VerifyRerunRetryDescriptor, CutError> {
    let prepared = prepared.clone();
    run_blocking("verify.rerun retry input fingerprints", move || {
        retry_descriptor(&prepared)
    })
    .await
}

pub(super) fn retry_descriptor(
    prepared: &PreparedVerifyRerun,
) -> Result<crate::jobs::VerifyRerunRetryDescriptor, CutError> {
    let receipt_input = fingerprint_retry_input("render_receipt", &prepared.receipt_path)?;
    let output_input = fingerprint_retry_input("rendered_output", &prepared.output)?;
    if format!("sha256:{}", output_input.sha256) != prepared.receipt.output_hash {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "rendered output no longer matches its receipt",
            "the artifact changed while retry inputs were being fingerprinted",
        )
        .with_suggested_action("render again before re-running output checks"));
    }
    let mut inputs = vec![receipt_input, output_input];
    inputs.sort_by(|left, right| left.role.cmp(&right.role).then(left.path.cmp(&right.path)));
    Ok(crate::jobs::VerifyRerunRetryDescriptor {
        project_revision: prepared.project_revision.clone(),
        render_id: prepared.receipt.render_id.clone(),
        output_hash: prepared.receipt.output_hash.clone(),
        duration_ms: prepared.receipt.duration_ms,
        footage_profile: prepared.profile.as_str().to_string(),
        inputs,
    })
}

fn fingerprint_retry_input(
    role: &str,
    path: &Path,
) -> Result<crate::jobs::JobInputFingerprint, CutError> {
    let mut file = std::fs::File::open(path)?;
    let bytes = file.metadata()?.len();
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(crate::jobs::JobInputFingerprint {
        role: role.into(),
        path: path.to_string_lossy().into_owned(),
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

pub(super) fn retry_conflict(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "verify.rerun retry is not safe to admit",
        cause.into(),
    )
}

pub(super) fn retry_revalidation_error(error: CutError) -> CutError {
    retry_conflict(format!(
        "the failed verification inputs are missing or no longer safe to use: {} ({})",
        error.message, error.cause
    ))
}

pub(super) fn verify_retry_descriptor_matches(
    expected: &crate::jobs::VerifyRerunRetryDescriptor,
    current: &crate::jobs::VerifyRerunRetryDescriptor,
) -> Result<(), CutError> {
    if expected == current {
        Ok(())
    } else {
        Err(retry_conflict(
            "the project revision, source render receipt, or rendered output changed since the failed verification",
        ))
    }
}
