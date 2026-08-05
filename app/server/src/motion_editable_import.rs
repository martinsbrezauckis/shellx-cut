//! Strict receiving/lowering boundary for ShellX Motion editable Cut plans.
//!
//! Motion owns the portable plan. This module accepts only the subset Cut can
//! represent without hidden approximation, converts it to normal Cut verbs, and
//! records a stable source-layer -> native clip binding for idempotency/reimport.

use crate::dispatch::dispatch_send;
use crate::events::Event;
use crate::state::AppState;
use cut_core::{error_codes, Actor, Checkpoint, CutError};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

const MAX_EDITABLE_OPERATIONS: usize = 64;
const MAX_MOTION_DURATION_MS: u64 = 10 * 60 * 1000;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const DOCUMENT_BACKGROUND_LAYER_ID: &str = "__shellx_motion_document_background__";

#[derive(Debug, Clone)]
pub(crate) struct EditableMotionImportPlan {
    pub document: EditableDocument,
    pub operations: Vec<EditableMotionOperation>,
}

#[derive(Debug, Clone)]
pub(crate) struct EditableDocument {
    pub width: f64,
    pub height: f64,
    pub duration_ms: u64,
    pub background: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EditableMotionOperation {
    pub raw: Value,
    pub verb: &'static str,
    pub source_layer_id: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub payload: Map<String, Value>,
}

#[derive(Debug, Clone)]
struct NativeStep {
    source_layer_id: String,
    source_verb: &'static str,
    cut_verb: &'static str,
    args: Value,
    keyframe_tracks: Vec<NativeKeyframeTrack>,
}

#[derive(Debug, Clone)]
struct NativeKeyframeTrack {
    param: &'static str,
    points: Vec<Value>,
    interp: &'static str,
}

pub(crate) struct EditableApplyResult {
    pub checkpoint: Value,
    pub bindings: Vec<Value>,
    pub op_ids: Vec<String>,
    pub mapping_op_id: String,
    pub already_applied: bool,
    pub reimported: bool,
}

pub(crate) fn parse_editable_motion_import_plan(
    plan: &Value,
    package_id: &str,
    motion_id: &str,
) -> Result<EditableMotionImportPlan, CutError> {
    verify_plan_receipt(plan, package_id, motion_id)?;
    if plan
        .get("unsupported")
        .and_then(Value::as_array)
        .is_none_or(|unsupported| !unsupported.is_empty())
    {
        return Err(editable_error(
            "Motion editable plan declares unsupported content",
            "unsupported must be an empty array for exact native lowering",
        ));
    }
    let document = parse_document(plan.get("document").unwrap_or(&Value::Null))?;
    let values = plan
        .get("operations")
        .and_then(Value::as_array)
        .filter(|operations| !operations.is_empty())
        .ok_or_else(|| {
            editable_error(
                "Motion editable plan has no operations",
                "expected at least one text or shape layer",
            )
        })?;
    if values.len() > MAX_EDITABLE_OPERATIONS {
        return Err(editable_error(
            "Motion editable plan has too many native operations",
            format!(
                "{} operations; Cut's bounded editable receiver allows {MAX_EDITABLE_OPERATIONS}",
                values.len()
            ),
        ));
    }
    let mut operations = Vec::with_capacity(values.len());
    let mut source_layer_ids = HashSet::with_capacity(values.len());
    for value in values {
        let operation = parse_operation(value, &document)?;
        if !source_layer_ids.insert(operation.source_layer_id.clone()) {
            return Err(editable_error(
                "Motion editable plan repeats a source layer identity",
                operation.source_layer_id,
            ));
        }
        operations.push(operation);
    }
    Ok(EditableMotionImportPlan {
        document,
        operations,
    })
}

pub(crate) fn editable_planned_steps(
    plan: &EditableMotionImportPlan,
) -> Result<Vec<Value>, CutError> {
    native_steps(plan).map(|steps| {
        let mut planned = Vec::new();
        for step in steps {
            planned.push(json!({
                "verb": step.cut_verb,
                "sourceVerb": step.source_verb,
                "sourceLayerId": step.source_layer_id,
                "args": step.args,
            }));
            for track in step.keyframe_tracks {
                planned.push(json!({
                    "verb": "edit.keyframe",
                    "sourceVerb": step.source_verb,
                    "sourceLayerId": step.source_layer_id,
                    "args": {
                        "clip": format!("$binding:{}", step.source_layer_id),
                        "param": track.param,
                        "points": track.points,
                        "interp": track.interp,
                    },
                }));
            }
        }
        planned
    })
}

fn native_steps(plan: &EditableMotionImportPlan) -> Result<Vec<NativeStep>, CutError> {
    let mut steps =
        Vec::with_capacity(plan.operations.len() + usize::from(plan.document.background.is_some()));
    if let Some(background) = &plan.document.background {
        steps.push(NativeStep {
            source_layer_id: DOCUMENT_BACKGROUND_LAYER_ID.into(),
            source_verb: "cut.document.background.create",
            cut_verb: "edit.add_shape",
            args: json!({
                "shape": "rect",
                "range_ms": [0, plan.document.duration_ms],
                "animation": "none",
                "fill": background,
                "x": 0.0,
                "y": 0.0,
                "w": 1.0,
                "h": 1.0,
            }),
            keyframe_tracks: Vec::new(),
        });
    }
    for operation in &plan.operations {
        steps.push(native_step(operation, &plan.document)?);
    }
    Ok(steps)
}

pub(crate) async fn apply_editable_motion_import(
    state: &AppState,
    plan: &EditableMotionImportPlan,
    plan_hash: &str,
    package_id: &str,
    motion_id: &str,
    actor: Actor,
) -> Result<EditableApplyResult, CutError> {
    let steps = native_steps(plan)?;

    // Only the latest mapping for this Motion identity is current. Exact replay
    // is idempotent; a different plan hash becomes an in-place reimport of the
    // same native objects. Older hashes must never masquerade as still applied.
    let existing = latest_identity_binding(state, package_id, motion_id).await?;
    if let Some(existing) = &existing {
        if !bindings_are_live(state, &existing.bindings).await? {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "Motion editable plan was already applied and is no longer active",
                "one or more bound Cut clips were removed or undone",
            )
            .with_suggested_action("redo the original import, or generate a new Motion plan for an intentional reimport"));
        }
        if existing.plan_hash == plan_hash {
            return Ok(EditableApplyResult {
                checkpoint: existing.checkpoint.clone(),
                bindings: existing.bindings.clone(),
                op_ids: vec![existing.op_id.clone()],
                mapping_op_id: existing.op_id.clone(),
                already_applied: true,
                reimported: false,
            });
        }
    }

    let group_id = format!("motion-editable-{}", &plan_hash[..16]);
    let reimport_steps = if let Some(existing) = &existing {
        Some(build_reimport_steps(state, &steps, &existing.bindings, &group_id).await?)
    } else {
        None
    };

    let checkpoint_result = dispatch_send(
        state,
        "project.checkpoint",
        json!({
            "name": format!("Before Motion editable import {}", &plan_hash[..12]),
            "rationale": "ShellX Motion editable lowering safety checkpoint",
        }),
        actor.clone(),
    )
    .await;
    if !checkpoint_result.ok {
        return Err(checkpoint_result.error.unwrap_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "Could not checkpoint before Motion editable import",
                "project.checkpoint returned no error details",
            )
        }));
    }
    let checkpoint_value = checkpoint_result
        .result
        .as_ref()
        .and_then(|result| result.get("checkpoint"))
        .cloned()
        .ok_or_else(|| {
            CutError::new(
                error_codes::CONFLICT,
                "Motion import checkpoint is missing",
                "project.checkpoint returned no checkpoint",
            )
        })?;
    let checkpoint: Checkpoint =
        serde_json::from_value(checkpoint_value.clone()).map_err(|error| {
            CutError::new(
                error_codes::CONFLICT,
                "Motion import checkpoint is invalid",
                error.to_string(),
            )
        })?;
    let mut op_ids = checkpoint_result.op_ids.unwrap_or_default();
    let mut bindings = Vec::with_capacity(steps.len());
    if let Some(reimport_steps) = reimport_steps {
        for step in reimport_steps {
            let replacement_asset = step.args.get("asset").cloned();
            let result = dispatch_send(state, step.cut_verb, step.args, actor.clone()).await;
            if !result.ok {
                rollback_to_checkpoint(state, &checkpoint.id, actor.clone()).await;
                return Err(result.error.unwrap_or_else(|| {
                    CutError::new(
                        error_codes::CONFLICT,
                        "Motion editable reimport failed",
                        format!("{} returned no error details", step.cut_verb),
                    )
                }));
            }
            let body = result.result.unwrap_or(Value::Null);
            op_ids.extend(result.op_ids.unwrap_or_default());
            let mut binding = step.binding;
            if let (Some(binding), Some(asset_id)) = (
                binding.as_object_mut(),
                body.get("asset_id")
                    .or_else(|| body.get("asset"))
                    .and_then(Value::as_str),
            ) {
                binding.insert("assetId".into(), json!(asset_id));
            } else if let (Some(binding), Some(asset_id)) =
                (binding.as_object_mut(), replacement_asset)
            {
                binding.insert("assetId".into(), asset_id);
            }
            let clip_id = binding
                .get("clipId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    editable_error(
                        "Motion reimport binding lost its clip identity",
                        step.cut_verb,
                    )
                })?;
            match apply_keyframe_tracks(
                state,
                clip_id,
                &step.keyframe_tracks,
                &group_id,
                actor.clone(),
                &step.previous_dynamic_params,
            )
            .await
            {
                Ok(ids) => op_ids.extend(ids),
                Err(error) => {
                    rollback_to_checkpoint(state, &checkpoint.id, actor.clone()).await;
                    return Err(error);
                }
            }
            // Keep the durable binding authoritative for the next reimport so a
            // later plan can clear a track that this plan created or retained.
            if let Some(binding_object) = binding.as_object_mut() {
                binding_object.insert(
                    "dynamicParams".into(),
                    dynamic_params_value(&step.keyframe_tracks),
                );
            }
            bindings.push(binding);
        }
    } else {
        for mut step in steps {
            if !step.args.is_object() {
                return Err(editable_error(
                    "Cut native lowering args are invalid",
                    step.args.to_string(),
                ));
            }
            let args = step.args.as_object_mut().expect("object checked above");
            args.insert("group_id".into(), json!(group_id));
            args.insert(
                "rationale".into(),
                json!(format!(
                    "ShellX Motion {} layer {}",
                    step.source_verb, step.source_layer_id
                )),
            );
            let keyframe_tracks = step.keyframe_tracks.clone();
            let source_asset = step.args.get("asset").cloned();
            let target_track = step.args.get("track").cloned();
            let result = dispatch_send(state, step.cut_verb, step.args, actor.clone()).await;
            if !result.ok {
                rollback_to_checkpoint(state, &checkpoint.id, actor.clone()).await;
                return Err(result.error.unwrap_or_else(|| {
                    CutError::new(
                        error_codes::CONFLICT,
                        "Motion editable lowering failed",
                        format!("{} returned no error details", step.cut_verb),
                    )
                }));
            }
            let body = result.result.unwrap_or(Value::Null);
            let Some(clip_id) = body
                .get("clip_id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                rollback_to_checkpoint(state, &checkpoint.id, actor.clone()).await;
                return Err(editable_error(
                    "Cut native lowering returned no clip identity",
                    format!(
                        "{} for source layer {}",
                        step.cut_verb, step.source_layer_id
                    ),
                ));
            };
            op_ids.extend(result.op_ids.unwrap_or_default());
            match apply_keyframe_tracks(
                state,
                &clip_id,
                &keyframe_tracks,
                &group_id,
                actor.clone(),
                &[],
            )
            .await
            {
                Ok(ids) => op_ids.extend(ids),
                Err(error) => {
                    rollback_to_checkpoint(state, &checkpoint.id, actor.clone()).await;
                    return Err(error);
                }
            }
            bindings.push(json!({
                "sourceLayerId": step.source_layer_id,
                "sourceVerb": step.source_verb,
                "cutVerb": step.cut_verb,
                "clipId": clip_id,
                "trackId": body.get("title_track").or_else(|| body.get("shape_track")).or_else(|| body.get("track")).cloned().or(target_track).unwrap_or(Value::Null),
                "assetId": body.get("asset_id").or_else(|| body.get("asset")).cloned().or(source_asset).unwrap_or(Value::Null),
                "dynamicParams": dynamic_params_value(&keyframe_tracks),
            }));
        }
    }

    let mapping_result = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(|| {
            CutError::new(
                error_codes::NO_PROJECT,
                "no project open",
                "open or create a project first",
            )
        })?;
        store.record_motion_editable_import(
            plan_hash,
            package_id,
            motion_id,
            checkpoint.clone(),
            Value::Array(bindings.clone()),
            bindings.len(),
            &group_id,
            actor.clone(),
            Some("ShellX Motion editable import identity binding".into()),
        )
    };
    let mapping_op = match mapping_result {
        Ok(op) => op,
        Err(error) => {
            rollback_to_checkpoint(state, &checkpoint.id, actor).await;
            return Err(error);
        }
    };
    state.events.publish(Event::OpApplied {
        op: mapping_op.clone(),
    });
    op_ids.push(mapping_op.op_id.clone());
    Ok(EditableApplyResult {
        checkpoint: checkpoint_value,
        bindings,
        op_ids,
        mapping_op_id: mapping_op.op_id,
        already_applied: false,
        reimported: existing.is_some(),
    })
}

