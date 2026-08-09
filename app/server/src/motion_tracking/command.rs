use super::contract::{RequestArgs, TrackingRequestPayload};
use crate::motion_bridge::motion_package_identity;
use crate::motion_runtime::{build_motion_cli_command, run_motion_command_spec, MotionCommandSpec};
use cut_core::{error_codes, CutError};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) async fn request(
    package: &Path,
    output: &Path,
    input: &RequestArgs,
    payload: &TrackingRequestPayload,
    caller_scope: &Path,
) -> Result<ValidatedMutation, CutError> {
    let spec = tracking_command(
        "tracking-request",
        "write_local",
        package,
        caller_scope,
        vec![
            "--out".into(),
            output.display().to_string(),
            "--analysis-id".into(),
            input.analysis_id.clone(),
            "--asset-id".into(),
            input.asset_id.clone(),
            "--mode".into(),
            input.mode.as_str().into(),
            "--model".into(),
            input.model.as_str().into(),
            "--reference-json".into(),
            serde_json::to_string(&payload.reference)?,
            "--settings-json".into(),
            serde_json::to_string(&payload.settings)?,
        ],
    );
    let envelope = run_motion_command_spec(spec, "analyze linked Motion footage").await?;
    let validated = validate_mutation(
        envelope,
        output,
        motion_package_identity(package)?,
        "analysis.tracking.request",
    )?;
    let lifecycle_id = validated
        .result
        .get("lifecycle")
        .and_then(|lifecycle| lifecycle.get("id").or_else(|| lifecycle.get("analysisId")))
        .and_then(Value::as_str);
    if lifecycle_id != Some(input.analysis_id.as_str()) {
        return Err(conflict(
            "Motion tracking lifecycle identity did not match the request",
        ));
    }
    Ok(validated)
}

pub(crate) async fn inspect(
    package: &Path,
    analysis_id: &str,
    caller_scope: &Path,
) -> Result<Value, CutError> {
    let envelope = run_motion_command_spec(
        tracking_command(
            "tracking-inspect",
            "read_motion",
            package,
            caller_scope,
            vec!["--analysis-id".into(), analysis_id.into()],
        ),
        "inspect linked Motion tracking",
    )
    .await?;
    let result = result_object(envelope)?;
    let lifecycle_id = result
        .get("lifecycle")
        .and_then(|lifecycle| lifecycle.get("id").or_else(|| lifecycle.get("analysisId")))
        .and_then(Value::as_str);
    if lifecycle_id != Some(analysis_id) {
        return Err(conflict(
            "Motion tracking inspect returned another lifecycle",
        ));
    }
    Ok(result)
}

pub(crate) async fn apply(
    package: &Path,
    output: &Path,
    analysis_id: &str,
    layer_id: &str,
    segment_index: Option<u64>,
    include_low_confidence: bool,
    caller_scope: &Path,
) -> Result<ValidatedMutation, CutError> {
    let mut args = vec![
        "--out".into(),
        output.display().to_string(),
        "--analysis-id".into(),
        analysis_id.into(),
        "--layer-id".into(),
        layer_id.into(),
    ];
    if let Some(index) = segment_index {
        args.extend(["--segment-index".into(), index.to_string()]);
    }
    if include_low_confidence {
        args.push("--include-low-confidence".into());
    }
    let envelope = run_motion_command_spec(
        tracking_command("tracking-apply", "edit_motion", package, caller_scope, args),
        "apply linked Motion stabilization",
    )
    .await?;
    let validated = validate_mutation(
        envelope,
        output,
        motion_package_identity(package)?,
        "analysis.tracking.apply",
    )?;
    if validated.result.get("layerId").and_then(Value::as_str) != Some(layer_id) {
        return Err(conflict(
            "Motion tracking apply returned another target layer",
        ));
    }
    Ok(validated)
}

pub(crate) async fn detach(
    package: &Path,
    output: &Path,
    layer_id: &str,
    caller_scope: &Path,
) -> Result<ValidatedMutation, CutError> {
    let envelope = run_motion_command_spec(
        tracking_command(
            "tracking-detach",
            "edit_motion",
            package,
            caller_scope,
            vec![
                "--out".into(),
                output.display().to_string(),
                "--layer-id".into(),
                layer_id.into(),
            ],
        ),
        "detach linked Motion stabilization",
    )
    .await?;
    let validated = validate_mutation(
        envelope,
        output,
        motion_package_identity(package)?,
        "analysis.tracking.detach",
    )?;
    if validated.result.get("layerId").and_then(Value::as_str) != Some(layer_id) {
        return Err(conflict(
            "Motion tracking detach returned another target layer",
        ));
    }
    Ok(validated)
}

