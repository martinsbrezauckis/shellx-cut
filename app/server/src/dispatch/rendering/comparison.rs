//! Revision-bound frame-pair rendering for the Preview monitor.
//!
//! This is deliberately a read-only review surface: it snapshots the current
//! durable log head, replays its immediately preceding compatible prefix, and
//! renders both compositions at one exact timeline position. It never changes
//! the open ProjectStore or asks the undo stack to manufacture a before state.

use super::*;

const COMPARISON_MAX_HEIGHT: u32 = 1080;

#[derive(serde::Deserialize)]
struct Args {
    at_ms: u64,
    /// The UI's observed durable head. This is an input identity, not the
    /// generic mutation-control `expected_revision`: comparison is read-only.
    revision: String,
    h: Option<u32>,
}

struct Snapshots {
    current: cut_core::Project,
    current_edl: cut_core::Edl,
    current_revision: String,
    prior: cut_core::Project,
    prior_edl: cut_core::Edl,
    prior_revision: String,
    compared_operation_id: String,
    compared_operation_verb: String,
    project_dir: PathBuf,
}

fn stale_revision(expected: &str, actual: Option<&str>) -> CutError {
    CutError::new(
        error_codes::CONFLICT,
        "the project changed before comparison could start",
        format!(
            "comparison requested revision '{expected}', but the durable head is '{}'",
            actual.unwrap_or("none")
        ),
    )
    .with_suggested_action("wait for the current edit to settle, then compare again")
}

fn no_prior(message: impl Into<String>, cause: impl Into<String>, at_ms: u64) -> CutError {
    CutError::new(error_codes::NOT_FOUND, message, cause)
        .with_at_ms(at_ms)
        .with_suggested_action("make an edit with visible media at this time, then compare again")
}

/// The frame pair compares the current durable head with the prefix immediately
/// before its latest timeline mutation. Metadata may trail that edit — it does
/// not change pixels — but it must remain in the *current* head identity.
/// Using the schema-derived `mutates_timeline` classification avoids a fragile
/// verb-name list and fails closed for a journal this Cut cannot classify.
fn snapshots_from_store(
    store: &ProjectStore,
    expected_revision: &str,
    at_ms: u64,
) -> Result<Snapshots, CutError> {
    let current_revision = store.log.current_revision()?.ok_or_else(|| {
        no_prior(
            "there is no saved edit to compare",
            "the project journal is empty",
            at_ms,
        )
    })?;
    if current_revision != expected_revision {
        return Err(stale_revision(expected_revision, Some(&current_revision)));
    }
    let ops = store.log.read_all()?;
    let mut timeline_index = None;
    for (index, op) in ops.iter().enumerate().rev() {
        if op.mutates_timeline()? {
            timeline_index = Some(index);
            break;
        }
    }
    let timeline_index = timeline_index.ok_or_else(|| {
        no_prior(
            "there is no timeline edit to compare",
            "the durable journal has no timeline-mutating operation",
            at_ms,
        )
    })?;
    if timeline_index == 0 {
        return Err(no_prior(
            "there is no earlier timeline state to compare",
            "the latest timeline edit has no materializable journal prefix",
            at_ms,
        ));
    }
    let compared = &ops[timeline_index];
    let prior_revision = ops[timeline_index - 1].op_id.clone();
    let prior = cut_core::rebuild_from_log(&ops[..timeline_index])?;
    let current = store.project.clone();

    // A change of active sequence, frame cadence, or output geometry makes a
    // side-by-side frame pair look comparable when it is not. Refuse instead
    // of scaling two unrelated timeline interpretations into one monitor.
    if current.active_sequence != prior.active_sequence
        || current.settings.width != prior.settings.width
        || current.settings.height != prior.settings.height
        || current.settings.fps != prior.settings.fps
    {
        return Err(no_prior(
            "the previous edit uses a different timeline view",
            "active sequence, frame rate, or frame size changed between the two revisions",
            at_ms,
        ));
    }

    let current_edl = cut_core::edl_from_project(&current);
    let prior_edl = cut_core::edl_from_project(&prior);
    if current_edl.duration_ms <= at_ms || prior_edl.duration_ms <= at_ms {
        return Err(no_prior(
            "the previous edit has no comparable frame here",
            format!(
                "frame {} ms is outside the current {} ms or previous {} ms composition",
                at_ms, current_edl.duration_ms, prior_edl.duration_ms
            ),
            at_ms,
        ));
    }

    Ok(Snapshots {
        current,
        current_edl,
        current_revision,
        prior,
        prior_edl,
        prior_revision,
        compared_operation_id: compared.op_id.clone(),
        compared_operation_verb: compared.verb.clone(),
        project_dir: store.dir.clone(),
    })
}

async fn ensure_current_head(
    state: &AppState,
    expected_revision: &str,
    expected_project_dir: &std::path::Path,
) -> Result<(), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    if store.dir != expected_project_dir {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the open project changed before comparison was ready",
            "a different project became active while the historical frames were rendering",
        )
        .with_suggested_action("open comparison again from the current project"));
    }
    let actual = store.log.current_revision()?;
    if actual.as_deref() == Some(expected_revision) {
        Ok(())
    } else {
        Err(stale_revision(expected_revision, actual.as_deref()))
    }
}