#[derive(Debug)]
struct ExistingBinding {
    plan_hash: String,
    checkpoint: Value,
    bindings: Vec<Value>,
    op_id: String,
}

async fn latest_identity_binding(
    state: &AppState,
    package_id: &str,
    motion_id: &str,
) -> Result<Option<ExistingBinding>, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NO_PROJECT,
            "no project open",
            "open or create a project first",
        )
    })?;
    let active_mapping_ids = store
        .undo_history
        .iter()
        .take(store.undo_pos.saturating_add(1))
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for op in store.log.read_all()?.into_iter().rev() {
        if op.verb != "motion.apply_import"
            || op.args.get("mode").and_then(Value::as_str) != Some("editable_lowering")
            || !active_mapping_ids.contains(op.op_id.as_str())
            || op.args.get("package_id").and_then(Value::as_str) != Some(package_id)
            || op.args.get("motion_id").and_then(Value::as_str) != Some(motion_id)
        {
            continue;
        }
        let plan_hash = op
            .args
            .get("idempotency_key")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                editable_error("Motion editable plan hash is missing", op.op_id.clone())
            })?
            .to_string();
        let detail = op
            .effects
            .iter()
            .map(|effect| &effect.detail)
            .find(|detail| detail.get("motion_editable_import") == Some(&Value::Bool(true)))
            .ok_or_else(|| {
                editable_error(
                    "Motion editable identity binding is corrupt",
                    op.op_id.clone(),
                )
            })?;
        let checkpoint = detail.get("checkpoint").cloned().ok_or_else(|| {
            editable_error("Motion editable checkpoint is missing", op.op_id.clone())
        })?;
        let bindings = detail
            .get("layer_bindings")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                editable_error(
                    "Motion editable layer bindings are missing",
                    op.op_id.clone(),
                )
            })?;
        return Ok(Some(ExistingBinding {
            plan_hash,
            checkpoint,
            bindings,
            op_id: op.op_id,
        }));
    }
    Ok(None)
}

async fn bindings_are_live(state: &AppState, bindings: &[Value]) -> Result<bool, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NO_PROJECT,
            "no project open",
            "open or create a project first",
        )
    })?;
    Ok(bindings.iter().all(|binding| {
        let Some(clip_id) = binding.get("clipId").and_then(Value::as_str) else {
            return false;
        };
        store
            .project
            .all_sequence_tracks()
            .any(|track| track.clips.iter().any(|clip| clip.id() == Some(clip_id)))
    }))
}

struct ReimportStep {
    cut_verb: &'static str,
    args: Value,
    binding: Value,
    keyframe_tracks: Vec<NativeKeyframeTrack>,
    previous_dynamic_params: Vec<String>,
}

