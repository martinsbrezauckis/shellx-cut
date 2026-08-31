//! Revision-bound review of one durable compound action.
//!
//! This route is intentionally narrower than generic transaction replay. It
//! exposes only an existing contiguous `group_id` action and can reject it only
//! while it remains the active undo-history tip. The core appends one ordinary
//! replayable restore record, so a group rejection has the same durable shape
//! and undo semantics as the rest of Cut's operation log.

use super::*;
use cut_core::{AtomicGroupPreview, AtomicGroupRejectStatus};
use sha2::{Digest, Sha256};

const PREVIEW_SCHEMA: &str = "shellx-cut/compound-action-preview/1";
const RECEIPT_SCHEMA: &str = "shellx-cut/compound-action-reject/1";

#[derive(serde::Deserialize)]
struct PreviewArgs {
    op_id: String,
}

#[derive(serde::Deserialize)]
struct RejectArgs {
    first_op_id: String,
    last_op_id: String,
    preview_hash: String,
}

/// Inspect one adjacent durable `group_id` action without changing the project.
pub(super) async fn project_group_preview(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: PreviewArgs = parse_args(args)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let project_revision = current_revision(store)?;
    let preview = guard_call("project.group_preview", || {
        store.atomic_group_preview(&args.op_id)
    })?;
    Ok(VerbResult::ok(preview_result(&preview, &project_revision)?))
}

/// Append one normal restore record for the exact active compound action.
///
/// Unlike an individual `edit.restore{mode:"rebase"}`, this never skips or
/// reorders a set of historical edits. It is tip-only and restores the prefix
/// before the adjacent group as one atomic history entry.
pub(super) async fn project_group_reject(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let rationale = args
        .get("rationale")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let args: RejectArgs = parse_args(args)?;
    let expected_revision = required_request_revision(&actor)?;
    let (preview, op, post_revision, warnings) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        let actual_revision = current_revision(store)?;
        if actual_revision != expected_revision {
            return stale_preview(&expected_revision, &actual_revision);
        }
        let preview = guard_call("project.group_reject", || {
            store.atomic_group_preview(&args.first_op_id)
        })?;
        if preview.last_op_id != args.last_op_id {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "compound action boundary changed after its preview",
                format!(
                    "preview named final op '{}', but the current adjacent group ends at '{}'",
                    args.last_op_id, preview.last_op_id
                ),
            )
            .with_suggested_action("preview the compound action again before rejecting it"));
        }
        let actual_hash = preview_hash(&preview, &actual_revision)?;
        if actual_hash != args.preview_hash {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "compound action preview no longer matches the project",
                "the preview identity, revision, group boundary, or active history cursor changed",
            )
            .with_suggested_action("run project.group_preview again before rejecting the action"));
        }
        if !preview.is_reject_ready() {
            return not_current_tip(&preview);
        }
        let op = guard_call("project.group_reject", || {
            store.restore_atomic_group_tip(&preview, actor, rationale)
        })?;
        let post_revision = current_revision(store)?;
        let warnings = store.take_commit_warnings(std::slice::from_ref(&op.op_id));
        (preview, op, post_revision, warnings)
    };
    state.events.publish(Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "schema": RECEIPT_SCHEMA,
            "pre_revision": expected_revision,
            "post_revision": post_revision,
            "group": group_identity(&preview),
            "restored_op_id": preview.first_op_id,
            "restore_op_id": op.op_id,
            "scope": {
                "mode": "tip_only",
                "append_only": true,
                "replay": "materialized_prefix_before_group",
                "generic_transaction_replay": false,
                "undo": "one project.undo restores the complete group",
            },
        }),
        vec![op.op_id],
    )
    .with_warnings(warnings))
}

fn current_revision(store: &cut_core::ProjectStore) -> Result<String, CutError> {
    store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "open project has no durable revision",
            "the project journal has no current operation to bind this review action",
        )
        .with_suggested_action("close and reopen the project before reviewing compound actions")
    })
}

fn required_request_revision(actor: &Actor) -> Result<String, CutError> {
    let request = actor.request.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "compound rejection requires request_id and expected_revision",
            "a reviewed destructive action needs both exact retry identity and revision protection",
        )
        .with_suggested_action(
            "run project.group_preview, then submit its revision with a unique request_id",
        )
    })?;
    request.expected_revision.clone().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "compound rejection requires expected_revision",
            "a request_id alone cannot prove that the reviewed group is still current",
        )
    })
}

fn stale_preview(expected: &str, actual: &str) -> Result<VerbResult, CutError> {
    Err(CutError::new(
        error_codes::CONFLICT,
        format!("compound action expected project revision '{expected}' but found '{actual}'"),
        "the review preview is stale; no operation was appended",
    )
    .with_suggested_action("run project.group_preview again and submit a new request_id"))
}

fn not_current_tip(preview: &AtomicGroupPreview) -> Result<VerbResult, CutError> {
    let AtomicGroupRejectStatus::NotCurrentTip { current_tip_op_id } = &preview.reject_status
    else {
        unreachable!("ready preview was checked before this helper")
    };
    Err(CutError::new(
        error_codes::GUARDRAIL,
        "compound action is no longer the current undo step",
        format!(
            "the reviewed action ends at '{}', while the live history tip is '{}'",
            preview.last_op_id,
            current_tip_op_id.as_deref().unwrap_or("the project baseline")
        ),
    )
    .with_suggested_action(
        "review the newer edits; use project.undo for the latest action or preview this group again after returning to it",
    ))
}

fn preview_result(preview: &AtomicGroupPreview, project_revision: &str) -> Result<Value, CutError> {
    Ok(json!({
        "schema": PREVIEW_SCHEMA,
        "project_revision": project_revision,
        "preview_hash": preview_hash(preview, project_revision)?,
        "group": group_identity(preview),
        "reject": match &preview.reject_status {
            AtomicGroupRejectStatus::Ready => json!({
                "status": "ready",
                "mode": "tip_only",
                "message": "Rejecting this action appends one restore record; one undo restores the complete action.",
            }),
            AtomicGroupRejectStatus::NotCurrentTip { current_tip_op_id } => json!({
                "status": "not_current_tip",
                "current_tip_op_id": current_tip_op_id,
                "message": "Newer history exists, so this action cannot be rejected as one unit without discarding work.",
            }),
        },
    }))
}

fn group_identity(preview: &AtomicGroupPreview) -> Value {
    json!({
        "group_id": preview.group_id,
        "first_op_id": preview.first_op_id,
        "last_op_id": preview.last_op_id,
        "op_ids": preview.op_ids,
        "operation_count": preview.op_ids.len(),
        "sequence_id": preview.sequence_id,
    })
}

fn preview_hash(preview: &AtomicGroupPreview, project_revision: &str) -> Result<String, CutError> {
    let canonical = json!({
        "schema": PREVIEW_SCHEMA,
        "project_revision": project_revision,
        "group": group_identity(preview),
    });
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&canonical)?)
    ))
}