pub(crate) async fn verify(
    package: &Path,
    layer_id: &str,
    analysis_id: Option<&str>,
    caller_scope: &Path,
) -> Result<Value, CutError> {
    let mut args = vec!["--layer-id".into(), layer_id.into()];
    if let Some(id) = analysis_id {
        args.extend(["--analysis-id".into(), id.into()]);
    }
    let envelope = run_motion_command_spec(
        tracking_command(
            "tracking-verify",
            "read_motion",
            package,
            caller_scope,
            args,
        ),
        "verify linked Motion stabilization",
    )
    .await?;
    let result = result_object(envelope)?;
    let verification = result.get("verification").ok_or_else(|| {
        CutError::new(
            error_codes::SIDECAR,
            "ShellX Motion returned no tracking verification",
            "result.verification is required",
        )
    })?;
    if verification.get("layerId").and_then(Value::as_str) != Some(layer_id) {
        return Err(conflict(
            "Motion tracking verification returned another target layer",
        ));
    }
    Ok(result)
}

#[derive(Debug)]
pub(crate) struct ValidatedMutation {
    pub result: Value,
    pub output: PathBuf,
    pub receipt_id: String,
    pub warnings: Vec<String>,
}

pub(super) fn tracking_command(
    alias: &str,
    tier: &str,
    package: &Path,
    caller_scope: &Path,
    extra: Vec<String>,
) -> MotionCommandSpec {
    let mut args = vec![
        "debug".into(),
        alias.into(),
        "--tier".into(),
        tier.into(),
        "--trusted-local-tier".into(),
        "--package".into(),
        package.display().to_string(),
    ];
    args.extend(extra);
    build_motion_cli_command(args, caller_scope)
}

pub(super) fn validate_mutation(
    envelope: Value,
    expected_output: &Path,
    expected_identity: (String, String),
    operation: &str,
) -> Result<ValidatedMutation, CutError> {
    let result = result_object(envelope)?;
    let metadata = std::fs::symlink_metadata(expected_output)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(conflict(
            "Motion tracking output is not a real package directory",
        ));
    }
    let output = expected_output.canonicalize()?;
    let reported = result
        .get("packageRoot")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| conflict("Motion tracking result omitted packageRoot"))?
        .canonicalize()?;
    if output != reported {
        return Err(conflict(
            "Motion tracking result pointed at an unexpected package",
        ));
    }
    let actual_identity = motion_package_identity(&output)?;
    if actual_identity != expected_identity {
        return Err(conflict(
            "Motion tracking output changed package or motion identity",
        ));
    }
    if result.get("packageId").and_then(Value::as_str) != Some(expected_identity.0.as_str()) {
        return Err(conflict(
            "Motion tracking result packageId did not match its package",
        ));
    }
    let receipt = result
        .get("receipt")
        .and_then(Value::as_object)
        .ok_or_else(|| conflict("Motion tracking result omitted its receipt"))?;
    if receipt.get("operation").and_then(Value::as_str) != Some(operation)
        || !matches!(
            receipt.get("status").and_then(Value::as_str),
            Some("passed" | "warning")
        )
    {
        return Err(conflict(
            "Motion tracking receipt operation or status is invalid",
        ));
    }
    let receipt_id = receipt
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(|| conflict("Motion tracking receipt omitted a bounded id"))?
        .to_string();
    let warnings = receipt
        .get("warnings")
        .or_else(|| result.get("warnings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| value.chars().take(500).collect())
        .take(32)
        .collect();
    Ok(ValidatedMutation {
        result,
        output,
        receipt_id,
        warnings,
    })
}

fn result_object(envelope: Value) -> Result<Value, CutError> {
    let connector_warnings = envelope.get("warnings").cloned();
    let mut result = envelope
        .get("result")
        .filter(|result| result.is_object())
        .cloned()
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "ShellX Motion tracking returned no result object",
                "result is required",
            )
        })?;
    if result.get("warnings").is_none() {
        if let (Some(warnings), Some(object)) = (connector_warnings, result.as_object_mut()) {
            object.insert("warnings".into(), warnings);
        }
    }
    Ok(result)
}

fn conflict(message: &str) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        message,
        "the candidate package was not attached to the Cut clip",
    )
}