async fn build_reimport_steps(
    state: &AppState,
    steps: &[NativeStep],
    existing_bindings: &[Value],
    group_id: &str,
) -> Result<Vec<ReimportStep>, CutError> {
    if steps.len() != existing_bindings.len() {
        return Err(editable_error(
            "Motion editable reimport changes the native layer set",
            "adding or removing layers is not enabled in this receiver",
        ));
    }
    let mut output = Vec::with_capacity(steps.len());
    for step in steps {
        let binding = existing_bindings
            .iter()
            .find(|binding| {
                binding.get("sourceLayerId").and_then(Value::as_str)
                    == Some(step.source_layer_id.as_str())
            })
            .ok_or_else(|| {
                editable_error(
                    "Motion editable reimport changes a source layer identity",
                    step.source_layer_id.clone(),
                )
            })?;
        if binding.get("cutVerb").and_then(Value::as_str) != Some(step.cut_verb)
            || binding.get("sourceVerb").and_then(Value::as_str) != Some(step.source_verb)
        {
            return Err(editable_error(
                "Motion editable reimport changes a native layer kind",
                step.source_layer_id.clone(),
            ));
        }
        let clip_id = binding
            .get("clipId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                editable_error(
                    "Motion editable binding has no clip identity",
                    step.source_layer_id.clone(),
                )
            })?;
        verify_reimport_range(state, clip_id, &step.args).await?;
        let rationale = format!("ShellX Motion reimport layer {}", step.source_layer_id);
        let (cut_verb, args) = match step.cut_verb {
            "title.add" | "edit.add_shape" => {
                let mut args = step.args.as_object().cloned().ok_or_else(|| {
                    editable_error(
                        "Motion reimport lowering args are invalid",
                        step.args.to_string(),
                    )
                })?;
                args.remove("range_ms");
                args.insert("clip".into(), json!(clip_id));
                args.insert("group_id".into(), json!(group_id));
                args.insert("rationale".into(), json!(rationale));
                let verb = if step.cut_verb == "title.add" {
                    "title.update"
                } else {
                    "shape.update"
                };
                (verb, Value::Object(args))
            }
            "edit.insert" => {
                let range = step
                    .args
                    .get("src_range_ms")
                    .and_then(Value::as_array)
                    .filter(|range| range.len() == 2)
                    .ok_or_else(|| {
                        editable_error("Motion video source range is invalid", clip_id)
                    })?;
                (
                    "edit.replace",
                    json!({
                        "target_clip": clip_id,
                        "asset": step.args.get("asset").cloned().unwrap_or(Value::Null),
                        "source_in_ms": range[0],
                        "source_out_ms": range[1],
                        "link_audio": false,
                        "group_id": group_id,
                        "rationale": rationale,
                    }),
                )
            }
            _ => {
                return Err(editable_error(
                    "Motion native layer cannot update in place",
                    step.cut_verb,
                ))
            }
        };
        output.push(ReimportStep {
            cut_verb,
            args,
            binding: binding.clone(),
            keyframe_tracks: step.keyframe_tracks.clone(),
            previous_dynamic_params: binding
                .get("dynamicParams")
                .and_then(Value::as_array)
                .map(|params| {
                    params
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|param| matches!(*param, "opacity" | "pos_x" | "pos_y"))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Ok(output)
}

async fn verify_reimport_range(
    state: &AppState,
    clip_id: &str,
    args: &Value,
) -> Result<(), CutError> {
    let (expected_start, expected_end) = if let Some(range) = args
        .get("range_ms")
        .and_then(Value::as_array)
        .filter(|range| range.len() == 2)
    {
        let start = range[0]
            .as_u64()
            .ok_or_else(|| editable_error("Motion reimport start is invalid", clip_id))?;
        let end = range[1]
            .as_u64()
            .ok_or_else(|| editable_error("Motion reimport end is invalid", clip_id))?;
        (start, end)
    } else {
        let start = args
            .get("at_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| editable_error("Motion video reimport start is invalid", clip_id))?;
        let source = args
            .get("src_range_ms")
            .and_then(Value::as_array)
            .filter(|range| range.len() == 2)
            .ok_or_else(|| editable_error("Motion video reimport range is invalid", clip_id))?;
        let source_start = source[0]
            .as_u64()
            .ok_or_else(|| editable_error("Motion video source start is invalid", clip_id))?;
        let source_end = source[1]
            .as_u64()
            .ok_or_else(|| editable_error("Motion video source end is invalid", clip_id))?;
        let duration = source_end
            .checked_sub(source_start)
            .ok_or_else(|| editable_error("Motion video source range is reversed", clip_id))?;
        (
            start,
            start
                .checked_add(duration)
                .ok_or_else(|| editable_error("Motion video timeline range overflows", clip_id))?,
        )
    };
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::NO_PROJECT,
            "no project open",
            "open or create a project first",
        )
    })?;
    let edl = cut_core::edl_from_project(&store.project);
    let segment = edl
        .segments
        .iter()
        .find(|segment| segment.clip_id.as_deref() == Some(clip_id))
        .ok_or_else(|| editable_error("Motion bound clip is missing from the timeline", clip_id))?;
    if segment.timeline_in_ms != expected_start || segment.timeline_out_ms != expected_end {
        return Err(editable_error(
            "Motion editable reimport changes layer timing",
            format!(
                "clip {clip_id} is {}..{}, new plan requests {expected_start}..{expected_end}",
                segment.timeline_in_ms, segment.timeline_out_ms
            ),
        ));
    }
    Ok(())
}

async fn apply_keyframe_tracks(
    state: &AppState,
    clip_id: &str,
    tracks: &[NativeKeyframeTrack],
    group_id: &str,
    actor: Actor,
    previous_dynamic_params: &[String],
) -> Result<Vec<String>, CutError> {
    let mut op_ids = Vec::new();
    for param in ["opacity", "pos_x", "pos_y"] {
        let track = tracks.iter().find(|track| track.param == param);
        if track.is_none()
            && !previous_dynamic_params
                .iter()
                .any(|previous| previous == param)
        {
            continue;
        }
        let result = dispatch_send(
            state,
            "edit.keyframe",
            json!({
                "clip": clip_id,
                "param": param,
                "points": track.map(|track| track.points.clone()).unwrap_or_default(),
                "interp": track.map_or("linear", |track| track.interp),
                "group_id": group_id,
                "rationale": format!("ShellX Motion native {param} track"),
            }),
            actor.clone(),
        )
        .await;
        if !result.ok {
            return Err(result.error.unwrap_or_else(|| {
                CutError::new(
                    error_codes::CONFLICT,
                    format!("Motion native {param} lowering failed"),
                    "edit.keyframe returned no error details",
                )
            }));
        }
        op_ids.extend(result.op_ids.unwrap_or_default());
    }
    Ok(op_ids)
}

fn dynamic_params_value(tracks: &[NativeKeyframeTrack]) -> Value {
    Value::Array(tracks.iter().map(|track| json!(track.param)).collect())
}

async fn rollback_to_checkpoint(state: &AppState, checkpoint_id: &str, actor: Actor) {
    let _ = dispatch_send(
        state,
        "project.revert",
        json!({ "to": checkpoint_id, "rationale": "rollback failed ShellX Motion editable import" }),
        actor,
    )
    .await;
}

fn parse_document(value: &Value) -> Result<EditableDocument, CutError> {
    let object = exact_object(
        value,
        &[
            "width",
            "height",
            "fps",
            "durationMs",
            "background",
            "safeAreas",
        ],
        "Motion editable document",
    )?;
    let width = finite_positive(object.get("width"), "Motion document width")?;
    let height = finite_positive(object.get("height"), "Motion document height")?;
    if width > 7_680.0 || height > 7_680.0 {
        return Err(editable_error(
            "Motion editable document is too large",
            format!("{width}x{height}; maximum side is 7680"),
        ));
    }
    let duration_ms = positive_u64(object.get("durationMs"), "Motion document durationMs")?;
    if duration_ms > MAX_MOTION_DURATION_MS {
        return Err(editable_error(
            "Motion editable document is too long",
            format!("{duration_ms} ms; maximum is {MAX_MOTION_DURATION_MS} ms"),
        ));
    }
    finite_positive(object.get("fps"), "Motion document fps")?;
    let background = optional_color(object.get("background"), "Motion document background")?;
    Ok(EditableDocument {
        width,
        height,
        duration_ms,
        background,
    })
}

fn parse_operation(
    value: &Value,
    document: &EditableDocument,
) -> Result<EditableMotionOperation, CutError> {
    let object = exact_object(
        value,
        &["verb", "sourceLayerId", "startMs", "durationMs", "payload"],
        "Motion editable operation",
    )?;
    let raw_verb = bounded_string(object.get("verb"), "Motion editable verb", 96)?;
    let verb = match raw_verb.as_str() {
        "cut.title.create" => "cut.title.create",
        "cut.shape.create" => "cut.shape.create",
        "cut.media.create" => "cut.media.create",
        "cut.audio.create" => "cut.audio.create",
        "cut.caption.create"
        | "cut.timeline.track.create"
        | "cut.timeline.scene.create"
        | "cut.timeline.marker.create" => {
            return Err(editable_error(
                format!("Motion editable operation {raw_verb} is not enabled in this Cut receiver"),
                "use rendered_media until the corresponding native Cut mapping is available",
            ));
        }
        _ => {
            return Err(editable_error(
                "Motion editable operation verb is unsupported",
                raw_verb,
            ))
        }
    };
    let source_layer_id =
        bounded_string(object.get("sourceLayerId"), "Motion source layer id", 256)?;
    if source_layer_id == DOCUMENT_BACKGROUND_LAYER_ID {
        return Err(editable_error(
            "Motion source layer id is reserved by the Cut receiver",
            source_layer_id,
        ));
    }
    let start_ms = nonnegative_u64(object.get("startMs"), "Motion layer startMs")?;
    let duration_ms = positive_u64(object.get("durationMs"), "Motion layer durationMs")?;
    if start_ms
        .checked_add(duration_ms)
        .is_none_or(|end| end > document.duration_ms)
    {
        return Err(editable_error(
            "Motion editable layer lies outside the document",
            format!(
                "{source_layer_id}: {start_ms}+{duration_ms} exceeds {}",
                document.duration_ms
            ),
        ));
    }
    let payload = object
        .get("payload")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| {
            editable_error(
                "Motion editable payload is invalid",
                source_layer_id.clone(),
            )
        })?;
    Ok(EditableMotionOperation {
        raw: value.clone(),
        verb,
        source_layer_id,
        start_ms,
        duration_ms,
        payload,
    })
}

fn native_step(
    operation: &EditableMotionOperation,
    document: &EditableDocument,
) -> Result<NativeStep, CutError> {
    match operation.verb {
        "cut.title.create" => lower_title(operation, document),
        "cut.shape.create" => lower_shape(operation, document),
        "cut.media.create" => lower_video(operation),
        "cut.audio.create" => lower_audio(operation),
        _ => Err(editable_error(
            "Motion editable operation is unsupported",
            operation.verb,
        )),
    }
}