async fn flatten_snapshot(
    project: cut_core::Project,
    edl: cut_core::Edl,
    dir: PathBuf,
    context: &'static str,
) -> Result<(cut_core::Project, cut_core::Edl), CutError> {
    run_blocking(context, move || {
        crate::nest::flatten_for_media_io(&project, &edl, &dir)
    })
    .await
}

fn frame_payload(revision: &str, bytes: &[u8], height: u32) -> serde_json::Value {
    use base64::Engine;
    let (width, actual_height) = jpeg_dimensions(bytes).unwrap_or((0, height));
    json!({
        "revision": revision,
        "mime": "image/jpeg",
        "width": width,
        "height": actual_height,
        "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

/// `render.compare{at_ms, revision, h?}` → a locked exact before/current pair.
///
/// One comparison owns one of the existing frame-render admission slots and
/// renders sequentially, so two concurrent comparisons can never start more
/// than the global two ffmpeg frame jobs. `scrub_frame_bytes_from_snapshot`
/// keeps the derived LRU key bound to each precise historical head (`dir` +
/// `at_op`), preventing prior/current bytes from colliding.
pub(crate) async fn render_compare(
    state: &AppState,
    args: serde_json::Value,
) -> Result<VerbResult, CutError> {
    let args: Args = parse_args(args)?;
    let height = args.h.unwrap_or(cut_media::render::SCRUB_DEFAULT_HEIGHT);
    if height > COMPARISON_MAX_HEIGHT {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("comparison frame height must not exceed {COMPARISON_MAX_HEIGHT}px"),
            "comparison renders two exact frames, so its per-frame preview budget is capped at 1080px",
        )
        .with_suggested_action("request h <= 1080"));
    }
    checked_preview_height(height.max(2), 16, 9)?;

    let snapshots = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        snapshots_from_store(store, &args.revision, args.at_ms)?
    };

    // Acquire exactly one admission slot for the whole pair. This stays held
    // through optional nest flattening as well: a comparison cannot flood the
    // renderer with two independent submissions or wait indefinitely in a
    // queue behind another pair.
    let _permit = state
        .frame_render_limiter
        .clone()
        .try_acquire_owned()
        .map_err(|_| frame_render_busy())?;

    let (prior, prior_edl) = flatten_snapshot(
        snapshots.prior,
        snapshots.prior_edl,
        snapshots.project_dir.clone(),
        "render.compare.prior.nests",
    )
    .await?;
    let (current, current_edl) = flatten_snapshot(
        snapshots.current,
        snapshots.current_edl,
        snapshots.project_dir.clone(),
        "render.compare.current.nests",
    )
    .await?;

    // compose:true is non-negotiable: fast proxy scrub frames omit the layers
    // that this control exists to review. Each snapshot's revision is part of
    // the cache identity, so the two equal-time requests cannot reuse pixels.
    let (before, before_fast) = scrub_frame_bytes_from_snapshot(
        state,
        prior,
        prior_edl,
        snapshots.project_dir.clone(),
        snapshots.prior_revision.clone(),
        args.at_ms,
        height,
        true,
        FrameResourceClass::Preview,
    )
    .await?;
    let (after, after_fast) = scrub_frame_bytes_from_snapshot(
        state,
        current,
        current_edl,
        snapshots.project_dir.clone(),
        snapshots.current_revision.clone(),
        args.at_ms,
        height,
        true,
        FrameResourceClass::Preview,
    )
    .await?;
    if before_fast || after_fast {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "comparison could not produce exact composed frames",
            "the renderer attempted a proxy-only frame despite compose:true",
        ));
    }

    // A project edit may land while ffmpeg works. Do not return an apparently
    // current pair in that race; the UI retains the locked old pair only when
    // it still names the live durable head.
    ensure_current_head(state, &snapshots.current_revision, &snapshots.project_dir).await?;

    Ok(VerbResult::ok(json!({
        "schema": "shellx-cut/preview-comparison/1",
        "at_ms": args.at_ms,
        "current_revision": snapshots.current_revision,
        "prior_revision": snapshots.prior_revision,
        "compared_operation": {
            "id": snapshots.compared_operation_id,
            "verb": snapshots.compared_operation_verb,
        },
        "before": frame_payload(&snapshots.prior_revision, &before, height),
        "current": frame_payload(&snapshots.current_revision, &after, height),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_height_is_stricter_than_one_frame_preview_budget() {
        const { assert!(COMPARISON_MAX_HEIGHT < FRAME_PREVIEW_MAX_HEIGHT) };
        assert!(checked_preview_height(COMPARISON_MAX_HEIGHT, 16, 9).is_ok());
    }

    #[test]
    fn stale_head_names_both_identities() {
        let error = stale_revision("op_000010", Some("op_000011"));
        assert_eq!(error.code, error_codes::CONFLICT);
        assert!(error.cause.contains("op_000010"));
        assert!(error.cause.contains("op_000011"));
    }
}
