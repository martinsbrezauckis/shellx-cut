//! The one atomic lowered edit that turns a reviewed plan into timeline state.

use super::super::*;
use super::binding::{
    ensure_apply_binding_current, ensure_binding, ensure_plan_matches, ensure_transcript_matches,
    require_apply_request, transcript_content_sha256, Materialization, PlanBinding,
};
use super::lowering::build_materialization_plan;

pub(in crate::dispatch::speech_text) async fn apply_planned_ranges(
    state: &AppState,
    actor: Actor,
    binding: PlanBinding,
    verb: &str,
    asset: &str,
    selected_ranges: Vec<[usize; 2]>,
    materialization: Materialization,
    op_args: Value,
) -> Result<VerbResult, CutError> {
    require_apply_request(&actor, &binding)?;
    ensure_plan_matches(&binding, verb, asset, &selected_ranges, &materialization)?;
    if selected_ranges.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "the reviewed Assemble plan has no matched transcript ranges",
            "review a plan with at least one matched range before applying it",
        ));
    }

    // Refuse a stale or foreign plan before reading its asset-side transcript.
    // The same check is repeated inside the write lock before the atomic commit.
    ensure_apply_binding_current(state, &binding).await?;

    // Read once before taking the write lock. The binding is rechecked under
    // that lock before any lower step is built or committed.
    let transcript = load_transcript(state, asset).await?;
    let transcript_sha256 = transcript_content_sha256(&transcript)?;
    ensure_transcript_matches(&binding, &transcript_sha256)?;
    let (op, receipt) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        ensure_binding(store, &binding)?;
        ensure_transcript_matches(&binding, &transcript_sha256)?;
        let plan = build_materialization_plan(
            store,
            asset,
            &transcript.words,
            &selected_ranges,
            &materialization,
        )?;
        let op = guard_call("assemble.apply", || {
            store.apply_lowered(
                verb,
                op_args,
                actor,
                Some(format!(
                    "assemble {} from reviewed transcript plan",
                    materialization.label()
                )),
                plan.steps,
                plan.effects,
            )
        })?;
        (op, plan.receipt)
    };

    let op_id = op.op_id.clone();
    state.events.publish(Event::OpApplied { op });
    Ok(VerbResult::ok_with_ops(
        json!({
            "materialized": true,
            "kind": materialization.label(),
            "asset": asset,
            "spans_placed": receipt.spans_placed,
            "video_track": receipt.video_track,
            "audio_track": receipt.audio_track,
            "video_clip_ids": receipt.video_clip_ids,
            "audio_clip_ids": receipt.audio_clip_ids,
            "caption_track": receipt.caption_track,
            "caption_clip_ids": receipt.caption_clip_ids,
            "total_ms": receipt.total_ms,
            "undo": {"verb": "project.undo", "op_id": op_id},
        }),
        vec![op_id],
    ))
}