fn lower_audio(operation: &EditableMotionOperation) -> Result<NativeStep, CutError> {
    exact_payload_fields(
        &operation.payload,
        &[
            "source",
            "trimStartMs",
            "trimDurationMs",
            "loop",
            "playbackRate",
            "volume",
            "pan",
            "muted",
            "fadeInMs",
            "fadeOutMs",
            "normalizeLoudness",
        ],
        &operation.source_layer_id,
    )?;
    let source = bounded_string(operation.payload.get("source"), "Motion audio source", 256)?;
    let asset_id = source.strip_prefix("cut-asset:").filter(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    });
    let asset_id = asset_id.ok_or_else(|| {
        editable_error(
            "Motion audio source is not a bounded Cut asset reference",
            "expected cut-asset:<asset_id>",
        )
    })?;
    let playback_rate = optional_finite(
        operation.payload.get("playbackRate"),
        "Motion audio playbackRate",
    )?;
    let volume = optional_finite(operation.payload.get("volume"), "Motion audio volume")?;
    let pan = optional_finite(operation.payload.get("pan"), "Motion audio pan")?;
    if playback_rate.is_some_and(|rate| (rate - 1.0).abs() > f64::EPSILON)
        || volume.is_some_and(|value| (value - 1.0).abs() > f64::EPSILON)
        || pan.is_some_and(|value| value.abs() > f64::EPSILON)
        || operation.payload.get("loop").and_then(Value::as_bool) == Some(true)
        || operation.payload.get("muted").and_then(Value::as_bool) == Some(true)
        || operation
            .payload
            .get("normalizeLoudness")
            .and_then(Value::as_bool)
            == Some(true)
        || operation.payload.get("fadeInMs").is_some()
        || operation.payload.get("fadeOutMs").is_some()
    {
        return Err(editable_error(
            "Motion audio processing is not enabled in this receiver",
            "use playbackRate/volume 1, pan 0, and no loop, mute, fades, or loudness normalization",
        ));
    }
    let trim_start_ms = operation
        .payload
        .get("trimStartMs")
        .map(|value| nonnegative_u64(Some(value), "Motion audio trimStartMs"))
        .transpose()?
        .unwrap_or_default();
    let trim_duration_ms = operation
        .payload
        .get("trimDurationMs")
        .map(|value| positive_u64(Some(value), "Motion audio trimDurationMs"))
        .transpose()?
        .unwrap_or(operation.duration_ms);
    if trim_duration_ms != operation.duration_ms {
        return Err(editable_error(
            "Motion audio trim duration differs from its timeline duration",
            "exact native playbackRate 1 lowering requires equal durations",
        ));
    }
    let trim_end_ms = trim_start_ms.checked_add(trim_duration_ms).ok_or_else(|| {
        editable_error(
            "Motion audio trim range overflows",
            operation.source_layer_id.clone(),
        )
    })?;
    Ok(NativeStep {
        source_layer_id: operation.source_layer_id.clone(),
        source_verb: operation.verb,
        cut_verb: "edit.insert",
        args: json!({
            "asset": asset_id,
            "track": "a1t",
            "at_ms": operation.start_ms,
            "src_range_ms": [trim_start_ms, trim_end_ms],
            "ripple": false,
        }),
        keyframe_tracks: Vec::new(),
    })
}

fn lower_video(operation: &EditableMotionOperation) -> Result<NativeStep, CutError> {
    reject_dynamic_payload(operation)?;
    exact_payload_fields(
        &operation.payload,
        &[
            "kind",
            "source",
            "fit",
            "trimStartMs",
            "trimDurationMs",
            "loop",
            "playbackRate",
            "includeAudio",
            "opacity",
            "keyframes",
            "transitions",
            "transform",
            "style",
        ],
        &operation.source_layer_id,
    )?;
    if operation.payload.get("kind").and_then(Value::as_str) != Some("video") {
        return Err(editable_error(
            "Motion native media layer is not a video",
            "this receiver accepts Cut-origin video references only",
        ));
    }
    let source = bounded_string(operation.payload.get("source"), "Motion video source", 256)?;
    let asset_id = source.strip_prefix("cut-asset:").filter(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    });
    let asset_id = asset_id.ok_or_else(|| {
        editable_error(
            "Motion video source is not a bounded Cut asset reference",
            "expected cut-asset:<asset_id>",
        )
    })?;
    if operation.payload.get("fit").and_then(Value::as_str) != Some("cover") {
        return Err(editable_error(
            "Motion video fit cannot be preserved by this Cut receiver",
            "exact native video lowering currently requires fit:'cover'",
        ));
    }
    let transform = optional_object(operation.payload.get("transform"), "Motion video transform")?;
    exact_payload_fields(
        &transform,
        &["scale", "rotation"],
        &operation.source_layer_id,
    )?;
    require_identity_transform(&transform, &operation.source_layer_id)?;
    let style = optional_object(operation.payload.get("style"), "Motion video style")?;
    exact_payload_fields(&style, &[], &operation.source_layer_id)?;
    if optional_finite(
        operation.payload.get("playbackRate"),
        "Motion video playbackRate",
    )?
    .is_some_and(|rate| (rate - 1.0).abs() > f64::EPSILON)
        || operation.payload.get("loop").and_then(Value::as_bool) == Some(true)
        || operation
            .payload
            .get("includeAudio")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return Err(editable_error(
            "Motion video playback/audio controls are not enabled in this receiver",
            "use playbackRate 1 and includeAudio false",
        ));
    }
    let trim_start_ms = operation
        .payload
        .get("trimStartMs")
        .map(|value| nonnegative_u64(Some(value), "Motion video trimStartMs"))
        .transpose()?
        .unwrap_or_default();
    let trim_duration_ms = operation
        .payload
        .get("trimDurationMs")
        .map(|value| positive_u64(Some(value), "Motion video trimDurationMs"))
        .transpose()?
        .unwrap_or(operation.duration_ms);
    if trim_duration_ms != operation.duration_ms {
        return Err(editable_error(
            "Motion video trim duration differs from its timeline duration",
            "exact native playbackRate 1 lowering requires equal durations",
        ));
    }
    let trim_end_ms = trim_start_ms.checked_add(trim_duration_ms).ok_or_else(|| {
        editable_error(
            "Motion video trim range overflows",
            operation.source_layer_id.clone(),
        )
    })?;
    Ok(NativeStep {
        source_layer_id: operation.source_layer_id.clone(),
        source_verb: operation.verb,
        cut_verb: "edit.insert",
        args: json!({
            "asset": asset_id,
            "track": "v1",
            "at_ms": operation.start_ms,
            "src_range_ms": [trim_start_ms, trim_end_ms],
            "ripple": false,
        }),
        keyframe_tracks: lower_visual_keyframe_tracks(operation, None)?,
    })
}

fn lower_title(
    operation: &EditableMotionOperation,
    document: &EditableDocument,
) -> Result<NativeStep, CutError> {
    reject_dynamic_payload(operation)?;
    exact_payload_fields(
        &operation.payload,
        &[
            "text",
            "transform",
            "style",
            "opacity",
            "keyframes",
            "transitions",
        ],
        &operation.source_layer_id,
    )?;
    let text = bounded_string(
        operation.payload.get("text"),
        "Motion title text",
        MAX_TEXT_BYTES,
    )?;
    let style = optional_object(operation.payload.get("style"), "Motion title style")?;
    exact_payload_fields(&style, &["color", "fontSize"], &operation.source_layer_id)?;
    let transform = optional_object(operation.payload.get("transform"), "Motion title transform")?;
    exact_payload_fields(
        &transform,
        &["x", "y", "scale", "rotation"],
        &operation.source_layer_id,
    )?;
    require_identity_transform(&transform, &operation.source_layer_id)?;
    let keyframe_tracks =
        lower_visual_keyframe_tracks(operation, Some((document.width, document.height)))?;
    let mut args = Map::new();
    args.insert("text".into(), json!(text));
    args.insert(
        "range_ms".into(),
        json!([
            operation.start_ms,
            operation.start_ms + operation.duration_ms
        ]),
    );
    args.insert("preset".into(), json!("headline"));
    args.insert("animation".into(), json!("none"));
    args.insert("bg".into(), json!(false));
    if let Some(color) = optional_color(style.get("color"), "Motion title color")? {
        args.insert("color".into(), json!(color));
    }
    if let Some(font_size) = optional_finite(style.get("fontSize"), "Motion title fontSize")? {
        if !(1.0..=512.0).contains(&font_size) {
            return Err(editable_error(
                "Motion title fontSize is out of range",
                font_size.to_string(),
            ));
        }
        args.insert("font_px".into(), json!(font_size));
    }
    let x = optional_finite(transform.get("x"), "Motion title x")?;
    let y = optional_finite(transform.get("y"), "Motion title y")?;
    if x.is_some() != y.is_some() {
        return Err(editable_error(
            "Motion title placement is incomplete",
            "x and y must either both be present or both be omitted",
        ));
    }
    if let (Some(x), Some(y)) = (x, y) {
        args.insert(
            "x".into(),
            json!(normalized_axis(x, document.width, "Motion title x")?),
        );
        args.insert(
            "y".into(),
            json!(normalized_axis(y, document.height, "Motion title y")?),
        );
    }
    Ok(NativeStep {
        source_layer_id: operation.source_layer_id.clone(),
        source_verb: operation.verb,
        cut_verb: "title.add",
        args: Value::Object(args),
        keyframe_tracks,
    })
}

