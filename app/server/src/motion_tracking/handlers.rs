use super::command;
use super::contract::{
    validate_safe_id, ApplyArgs, DetachArgs, InspectArgs, InventoryArgs, RequestArgs,
    TrackingInventory, VerifyArgs,
};
use super::link::{attach_candidate, linked_source, output_package};
use crate::state::AppState;
use cut_core::{error_codes, Actor, CutError, VerbResult};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const INVALID_REQUEST_MESSAGE: &str = "Motion tracking request is invalid";

pub(crate) async fn inventory(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let request: InventoryArgs = parse(args, "motion.link.tracking.inventory")?;
    let source = linked_source(state, &request.clip).await?;
    let inventory = TrackingInventory::load(&source.package)?;
    Ok(VerbResult::ok(json!({
        "ok": true,
        "schema": "shellx-cut/motion-tracking-inventory@1",
        "clip": request.clip,
        "inventory": inventory,
        "tracking": source.link.get("tracking").cloned().unwrap_or(Value::Null),
        "localOnly": true,
    })))
}

pub(crate) async fn request(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let request: RequestArgs = parse(args, "motion.link.tracking.request")?;
    let source = linked_source(state, &request.clip).await?;
    let inventory = TrackingInventory::load(&source.package)?;
    let payload = inventory.request_payload(&request)?;
    let output = output_package(&source.project_dir, &request.clip, "analysis")?;
    let mutation = command::request(
        &source.package,
        &output,
        &request,
        &payload,
        &source.project_dir,
    )
    .await?;
    let lifecycle = lifecycle_summary(&mutation.result);
    let lifecycle_state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let op_id = attach_candidate(
        state,
        source,
        mutation.output,
        &mutation.receipt_id,
        "motion.link.tracking.request",
        "linked-current",
        json!({
            "analysisId": request.analysis_id,
            "assetId": request.asset_id,
            "mode": request.mode.as_str(),
            "model": request.model.as_str(),
            "lifecycleState": lifecycle_state,
            "attachedLayerId": null,
        }),
        actor,
        request
            .rationale
            .or_else(|| Some("Analyze linked Motion footage for stabilization".into())),
    )
    .await?;
    Ok(VerbResult::ok_with_ops(
        json!({
            "ok": true,
            "schema": "shellx-cut/motion-tracking-request@1",
            "clip": request.clip,
            "analysisId": request.analysis_id,
            "lifecycle": lifecycle,
            "receipt": {"id": mutation.receipt_id, "operation": "analysis.tracking.request"},
            "warnings": mutation.warnings,
            "state": "linked-current",
            "localOnly": true,
            "restore_hint": "The analysis is attached as a new package revision; source pixels are unchanged.",
        }),
        vec![op_id],
    ))
}

pub(crate) async fn inspect(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let request: InspectArgs = parse(args, "motion.link.tracking.inspect")?;
    validate_safe_id(&request.analysis_id, "analysis_id")?;
    let source = linked_source(state, &request.clip).await?;
    let result =
        command::inspect(&source.package, &request.analysis_id, &source.project_dir).await?;
    Ok(VerbResult::ok(json!({
        "ok": true,
        "schema": "shellx-cut/motion-tracking-inspect@1",
        "clip": request.clip,
        "analysisId": request.analysis_id,
        "lifecycle": lifecycle_summary(&result),
        "source": source_summary(&result),
        "current": result.get("current").and_then(Value::as_bool).unwrap_or(false),
        "receipt": receipt_summary(&result),
        "warnings": string_list(result.get("warnings")),
        "localOnly": true,
    })))
}

