//! Bounded revision-aware project state and operation-log synchronization.

use super::*;

pub(super) const DEFAULT_SYNC_LIMIT: usize = 128;
pub(super) const MAX_SYNC_LIMIT: usize = 512;
pub(super) const MAX_SYNC_BYTES: usize = 512 * 1024;

#[derive(serde::Deserialize, Default)]
struct OpsArgs {
    since: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(serde::Deserialize, Default)]
struct StateArgs {
    since_revision: Option<String>,
    limit: Option<usize>,
}

fn bounded_limit(limit: Option<usize>) -> Result<usize, CutError> {
    let limit = limit.unwrap_or(DEFAULT_SYNC_LIMIT);
    if (1..=MAX_SYNC_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            "sync page limit is outside the supported range",
            format!("limit must be an integer from 1 through {MAX_SYNC_LIMIT}"),
        ))
    }
}

pub(super) async fn project_ops(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: OpsArgs = parse_args(args)?;
    if args.since.is_some() && args.cursor.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project.ops accepts either since or cursor, not both",
            "since is the compatibility checkpoint/ref cursor; cursor is a raw op id returned by a prior page",
        ));
    }
    let limit = bounded_limit(args.limit)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let cursor = match args.since.as_deref() {
        None => args.cursor,
        Some(reference) => Some(cut_core::diff::resolve_ref(
            &store.project,
            &reference.to_string(),
        )?),
    };
    let page = store
        .log
        .page_after(cursor.as_deref(), limit, MAX_SYNC_BYTES)?;
    Ok(VerbResult::ok(json!({
        "ops": page.ops,
        "cursor": cursor,
        "next_cursor": page.next_cursor,
        "has_more": page.has_more,
        "limit": limit,
        "encoded_bytes": page.encoded_bytes,
        "undo_available": store.undo_available(),
        "redo_available": store.redo_available(),
        "project_revision": store.log.current_revision()?,
    })))
}

pub(super) async fn project_state(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: StateArgs = parse_args(args)?;
    let limit = bounded_limit(args.limit)?;
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let Some(since_revision) = args.since_revision else {
        return snapshot(store, "cold");
    };
    let page = match store
        .log
        .page_after(Some(&since_revision), limit, MAX_SYNC_BYTES)
    {
        Ok(page) => page,
        Err(error) if error.code == error_codes::NOT_FOUND => {
            return snapshot(store, "invalid_revision")
        }
        // A single valid operation can exceed the delta byte cap. It is not a
        // malformed request: return the canonical full-state fallback rather
        // than leaving a reconnect unable to converge.
        Err(error) if error.code == error_codes::INVALID_ARGS => {
            return snapshot(store, "delta_too_large")
        }
        Err(error) => return Err(error),
    };
    if page.has_more {
        return snapshot(store, "too_old");
    }
    let Some((changes, affected)) = bounded_changes(&page.ops) else {
        return snapshot(store, "unsupported_delta");
    };
    Ok(VerbResult::ok(json!({
        "sync": {
            "mode": "delta",
            "from_revision": since_revision,
            "project_revision": store.log.current_revision()?,
            "ops": page.ops,
            "changes": changes,
            "affected": affected,
            "encoded_bytes": page.encoded_bytes,
        }
    })))
}

fn snapshot(store: &ProjectStore, reason: &str) -> Result<VerbResult, CutError> {
    Ok(VerbResult::ok(super::full_project_state(
        store,
        json!({
            "mode": "snapshot",
            "reason": reason,
            "project_revision": store.log.current_revision()?,
        }),
    )?))
}

/// Return only state changes the UI can apply without replaying Cut's full
/// editor reducer. Any verb outside this narrow, lossless set returns `None`,
/// forcing the truthful full-state fallback instead of guessing at timeline
/// mutations such as ripple edits.
fn bounded_changes(ops: &[OpRecord]) -> Option<(Vec<Value>, Value)> {
    let mut changes = Vec::with_capacity(ops.len());
    let mut markers = 0usize;
    let mut assets = 0usize;
    let mut project = 0usize;
    for op in ops {
        let change = match op.verb.as_str() {
            "edit.add_marker" => {
                let marker = op
                    .effects
                    .iter()
                    .find_map(|effect| effect.detail.get("added_marker"))?
                    .clone();
                markers += 1;
                json!({"kind":"marker_upsert", "marker": marker})
            }
            "edit.remove_marker" => {
                let id = op.args.get("id")?.as_str()?;
                markers += 1;
                json!({"kind":"marker_remove", "id": id})
            }
            "media.import" => {
                let detail = op.effects.iter().find_map(|effect| {
                    effect
                        .detail
                        .get("asset_id")
                        .zip(effect.detail.get("asset"))
                })?;
                assets += 1;
                json!({"kind":"asset_upsert", "id": detail.0, "asset": detail.1})
            }
            "media.remove" => {
                let id = op
                    .effects
                    .iter()
                    .find_map(|effect| effect.detail.get("asset_id"))?
                    .as_str()?;
                assets += 1;
                json!({"kind":"asset_remove", "id": id})
            }
            "project.rename" => {
                let name = op.args.get("name")?.as_str()?;
                project += 1;
                json!({"kind":"project_name", "name": name})
            }
            _ => return None,
        };
        changes.push(change);
    }
    Some((
        changes,
        json!({"markers": markers, "assets": assets, "project": project}),
    ))
}

#[cfg(test)]
mod tests;