fn lower_shape(
    operation: &EditableMotionOperation,
    document: &EditableDocument,
) -> Result<NativeStep, CutError> {
    reject_dynamic_payload(operation)?;
    exact_payload_fields(
        &operation.payload,
        &[
            "shape",
            "fill",
            "color",
            "transform",
            "style",
            "opacity",
            "keyframes",
            "transitions",
        ],
        &operation.source_layer_id,
    )?;
    let source_shape = bounded_string(operation.payload.get("shape"), "Motion shape kind", 64)?;
    let shape = match source_shape.as_str() {
        "rect" | "rounded-rect" => "rect",
        "ellipse" | "circle" => "ellipse",
        "line" => "line",
        _ => {
            return Err(editable_error(
                "Motion shape kind cannot be preserved by Cut",
                source_shape,
            ))
        }
    };
    let style = optional_object(operation.payload.get("style"), "Motion shape style")?;
    exact_payload_fields(
        &style,
        &["fill", "stroke", "strokeWidth", "radius", "opacity"],
        &operation.source_layer_id,
    )?;
    let transform = optional_object(operation.payload.get("transform"), "Motion shape transform")?;
    exact_payload_fields(
        &transform,
        &["x", "y", "width", "height", "scale", "rotation"],
        &operation.source_layer_id,
    )?;
    require_identity_transform(&transform, &operation.source_layer_id)?;
    let keyframe_tracks =
        lower_visual_keyframe_tracks(operation, Some((document.width, document.height)))?;
    let mut args = Map::new();
    args.insert("shape".into(), json!(shape));
    args.insert(
        "range_ms".into(),
        json!([
            operation.start_ms,
            operation.start_ms + operation.duration_ms
        ]),
    );
    args.insert("animation".into(), json!("none"));
    let fill_value = operation
        .payload
        .get("fill")
        .or_else(|| operation.payload.get("color"))
        .or_else(|| style.get("fill"));
    if let Some(fill) = optional_color(fill_value, "Motion shape fill")? {
        args.insert("fill".into(), json!(fill));
    }
    if let Some(stroke) = optional_color(style.get("stroke"), "Motion shape stroke")? {
        args.insert("stroke".into(), json!(stroke));
    }
    if let Some(value) = optional_finite(style.get("strokeWidth"), "Motion shape strokeWidth")? {
        if value < 0.0 {
            return Err(editable_error(
                "Motion shape strokeWidth is out of range",
                value.to_string(),
            ));
        }
        args.insert("stroke_px".into(), json!(value));
    }
    if let Some(value) = optional_finite(style.get("radius"), "Motion shape radius")? {
        if value < 0.0 {
            return Err(editable_error(
                "Motion shape radius is out of range",
                value.to_string(),
            ));
        }
        args.insert("radius_px".into(), json!(value));
    }
    if let Some(value) = optional_finite(style.get("opacity"), "Motion shape opacity")? {
        if !(0.0..=1.0).contains(&value) {
            return Err(editable_error(
                "Motion shape opacity is out of range",
                value.to_string(),
            ));
        }
        args.insert("opacity".into(), json!(value));
    }
    let x = optional_finite(transform.get("x"), "Motion shape x")?.unwrap_or(document.width * 0.3);
    let y = optional_finite(transform.get("y"), "Motion shape y")?.unwrap_or(document.height * 0.4);
    let width = optional_finite(transform.get("width"), "Motion shape width")?
        .unwrap_or(document.width * 0.4);
    let height = optional_finite(transform.get("height"), "Motion shape height")?
        .unwrap_or(document.height * 0.2);
    let normalized_x = normalized_axis(x, document.width, "Motion shape x")?;
    let normalized_y = normalized_axis(y, document.height, "Motion shape y")?;
    args.insert("x".into(), json!(normalized_x));
    args.insert("y".into(), json!(normalized_y));
    if shape == "line" {
        args.insert(
            "x2".into(),
            json!(normalized_axis(
                x + width,
                document.width,
                "Motion line x2"
            )?),
        );
        args.insert(
            "y2".into(),
            json!(normalized_axis(
                y + height,
                document.height,
                "Motion line y2"
            )?),
        );
    } else {
        let normalized_width =
            positive_normalized_size(width, document.width, "Motion shape width")?;
        let normalized_height =
            positive_normalized_size(height, document.height, "Motion shape height")?;
        if normalized_x + normalized_width > 1.0 || normalized_y + normalized_height > 1.0 {
            return Err(editable_error(
                "Motion shape bounds exceed the Cut canvas",
                format!("x={x}, y={y}, width={width}, height={height}"),
            ));
        }
        args.insert("w".into(), json!(normalized_width));
        args.insert("h".into(), json!(normalized_height));
    }
    Ok(NativeStep {
        source_layer_id: operation.source_layer_id.clone(),
        source_verb: operation.verb,
        cut_verb: "edit.add_shape",
        args: Value::Object(args),
        keyframe_tracks,
    })
}

fn reject_dynamic_payload(operation: &EditableMotionOperation) -> Result<(), CutError> {
    for field in ["mask", "effects"] {
        if operation
            .payload
            .get(field)
            .is_some_and(|value| value.as_object().is_none_or(|object| !object.is_empty()))
        {
            return Err(editable_error(
                format!("Motion layer {} uses {field}, which this Cut native receiver cannot preserve yet", operation.source_layer_id),
                "regenerate the handoff in rendered_media mode",
            ));
        }
    }
    if operation.payload.get("trackId").is_some() || operation.payload.get("blendMode").is_some() {
        return Err(editable_error(
            format!(
                "Motion layer {} uses track/blend state not enabled in this receiver",
                operation.source_layer_id
            ),
            "regenerate the handoff in rendered_media mode",
        ));
    }
    Ok(())
}

fn lower_visual_keyframe_tracks(
    operation: &EditableMotionOperation,
    position_extents: Option<(f64, f64)>,
) -> Result<Vec<NativeKeyframeTrack>, CutError> {
    let keyframes = optional_object(operation.payload.get("keyframes"), "Motion layer keyframes")?;
    let allowed = if position_extents.is_some() {
        ["opacity", "transform.x", "transform.y"].as_slice()
    } else {
        ["opacity"].as_slice()
    };
    exact_payload_fields(&keyframes, allowed, &operation.source_layer_id)?;

    let mut tracks = Vec::with_capacity(keyframes.len().max(1));
    if let Some(track) = lower_opacity_track(operation, &keyframes)? {
        tracks.push(track);
    }
    if let Some((width, height)) = position_extents {
        if let Some(frames) = keyframes.get("transform.x") {
            tracks.push(lower_position_track(
                operation,
                frames,
                "transform.x",
                "pos_x",
                width,
            )?);
        }
        if let Some(frames) = keyframes.get("transform.y") {
            tracks.push(lower_position_track(
                operation,
                frames,
                "transform.y",
                "pos_y",
                height,
            )?);
        }
    }
    Ok(tracks)
}

fn lower_opacity_track(
    operation: &EditableMotionOperation,
    keyframes: &Map<String, Value>,
) -> Result<Option<NativeKeyframeTrack>, CutError> {
    let base_opacity = optional_finite(operation.payload.get("opacity"), "Motion layer opacity")?;
    if base_opacity.is_some_and(|value| !(0.0..=1.0).contains(&value)) {
        return Err(editable_error(
            "Motion layer opacity is out of range",
            base_opacity.unwrap_or_default().to_string(),
        ));
    }
    let transitions = optional_object(
        operation.payload.get("transitions"),
        "Motion layer transitions",
    )?;
    exact_payload_fields(&transitions, &["in", "out"], &operation.source_layer_id)?;
    if !transitions.is_empty() {
        if keyframes.get("opacity").is_some() {
            return Err(editable_error(
                "Motion fade transitions conflict with opacity keyframes",
                "Cut cannot exactly combine multiplicative fades with an explicit opacity track",
            ));
        }
        return lower_fade_opacity_track(operation, base_opacity.unwrap_or(1.0), &transitions)
            .map(Some);
    }
    let Some(frames_value) = keyframes.get("opacity") else {
        return Ok(base_opacity.map(|opacity| NativeKeyframeTrack {
            param: "opacity",
            points: vec![
                json!({ "t_ms": 0, "value": opacity }),
                json!({ "t_ms": operation.duration_ms, "value": opacity }),
            ],
            interp: "linear",
        }));
    };
    let frames = frames_value.as_array().ok_or_else(|| {
        editable_error("Motion opacity keyframes are invalid", "expected an array")
    })?;
    if frames.is_empty() || frames.len() > 128 {
        return Err(editable_error(
            "Motion opacity keyframe count is out of range",
            format!("{} frames; expected 1..=128", frames.len()),
        ));
    }
    let mut points = Vec::with_capacity(frames.len());
    let mut segment_interp: Option<&'static str> = None;
    let mut previous_at_ms: Option<u64> = None;
    for (index, frame) in frames.iter().enumerate() {
        let frame = exact_object(
            frame,
            &["atMs", "value", "easing"],
            "Motion opacity keyframe",
        )?;
        let at_ms = nonnegative_u64(frame.get("atMs"), "Motion opacity keyframe atMs")?;
        let end_ms = operation.start_ms + operation.duration_ms;
        if at_ms < operation.start_ms || at_ms > end_ms {
            return Err(editable_error(
                "Motion opacity keyframe lies outside its layer",
                format!(
                    "{}: {at_ms} not in {}..={end_ms}",
                    operation.source_layer_id, operation.start_ms
                ),
            ));
        }
        if previous_at_ms.is_some_and(|previous| at_ms <= previous) {
            return Err(editable_error(
                "Motion opacity keyframes are not strictly ordered",
                at_ms.to_string(),
            ));
        }
        previous_at_ms = Some(at_ms);
        let value = optional_finite(frame.get("value"), "Motion opacity keyframe value")?
            .ok_or_else(|| {
                editable_error(
                    "Motion opacity keyframe value is missing",
                    operation.source_layer_id.clone(),
                )
            })?;
        if !(0.0..=1.0).contains(&value) {
            return Err(editable_error(
                "Motion opacity keyframe value is out of range",
                value.to_string(),
            ));
        }
        points.push(json!({
            "t_ms": at_ms - operation.start_ms,
            "value": value,
        }));
        if index + 1 < frames.len() {
            let interp = cut_opacity_interp(frame.get("easing"))?;
            if segment_interp.is_some_and(|existing| existing != interp) {
                return Err(editable_error(
                    "Motion opacity keyframes use mixed easing",
                    "Cut requires one interpolation mode for the complete native opacity track",
                ));
            }
            segment_interp = Some(interp);
        }
    }
    Ok(Some(NativeKeyframeTrack {
        param: "opacity",
        points,
        interp: segment_interp.unwrap_or("linear"),
    }))
}