pub(crate) async fn apply(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let request: ApplyArgs = parse(args, "motion.link.tracking.apply")?;
    validate_safe_id(&request.analysis_id, "analysis_id")?;
    validate_safe_id(&request.layer_id, "layer_id")?;
    let source = linked_source(state, &request.clip).await?;
    let inventory = TrackingInventory::load(&source.package)?;
    if !inventory
        .target_layers
        .iter()
        .any(|layer| layer.id == request.layer_id)
    {
        return Err(invalid("layer_id must name a current visual Motion layer"));
    }
    let output = output_package(&source.project_dir, &request.clip, "stabilized")?;
    let mutation = command::apply(
        &source.package,
        &output,
        &request.analysis_id,
        &request.layer_id,
        request.segment_index,
        request.include_low_confidence,
        &source.project_dir,
    )
    .await?;
    let tracking = json!({
        "analysisId": request.analysis_id,
        "assetId": mutation.result.get("plan").and_then(|plan| plan.get("assetId")).cloned().unwrap_or(Value::Null),
        "lifecycleState": "succeeded",
        "attachedLayerId": request.layer_id,
        "fidelity": mutation.result.get("plan").and_then(|plan| plan.get("fidelity")).cloned().unwrap_or(Value::Null),
    });
    let op_id = attach_candidate(
        state,
        source,
        mutation.output,
        &mutation.receipt_id,
        "motion.link.tracking.apply",
        "source-dirty",
        tracking,
        actor,
        request
            .rationale
            .or_else(|| Some("Apply linked Motion tracking stabilization".into())),
    )
    .await?;
    Ok(VerbResult::ok_with_ops(
        json!({
            "ok": true,
            "schema": "shellx-cut/motion-tracking-apply@1",
            "clip": request.clip,
            "analysisId": request.analysis_id,
            "layerId": request.layer_id,
            "plan": plan_summary(&mutation.result),
            "changedPaths": string_list(mutation.result.get("changedPaths")),
            "receipt": {"id": mutation.receipt_id, "operation": "analysis.tracking.apply"},
            "warnings": mutation.warnings,
            "state": "source-dirty",
            "refreshRequired": true,
            "restore_hint": "Refresh the linked render to update pixels; project history retains the prior package binding.",
        }),
        vec![op_id],
    ))
}

pub(crate) async fn verify(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let request: VerifyArgs = parse(args, "motion.link.tracking.verify")?;
    validate_safe_id(&request.layer_id, "layer_id")?;
    if let Some(id) = &request.analysis_id {
        validate_safe_id(id, "analysis_id")?;
    }
    let source = linked_source(state, &request.clip).await?;
    let result = command::verify(
        &source.package,
        &request.layer_id,
        request.analysis_id.as_deref(),
        &source.project_dir,
    )
    .await?;
    Ok(VerbResult::ok(json!({
        "ok": true,
        "schema": "shellx-cut/motion-tracking-verify@1",
        "clip": request.clip,
        "layerId": request.layer_id,
        "analysisId": request.analysis_id,
        "verification": result.get("verification").cloned().unwrap_or(Value::Null),
        "lifecycle": lifecycle_summary(&result),
        "source": source_summary(&result),
        "receipt": receipt_summary(&result),
        "warnings": string_list(result.get("warnings")),
        "localOnly": true,
    })))
}

pub(crate) async fn detach(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let request: DetachArgs = parse(args, "motion.link.tracking.detach")?;
    validate_safe_id(&request.layer_id, "layer_id")?;
    let source = linked_source(state, &request.clip).await?;
    let output = output_package(&source.project_dir, &request.clip, "detached")?;
    let mutation = command::detach(
        &source.package,
        &output,
        &request.layer_id,
        &source.project_dir,
    )
    .await?;
    let analysis_id = mutation
        .result
        .get("analysisId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let op_id = attach_candidate(
        state,
        source,
        mutation.output,
        &mutation.receipt_id,
        "motion.link.tracking.detach",
        "source-dirty",
        json!({
            "analysisId": analysis_id,
            "lifecycleState": "detached",
            "attachedLayerId": null,
        }),
        actor,
        request
            .rationale
            .or_else(|| Some("Detach linked Motion tracking stabilization".into())),
    )
    .await?;
    Ok(VerbResult::ok_with_ops(
        json!({
            "ok": true,
            "schema": "shellx-cut/motion-tracking-detach@1",
            "clip": request.clip,
            "layerId": request.layer_id,
            "analysisId": analysis_id,
            "restoredPreviousKeyframes": mutation.result.get("restoredPreviousKeyframes").cloned().unwrap_or(Value::Bool(false)),
            "changedPaths": string_list(mutation.result.get("changedPaths")),
            "receipt": {"id": mutation.receipt_id, "operation": "analysis.tracking.detach"},
            "warnings": mutation.warnings,
            "state": "source-dirty",
            "refreshRequired": true,
        }),
        vec![op_id],
    ))
}

