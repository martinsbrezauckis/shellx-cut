//! Revision-bound Caption Find & Replace preview and atomic apply.
//!
//! Preview state is volatile and server-issued. Apply names only its opaque
//! hash, then materializes the reviewed batch as one reversible timeline edit.

use super::*;

mod plan;

use plan::{
    apply_ticket_to_tracks, attach_timing_refresh, build_plan, preview_hash, preview_result,
    validate_preview_args, ApplyArgs, PreviewArgs, PreviewTicket,
};

const RECEIPT_SCHEMA: &str = "shellx-cut/caption-bulk-apply/1";

pub(super) async fn captions_bulk_preview(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: PreviewArgs = parse_args(args)?;
    validate_preview_args(&args)?;
    // Supersede a prior review as soon as a new preview starts; a failed or
    // interrupted search must not leave an older target set applyable.
    *state.caption_bulk_preview.lock().await = None;

    let (project, project_dir, project_revision) = snapshot_project(state).await?;
    let mut plan = build_plan(&project, args)?;
    attach_timing_refresh(state, &mut plan).await;

    // Transcript evidence is read separately. Do not issue a preview when an
    // intervening edit changed the revision while that evidence was collected.
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let current_revision = current_revision(store)?;
    if current_revision != project_revision
        || store.dir.to_string_lossy().as_ref() != project_dir.as_str()
    {
        return stale_preview(&project_revision, &current_revision);
    }
    drop(guard);

    let preview_hash = preview_hash(&project_dir, &project_revision, &plan)?;
    let ticket = PreviewTicket {
        preview_hash: preview_hash.clone(),
        project_dir,
        project_revision: project_revision.clone(),
        plan: plan.clone(),
    };
    // A no-op preview remains useful feedback but cannot be sent to apply.
    if plan.affected_cue_count > 0 {
        *state.caption_bulk_preview.lock().await = Some(serde_json::to_value(ticket)?);
    }

    Ok(VerbResult::ok(preview_result(
        &plan,
        &project_revision,
        &preview_hash,
    )))
}

pub(super) async fn captions_bulk_apply(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let args: ApplyArgs = parse_args(args)?;
    let expected_revision = required_request_revision(&actor)?;

    // Keep the ticket through the commit so a second preview cannot swap a
    // reviewed target list between revalidation and materialization.
    let mut ticket_slot = state.caption_bulk_preview.lock().await;
    let Some(raw_ticket) = ticket_slot.clone() else {
        return Err(preview_unavailable());
    };
    let ticket: PreviewTicket = serde_json::from_value(raw_ticket).map_err(|error| {
        CutError::new(
            error_codes::CONFLICT,
            "caption replacement preview is unavailable",
            format!("the server preview ticket could not be read: {error}"),
        )
        .with_suggested_action("run captions.bulk_preview again before replacing captions")
    })?;
    if ticket.preview_hash != args.preview_hash {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "caption replacement preview no longer matches",
            "the submitted opaque preview hash is not the latest server-issued preview",
        )
        .with_suggested_action("run captions.bulk_preview again before replacing captions"));
    }
    if ticket.project_revision != expected_revision {
        *ticket_slot = None;
        return stale_preview(&expected_revision, &ticket.project_revision);
    }
    if args.refresh_timing && !ticket.plan.timing_refresh.available {
        return Err(CutError::new(
            error_codes::GUARDRAIL,
            "timing refresh is unavailable for this caption replacement",
            ticket
                .plan
                .timing_refresh
                .reason
                .clone()
                .unwrap_or_else(|| {
                    "exact transcript word ranges do not cover every changed cue".into()
                }),
        )
        .with_suggested_action(
            "apply the reviewed replacement with timing preserved, or refresh the transcript first",
        ));
    }

    let (op, post_revision, warnings) = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        let actual_revision = current_revision(store)?;
        if actual_revision != expected_revision {
            *ticket_slot = None;
            return stale_preview(&expected_revision, &actual_revision);
        }
        if store.dir.to_string_lossy().as_ref() != ticket.project_dir.as_str() {
            *ticket_slot = None;
            return Err(preview_unavailable());
        }

        let mut tracks = store.project.tracks.clone();
        apply_ticket_to_tracks(&mut tracks, &ticket.plan, args.refresh_timing)?;
        let steps = vec![InverseOp {
            verb: "edit._set_timeline".into(),
            args: json!({
                "tracks": tracks,
                "markers": store.project.markers,
                "caption_styles": store.project.caption_styles,
            }),
        }];
        let extra = vec![effect(
            Some(&ticket.plan.track),
            json!({
                "caption_bulk": {
                    "matched": ticket.plan.total_match_count,
                    "cues_replaced": ticket.plan.affected_cue_count,
                    "range_ms": ticket.plan.range_ms,
                    "timing_refreshed": args.refresh_timing,
                },
            }),
        )];
        let committed = guard_call("captions.bulk_apply", || {
            store.apply_lowered(
                "captions.bulk_apply",
                json!({
                    "preview_hash": ticket.preview_hash,
                    "refresh_timing": args.refresh_timing,
                }),
                actor,
                args.rationale.clone(),
                steps,
                extra,
            )
        })?;
        let post_revision = current_revision(store)?;
        let warnings = store.take_commit_warnings(std::slice::from_ref(&committed.op_id));
        (committed, post_revision, warnings)
    };
    // Request-control replays a durable receipt before this handler on retry;
    // clearing after commit therefore preserves request-key idempotency.
    *ticket_slot = None;
    drop(ticket_slot);
    state.events.publish(Event::OpApplied { op: op.clone() });

    Ok(VerbResult::ok_with_ops(
        json!({
            "schema": RECEIPT_SCHEMA,
            "pre_revision": expected_revision,
            "post_revision": post_revision,
            "preview_hash": args.preview_hash,
            "track": ticket.plan.track,
            "match_count": ticket.plan.total_match_count,
            "cues_replaced": ticket.plan.affected_cue_count,
            "timing_preserved": !args.refresh_timing,
            "timing_refreshed": args.refresh_timing,
            "undo": "one project.undo restores every replacement in this batch",
        }),
        vec![op.op_id],
    )
    .with_warnings(warnings))
}

async fn snapshot_project(
    state: &AppState,
) -> Result<(cut_core::Project, String, String), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    Ok((
        store.project.clone(),
        store.dir.to_string_lossy().into_owned(),
        current_revision(store)?,
    ))
}

fn current_revision(store: &cut_core::ProjectStore) -> Result<String, CutError> {
    store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "open project has no durable revision",
            "save or reopen the project before previewing a caption replacement",
        )
    })
}

fn required_request_revision(actor: &Actor) -> Result<String, CutError> {
    let request = actor.request.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "caption replacement requires request_id and expected_revision",
            "a reviewed batch needs durable retry identity and revision protection",
        )
        .with_suggested_action("preview again, then submit request_id plus expected_revision")
    })?;
    request.expected_revision.clone().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "caption replacement requires expected_revision",
            "a request_id without revision protection could apply a stale preview",
        )
    })
}

fn stale_preview<T>(expected: &str, actual: &str) -> Result<T, CutError> {
    Err(CutError::new(
        error_codes::CONFLICT,
        format!("caption replacement expected project revision '{expected}' but found '{actual}'"),
        "the caption preview is stale; no cues were changed",
    )
    .with_suggested_action("run captions.bulk_preview again and submit a new request_id"))
}

fn preview_unavailable() -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "caption replacement preview is unavailable",
        "the preview was consumed, superseded, or belongs to a previous server run; no cues were changed",
    )
    .with_suggested_action("run captions.bulk_preview again before replacing captions")
}