fn lower_position_track(
    operation: &EditableMotionOperation,
    frames_value: &Value,
    motion_target: &'static str,
    cut_param: &'static str,
    extent: f64,
) -> Result<NativeKeyframeTrack, CutError> {
    let frames = frames_value.as_array().ok_or_else(|| {
        editable_error(
            format!("Motion {motion_target} keyframes are invalid"),
            "expected an array",
        )
    })?;
    if frames.is_empty() || frames.len() > 128 {
        return Err(editable_error(
            format!("Motion {motion_target} keyframe count is out of range"),
            format!("{} frames; expected 1..=128", frames.len()),
        ));
    }
    let mut points = Vec::with_capacity(frames.len());
    let mut segment_interp: Option<&'static str> = None;
    let mut previous_at_ms: Option<u64> = None;
    for (index, frame) in frames.iter().enumerate() {
        let frame = exact_object(
            frame,
            &["atMs", "value", "easing"],
            &format!("Motion {motion_target} keyframe"),
        )?;
        let at_ms = nonnegative_u64(
            frame.get("atMs"),
            &format!("Motion {motion_target} keyframe atMs"),
        )?;
        let end_ms = operation.start_ms + operation.duration_ms;
        if at_ms < operation.start_ms || at_ms > end_ms {
            return Err(editable_error(
                format!("Motion {motion_target} keyframe lies outside its layer"),
                format!(
                    "{}: {at_ms} not in {}..={end_ms}",
                    operation.source_layer_id, operation.start_ms
                ),
            ));
        }
        if previous_at_ms.is_some_and(|previous| at_ms <= previous) {
            return Err(editable_error(
                format!("Motion {motion_target} keyframes are not strictly ordered"),
                at_ms.to_string(),
            ));
        }
        previous_at_ms = Some(at_ms);
        let value = optional_finite(
            frame.get("value"),
            &format!("Motion {motion_target} keyframe value"),
        )?
        .ok_or_else(|| {
            editable_error(
                format!("Motion {motion_target} keyframe value is missing"),
                operation.source_layer_id.clone(),
            )
        })?;
        points.push(json!({
            "t_ms": at_ms - operation.start_ms,
            "value": value / extent,
        }));
        if index + 1 < frames.len() {
            let interp = cut_keyframe_interp(frame.get("easing"), motion_target)?;
            if segment_interp.is_some_and(|existing| existing != interp) {
                return Err(editable_error(
                    format!("Motion {motion_target} keyframes use mixed easing"),
                    format!("Cut requires one interpolation mode for the complete native {cut_param} track"),
                ));
            }
            segment_interp = Some(interp);
        }
    }
    Ok(NativeKeyframeTrack {
        param: cut_param,
        points,
        interp: segment_interp.unwrap_or("linear"),
    })
}

#[derive(Clone, Copy)]
struct FadeEdge {
    duration_ms: u64,
    interp: &'static str,
}

fn lower_fade_opacity_track(
    operation: &EditableMotionOperation,
    base_opacity: f64,
    transitions: &Map<String, Value>,
) -> Result<NativeKeyframeTrack, CutError> {
    let fade_in = lower_fade_edge(transitions.get("in"), "in")?;
    let fade_out = lower_fade_edge(transitions.get("out"), "out")?;
    let fade_duration_ms = fade_in
        .map(|edge| edge.duration_ms)
        .unwrap_or_default()
        .checked_add(fade_out.map(|edge| edge.duration_ms).unwrap_or_default())
        .ok_or_else(|| {
            editable_error(
                "Motion fade duration overflows",
                operation.source_layer_id.clone(),
            )
        })?;
    if fade_duration_ms > operation.duration_ms {
        return Err(editable_error(
            "Motion fade transitions overlap",
            "Motion multiplies overlapping fades, which one Cut opacity track cannot preserve exactly",
        ));
    }
    let interp = fade_in
        .map(|edge| edge.interp)
        .or_else(|| fade_out.map(|edge| edge.interp))
        .unwrap_or("linear");
    if fade_in.is_some_and(|edge| edge.interp != interp)
        || fade_out.is_some_and(|edge| edge.interp != interp)
    {
        return Err(editable_error(
            "Motion fade transitions use mixed easing",
            "Cut requires one interpolation mode for the complete native opacity track",
        ));
    }

    let mut points = Vec::with_capacity(4);
    if let Some(edge) = fade_in {
        points.push(json!({ "t_ms": 0, "value": 0 }));
        points.push(json!({ "t_ms": edge.duration_ms, "value": base_opacity }));
    } else {
        points.push(json!({ "t_ms": 0, "value": base_opacity }));
    }
    if let Some(edge) = fade_out {
        let out_start_ms = operation.duration_ms - edge.duration_ms;
        let last_ms = points
            .last()
            .and_then(|point| point.get("t_ms"))
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if out_start_ms > last_ms {
            points.push(json!({ "t_ms": out_start_ms, "value": base_opacity }));
        }
        points.push(json!({ "t_ms": operation.duration_ms, "value": 0 }));
    } else if points
        .last()
        .and_then(|point| point.get("t_ms"))
        .and_then(Value::as_u64)
        != Some(operation.duration_ms)
    {
        points.push(json!({ "t_ms": operation.duration_ms, "value": base_opacity }));
    }
    Ok(NativeKeyframeTrack {
        param: "opacity",
        points,
        interp,
    })
}

fn lower_fade_edge(value: Option<&Value>, edge: &str) -> Result<Option<FadeEdge>, CutError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let transition = exact_object(
        value,
        &["type", "durationMs", "easing"],
        "Motion fade transition",
    )?;
    if transition.get("type").and_then(Value::as_str) != Some("fade") {
        return Err(editable_error(
            format!("Motion {edge} transition is not a fade"),
            "only exact fade transitions can lower to native Cut opacity automation",
        ));
    }
    let duration_ms = nonnegative_u64(
        transition.get("durationMs"),
        "Motion fade transition durationMs",
    )?;
    if duration_ms == 0 {
        return Err(editable_error(
            "Motion fade transition duration must be positive",
            edge,
        ));
    }
    Ok(Some(FadeEdge {
        duration_ms,
        interp: cut_fade_interp(transition.get("easing"))?,
    }))
}

fn cut_fade_interp(value: Option<&Value>) -> Result<&'static str, CutError> {
    if value.and_then(Value::as_str) == Some("back-out") {
        return Err(editable_error(
            "Motion fade easing cannot be preserved by Cut",
            "back-out fades clamp their multiplier before base opacity is applied",
        ));
    }
    cut_keyframe_interp(value, "opacity")
}

fn cut_opacity_interp(value: Option<&Value>) -> Result<&'static str, CutError> {
    cut_keyframe_interp(value, "opacity")
}

fn cut_keyframe_interp(
    value: Option<&Value>,
    motion_target: &str,
) -> Result<&'static str, CutError> {
    match value.and_then(Value::as_str).unwrap_or("linear") {
        "linear" => Ok("linear"),
        "hold" => Ok("hold"),
        "ease-in" => Ok("ease_in_quad"),
        "ease-out" => Ok("ease_out_quad"),
        "ease-in-out" => Ok("ease_in_out_quad"),
        "back-out" => Ok("ease_out_back"),
        "bounce-out" => Ok("ease_out_bounce"),
        easing => Err(editable_error(
            format!("Motion {motion_target} easing cannot be preserved by Cut"),
            easing,
        )),
    }
}

fn require_identity_transform(
    transform: &Map<String, Value>,
    layer_id: &str,
) -> Result<(), CutError> {
    if optional_finite(transform.get("scale"), "Motion transform scale")?
        .is_some_and(|value| (value - 1.0).abs() > f64::EPSILON)
        || optional_finite(transform.get("rotation"), "Motion transform rotation")?
            .is_some_and(|value| value.abs() > f64::EPSILON)
    {
        return Err(editable_error(
            format!("Motion layer {layer_id} uses scale or rotation not enabled in this receiver"),
            "regenerate the handoff in rendered_media mode",
        ));
    }
    Ok(())
}