fn lifecycle_summary(result: &Value) -> Value {
    let lifecycle = result.get("lifecycle").unwrap_or(&Value::Null);
    json!({
        "analysisId": lifecycle.get("id").or_else(|| lifecycle.get("analysisId")).cloned().unwrap_or(Value::Null),
        "state": lifecycle.get("state").cloned().unwrap_or(Value::Null),
        "attempt": lifecycle.get("attempt").cloned().unwrap_or(Value::Null),
        "updatedAt": lifecycle.get("updatedAt").cloned().unwrap_or(Value::Null),
        "source": lifecycle.get("source").or_else(|| lifecycle.get("requestedSource")).cloned().unwrap_or(Value::Null),
        "lastGood": lifecycle.get("lastGood").map(tracking_analysis_summary).unwrap_or(Value::Null),
    })
}

fn tracking_analysis_summary(analysis: &Value) -> Value {
    json!({
        "status": analysis.get("status").cloned().unwrap_or(Value::Null),
        "mode": analysis.get("mode").cloned().unwrap_or(Value::Null),
        "model": analysis.get("model").cloned().unwrap_or(Value::Null),
        "samples": analysis.get("samples").and_then(Value::as_array).map(|samples| samples.len()).unwrap_or(0),
        "spans": analysis.get("spans").and_then(Value::as_array).map(|spans| spans.len()).unwrap_or(0),
        "warnings": string_list(analysis.get("warnings")),
    })
}

fn source_summary(result: &Value) -> Value {
    let source = result.get("source").unwrap_or(&Value::Null);
    json!({
        "assetId": source.get("assetId").cloned().unwrap_or(Value::Null),
        "current": source.get("current").cloned().unwrap_or(Value::Null),
        "sha256": source.get("sha256").cloned().unwrap_or(Value::Null),
        "byteLength": source.get("byteLength").cloned().unwrap_or(Value::Null),
    })
}

fn plan_summary(result: &Value) -> Value {
    let plan = result.get("plan").unwrap_or(&Value::Null);
    json!({
        "status": plan.get("status").cloned().unwrap_or(Value::Null),
        "fidelity": plan.get("fidelity").cloned().unwrap_or(Value::Null),
        "segmentCount": plan.get("segments").and_then(Value::as_array).map(|segments| segments.len()).unwrap_or(0),
        "warnings": string_list(plan.get("warnings")),
    })
}

fn receipt_summary(result: &Value) -> Value {
    let receipt = result.get("receipt").unwrap_or(&Value::Null);
    json!({
        "id": receipt.get("id").cloned().unwrap_or(Value::Null),
        "operation": receipt.get("operation").cloned().unwrap_or(Value::Null),
        "status": receipt.get("status").cloned().unwrap_or(Value::Null),
    })
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| value.chars().take(500).collect())
        .take(32)
        .collect()
}

fn parse<T: DeserializeOwned>(args: Value, verb: &str) -> Result<T, CutError> {
    serde_json::from_value(args).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("{verb} arguments are invalid"),
            error.to_string(),
        )
    })
}

fn invalid(detail: &str) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, INVALID_REQUEST_MESSAGE, detail)
}