fn verify_plan_receipt(plan: &Value, package_id: &str, _motion_id: &str) -> Result<(), CutError> {
    let receipt = exact_object(
        plan.get("receipt").unwrap_or(&Value::Null),
        &[
            "schema",
            "id",
            "operation",
            "status",
            "packageId",
            "inputHashes",
            "createdAt",
            "lane",
            "output",
            "warnings",
        ],
        "Motion editable plan receipt",
    )?;
    require_string(
        receipt,
        "schema",
        "shellx-motion/receipt@1",
        "Motion editable plan receipt",
    )?;
    require_string(
        receipt,
        "operation",
        "cut.import.plan",
        "Motion editable plan receipt",
    )?;
    require_success_status(receipt, "status", "Motion editable plan receipt")?;
    require_string(
        receipt,
        "packageId",
        package_id,
        "Motion editable plan receipt",
    )?;
    let output = receipt
        .get("output")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            editable_error(
                "Motion editable receipt output is invalid",
                "expected an object",
            )
        })?;
    require_string(
        output,
        "mode",
        "editable_lowering",
        "Motion editable plan receipt output",
    )?;
    Ok(())
}

fn exact_object<'a>(
    value: &'a Value,
    allowed: &[&str],
    label: &str,
) -> Result<&'a Map<String, Value>, CutError> {
    let object = value
        .as_object()
        .ok_or_else(|| editable_error(format!("{label} is invalid"), "expected an object"))?;
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(editable_error(
            format!("{label} contains an unknown field"),
            field.clone(),
        ));
    }
    Ok(object)
}

fn exact_payload_fields(
    payload: &Map<String, Value>,
    allowed: &[&str],
    layer_id: &str,
) -> Result<(), CutError> {
    if let Some(field) = payload
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(editable_error(
            format!("Motion layer {layer_id} uses payload field '{field}' that Cut cannot preserve exactly"),
            "regenerate the handoff in rendered_media mode",
        ));
    }
    Ok(())
}

fn optional_object(value: Option<&Value>, label: &str) -> Result<Map<String, Value>, CutError> {
    match value {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(value) => value
            .as_object()
            .cloned()
            .ok_or_else(|| editable_error(format!("{label} is invalid"), "expected an object")),
    }
}

fn bounded_string(
    value: Option<&Value>,
    label: &str,
    max_bytes: usize,
) -> Result<String, CutError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.len() <= max_bytes)
        .map(str::to_string)
        .ok_or_else(|| {
            editable_error(
                format!("{label} is invalid"),
                format!("expected a non-empty string no longer than {max_bytes} bytes"),
            )
        })
}

fn finite_positive(value: Option<&Value>, label: &str) -> Result<f64, CutError> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| {
            editable_error(
                format!("{label} is invalid"),
                "expected a positive finite number",
            )
        })
}

fn optional_finite(value: Option<&Value>, label: &str) -> Result<Option<f64>, CutError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| {
                editable_error(format!("{label} is invalid"), "expected a finite number")
            }),
    }
}

fn normalized_axis(value: f64, extent: f64, label: &str) -> Result<f64, CutError> {
    let normalized = value / extent;
    if (0.0..=1.0).contains(&normalized) {
        Ok(normalized)
    } else {
        Err(editable_error(
            format!("{label} is outside the Cut canvas"),
            value.to_string(),
        ))
    }
}

fn positive_normalized_size(value: f64, extent: f64, label: &str) -> Result<f64, CutError> {
    let normalized = value / extent;
    if normalized > 0.0 && normalized <= 1.0 {
        Ok(normalized)
    } else {
        Err(editable_error(
            format!("{label} is out of range"),
            value.to_string(),
        ))
    }
}

fn positive_u64(value: Option<&Value>, label: &str) -> Result<u64, CutError> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| editable_error(format!("{label} is invalid"), "expected a positive integer"))
}

fn nonnegative_u64(value: Option<&Value>, label: &str) -> Result<u64, CutError> {
    value.and_then(Value::as_u64).ok_or_else(|| {
        editable_error(
            format!("{label} is invalid"),
            "expected a non-negative integer",
        )
    })
}

fn optional_color(value: Option<&Value>, label: &str) -> Result<Option<String>, CutError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(color) = value.as_str() else {
        return Err(editable_error(
            format!("{label} is invalid"),
            "expected #RRGGBB",
        ));
    };
    if color.len() == 7
        && color.starts_with('#')
        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(Some(color.to_ascii_uppercase()))
    } else {
        Err(editable_error(format!("{label} is invalid"), color))
    }
}

fn require_string(
    object: &Map<String, Value>,
    field: &str,
    expected: &str,
    label: &str,
) -> Result<(), CutError> {
    if object.get(field).and_then(Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(editable_error(
            format!("{label} field '{field}' is unsupported"),
            format!("expected '{expected}'"),
        ))
    }
}

fn require_success_status(
    object: &Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<(), CutError> {
    match object.get(field).and_then(Value::as_str) {
        Some("passed" | "warning") => Ok(()),
        status => Err(editable_error(
            format!("{label} field '{field}' is not successful"),
            format!(
                "expected 'passed' or 'warning', got {}",
                status.unwrap_or("missing")
            ),
        )),
    }
}

fn editable_error(message: impl Into<String>, details: impl Into<String>) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, details)
        .with_suggested_action("render the Motion package and import rendered_media when exact native lowering is unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn static_plan() -> Value {
        json!({
            "document": { "width": 1920, "height": 1080, "fps": 30, "durationMs": 3000 },
            "operations": [
                { "verb": "cut.shape.create", "sourceLayerId": "panel", "startMs": 0, "durationMs": 3000,
                  "payload": { "shape": "rounded-rect", "fill": "#112233", "transform": { "x": 192, "y": 756, "width": 960, "height": 216 } } },
                { "verb": "cut.title.create", "sourceLayerId": "title", "startMs": 100, "durationMs": 2500,
                  "payload": { "text": "Native Motion title", "transform": { "x": 240, "y": 820 }, "style": { "color": "#ffffff", "fontSize": 64 } } }
            ],
            "unsupported": [],
            "receipt": { "schema": "shellx-motion/receipt@1", "id": "cut-import-1", "operation": "cut.import.plan", "status": "passed", "packageId": "pkg_demo", "inputHashes": {}, "createdAt": "2026-07-12T00:00:00Z", "lane": "cut", "output": { "mode": "editable_lowering" }, "warnings": [] }
        })
    }

    #[test]
    fn maps_static_text_and_shape_to_visible_native_cut_verbs() {
        let plan =
            parse_editable_motion_import_plan(&static_plan(), "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        assert_eq!(steps[0]["verb"], json!("edit.add_shape"));
        assert_eq!(steps[0]["args"]["shape"], json!("rect"));
        assert_eq!(steps[0]["args"]["x"], json!(0.1));
        assert_eq!(steps[1]["verb"], json!("title.add"));
        assert_eq!(steps[1]["args"]["text"], json!("Native Motion title"));
        assert_eq!(steps[1]["args"]["animation"], json!("none"));
    }

    #[test]
    fn materializes_document_background_as_a_bound_native_shape() {
        let mut source = static_plan();
        source["document"]["background"] = json!("#102030");
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0]["verb"], json!("edit.add_shape"));
        assert_eq!(
            steps[0]["sourceLayerId"],
            json!(DOCUMENT_BACKGROUND_LAYER_ID)
        );
        assert_eq!(steps[0]["args"]["fill"], json!("#102030"));
        assert_eq!(steps[0]["args"]["w"], json!(1.0));
        assert_eq!(steps[0]["args"]["h"], json!(1.0));

        source["operations"][0]["sourceLayerId"] = json!(DOCUMENT_BACKGROUND_LAYER_ID);
        assert!(parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").is_err());
    }

    #[test]
    fn refuses_unsupported_dynamic_or_unknown_payload_instead_of_approximating_it() {
        let mut plan = static_plan();
        plan["operations"][1]["payload"]["keyframes"] =
            json!({ "transform.scale": [{ "atMs": 100, "value": 1 }] });
        let parsed = parse_editable_motion_import_plan(&plan, "pkg_demo", "motion_demo").unwrap();
        let error = editable_planned_steps(&parsed).unwrap_err();
        assert!(error.message.contains("transform.scale"));

        let mut unknown = static_plan();
        unknown["operations"][0]["payload"]["shader"] = json!("untrusted");
        let parsed =
            parse_editable_motion_import_plan(&unknown, "pkg_demo", "motion_demo").unwrap();
        let error = editable_planned_steps(&parsed).unwrap_err();
        assert!(error.message.contains("shader"));
    }

    #[test]
    fn lowers_uniform_position_keyframes_to_normalized_native_tracks() {
        let mut source = static_plan();
        source["operations"][1]["payload"]["keyframes"] = json!({
            "transform.x": [
                { "atMs": 100, "value": -192, "easing": "ease-out" },
                { "atMs": 600, "value": 960 }
            ],
            "transform.y": [
                { "atMs": 100, "value": 108, "easing": "ease-out" },
                { "atMs": 600, "value": 540 }
            ]
        });
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        let pos_x = steps
            .iter()
            .find(|step| step["args"]["param"] == json!("pos_x"))
            .unwrap();
        let pos_y = steps
            .iter()
            .find(|step| step["args"]["param"] == json!("pos_y"))
            .unwrap();
        assert_eq!(pos_x["verb"], json!("edit.keyframe"));
        assert_eq!(pos_x["args"]["interp"], json!("ease_out_quad"));
        assert_eq!(pos_x["args"]["points"][0]["t_ms"], json!(0));
        assert_eq!(pos_x["args"]["points"][0]["value"], json!(-0.1));
        assert_eq!(pos_x["args"]["points"][1]["value"], json!(0.5));
        assert_eq!(pos_y["args"]["points"][1]["t_ms"], json!(500));
        assert_eq!(pos_y["args"]["points"][1]["value"], json!(0.5));

        source["operations"][1]["payload"]["keyframes"]["transform.x"][1]["easing"] = json!("hold");
        source["operations"][1]["payload"]["keyframes"]["transform.x"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "atMs": 900, "value": 1200 }));
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        assert!(editable_planned_steps(&plan)
            .unwrap_err()
            .message
            .contains("mixed easing"));
    }

    #[test]
    fn lowers_uniform_opacity_keyframes_to_clip_local_native_points() {
        let mut source = static_plan();
        source["operations"][1]["payload"]["opacity"] = json!(0.8);
        source["operations"][1]["payload"]["keyframes"] = json!({
            "opacity": [
                { "atMs": 100, "value": 0, "easing": "ease-out" },
                { "atMs": 600, "value": 0.8 }
            ]
        });
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[2]["verb"], json!("edit.keyframe"));
        assert_eq!(steps[2]["args"]["param"], json!("opacity"));
        assert_eq!(steps[2]["args"]["interp"], json!("ease_out_quad"));
        assert_eq!(steps[2]["args"]["points"][0]["t_ms"], json!(0));
        assert_eq!(steps[2]["args"]["points"][1]["t_ms"], json!(500));

        source["operations"][1]["payload"]["keyframes"]["opacity"][1]["easing"] = json!("hold");
        source["operations"][1]["payload"]["keyframes"]["opacity"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "atMs": 900, "value": 1 }));
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let error = editable_planned_steps(&plan).unwrap_err();
        assert!(error.message.contains("mixed easing"));
    }

    #[test]
    fn lowers_non_overlapping_uniform_fades_to_native_opacity_points() {
        let mut source = static_plan();
        source["operations"][0]["payload"]["opacity"] = json!(0.8);
        source["operations"][0]["payload"]["transitions"] = json!({
            "in": { "type": "fade", "durationMs": 500, "easing": "ease-in-out" },
            "out": { "type": "fade", "durationMs": 700, "easing": "ease-in-out" }
        });
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        assert_eq!(steps.len(), 3);
        let fade = steps
            .iter()
            .find(|step| {
                step["sourceLayerId"] == json!("panel") && step["verb"] == json!("edit.keyframe")
            })
            .unwrap();
        assert_eq!(fade["args"]["interp"], json!("ease_in_out_quad"));
        assert_eq!(
            fade["args"]["points"],
            json!([
                { "t_ms": 0, "value": 0 },
                { "t_ms": 500, "value": 0.8 },
                { "t_ms": 2300, "value": 0.8 },
                { "t_ms": 3000, "value": 0 }
            ])
        );

        source["operations"][0]["payload"]["transitions"]["out"]["easing"] = json!("ease-out");
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        assert!(editable_planned_steps(&plan)
            .unwrap_err()
            .message
            .contains("mixed easing"));

        source["operations"][0]["payload"]["transitions"]["out"]["easing"] = json!("ease-in-out");
        source["operations"][0]["payload"]["transitions"]["in"]["durationMs"] = json!(2500);
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        assert!(editable_planned_steps(&plan)
            .unwrap_err()
            .message
            .contains("overlap"));

        source["operations"][0]["payload"]["transitions"]["in"]["durationMs"] = json!(500);
        source["operations"][0]["payload"]["keyframes"] = json!({
            "opacity": [{ "atMs": 0, "value": 1 }]
        });
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        assert!(editable_planned_steps(&plan)
            .unwrap_err()
            .message
            .contains("conflict"));
    }

    #[test]
    fn lowers_cut_origin_video_to_native_insert_without_trusting_a_path() {
        let mut source = static_plan();
        source["operations"].as_array_mut().unwrap().push(json!({
            "verb": "cut.media.create",
            "sourceLayerId": "footage",
            "startMs": 500,
            "durationMs": 1000,
            "payload": {
                "kind": "video",
                "source": "cut-asset:a7",
                "fit": "cover",
                "trimStartMs": 250,
                "trimDurationMs": 1000,
                "playbackRate": 1,
                "includeAudio": false,
                "transform": { "scale": 1, "rotation": 0 },
                "style": {}
            }
        }));
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        let insert = steps
            .iter()
            .find(|step| step["sourceLayerId"] == json!("footage"))
            .unwrap();
        assert_eq!(insert["verb"], json!("edit.insert"));
        assert_eq!(insert["args"]["asset"], json!("a7"));
        assert_eq!(insert["args"]["track"], json!("v1"));
        assert_eq!(insert["args"]["at_ms"], json!(500));
        assert_eq!(insert["args"]["src_range_ms"], json!([250, 1250]));

        source["operations"][2]["payload"]["source"] = json!("/tmp/untrusted.mp4");
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        assert!(editable_planned_steps(&plan)
            .unwrap_err()
            .message
            .contains("Cut asset reference"));
    }

    #[test]
    fn lowers_cut_origin_audio_to_the_native_audio_track() {
        let mut source = static_plan();
        source["operations"].as_array_mut().unwrap().push(json!({
            "verb": "cut.audio.create",
            "sourceLayerId": "music",
            "startMs": 250,
            "durationMs": 1500,
            "payload": {
                "source": "cut-asset:a9",
                "trimStartMs": 500,
                "trimDurationMs": 1500,
                "playbackRate": 1,
                "volume": 1,
                "pan": 0,
                "muted": false,
                "loop": false,
                "normalizeLoudness": false
            }
        }));
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        let steps = editable_planned_steps(&plan).unwrap();
        let insert = steps
            .iter()
            .find(|step| step["sourceLayerId"] == json!("music"))
            .unwrap();
        assert_eq!(insert["verb"], json!("edit.insert"));
        assert_eq!(insert["args"]["asset"], json!("a9"));
        assert_eq!(insert["args"]["track"], json!("a1t"));
        assert_eq!(insert["args"]["src_range_ms"], json!([500, 2000]));

        source["operations"][2]["payload"]["volume"] = json!(0.5);
        let plan = parse_editable_motion_import_plan(&source, "pkg_demo", "motion_demo").unwrap();
        assert!(editable_planned_steps(&plan)
            .unwrap_err()
            .message
            .contains("audio processing"));
    }

    #[test]
    fn refuses_out_of_bounds_or_incomplete_geometry_instead_of_clamping_it() {
        let mut title = static_plan();
        title["operations"][1]["payload"]["transform"]["x"] = json!(2200);
        let parsed = parse_editable_motion_import_plan(&title, "pkg_demo", "motion_demo").unwrap();
        let error = editable_planned_steps(&parsed).unwrap_err();
        assert!(error.message.contains("outside the Cut canvas"));

        let mut incomplete = static_plan();
        incomplete["operations"][1]["payload"]["transform"]
            .as_object_mut()
            .unwrap()
            .remove("y");
        let parsed =
            parse_editable_motion_import_plan(&incomplete, "pkg_demo", "motion_demo").unwrap();
        let error = editable_planned_steps(&parsed).unwrap_err();
        assert!(error.message.contains("placement is incomplete"));

        let mut shape = static_plan();
        shape["operations"][0]["payload"]["transform"]["x"] = json!(1600);
        shape["operations"][0]["payload"]["transform"]["width"] = json!(600);
        let parsed = parse_editable_motion_import_plan(&shape, "pkg_demo", "motion_demo").unwrap();
        let error = editable_planned_steps(&parsed).unwrap_err();
        assert!(error.message.contains("bounds exceed"));
    }

    #[test]
    fn accepts_warned_success_but_rejects_failed_or_misbound_receipts() {
        let mut warned = static_plan();
        warned["receipt"]["status"] = json!("warning");
        warned["receipt"]["warnings"] = json!(["Static editable layer uses a fallback font"]);
        assert!(parse_editable_motion_import_plan(&warned, "pkg_demo", "motion_demo").is_ok());

        let mut plan = static_plan();
        plan["receipt"]["packageId"] = json!("pkg_other");
        assert!(parse_editable_motion_import_plan(&plan, "pkg_demo", "motion_demo").is_err());
        let mut plan = static_plan();
        plan["receipt"]["status"] = json!("failed");
        assert!(parse_editable_motion_import_plan(&plan, "pkg_demo", "motion_demo").is_err());

        let mut plan = static_plan();
        plan["unsupported"] =
            json!([{ "layerId": "title", "feature": "font", "reason": "unsupported" }]);
        assert!(parse_editable_motion_import_plan(&plan, "pkg_demo", "motion_demo").is_err());

        let mut plan = static_plan();
        plan["operations"][1]["sourceLayerId"] = json!("panel");
        let error =
            parse_editable_motion_import_plan(&plan, "pkg_demo", "motion_demo").unwrap_err();
        assert!(error.message.contains("repeats a source layer"));
    }
}
