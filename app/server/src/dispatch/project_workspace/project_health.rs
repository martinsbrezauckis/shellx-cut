//! Bounded, path-free project Health & Recovery read surface.
//!
//! This is deliberately separate from `project.state`: checking derived media
//! files is filesystem work, so a normal state sync must never pay for it. The
//! journal is validated before every page; a failed identity check returns an
//! honest journal-only report and refuses to describe stale membership.

use super::*;

mod media_page;
use media_page::{checked_media_page, unavailable_media_page};

pub(super) const DEFAULT_HEALTH_LIMIT: usize = 64;
pub(super) const MAX_HEALTH_LIMIT: usize = 128;

#[derive(serde::Deserialize, Default)]
struct HealthArgs {
    cursor: Option<String>,
    revision: Option<String>,
    limit: Option<usize>,
}

pub(super) async fn project_health(state: &AppState, args: Value) -> Result<VerbResult, CutError> {
    let args: HealthArgs = parse_args(args)?;
    let limit = health_limit(args.limit)?;
    if args.cursor.is_some() && args.revision.is_none() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "project.health cursor requires a project revision",
            "send the opaque project_revision returned by the first page with every cursor continuation",
        ));
    }

    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    let (revision, record_count) = match store.log.current_revision_and_count() {
        Ok(summary) => summary,
        Err(error) => return Ok(VerbResult::ok(unavailable_report(store, limit, &error))),
    };
    if let Some(expected) = args.revision.as_deref() {
        if revision.as_deref() != Some(expected) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "project changed while Health & Recovery was paging assets",
                "the supplied revision no longer matches the validated journal head",
            )
            .with_suggested_action("restart project.health without cursor or revision"));
        }
    }

    let media = checked_media_page(store, args.cursor.as_deref(), limit)?;
    let mut result = Map::new();
    result.insert("schema".into(), json!("shellx-cut/project-health/1"));
    if let Some(revision) = revision {
        result.insert("project_revision".into(), json!(revision));
    }
    result.insert(
        "journal".into(),
        journal_report(store, "verified", Some(record_count), None),
    );
    result.insert("media".into(), media);
    Ok(VerbResult::ok(Value::Object(result)))
}

fn health_limit(limit: Option<usize>) -> Result<usize, CutError> {
    let limit = limit.unwrap_or(DEFAULT_HEALTH_LIMIT);
    if (1..=MAX_HEALTH_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Health & Recovery page limit is outside the supported range",
            format!("limit must be an integer from 1 through {MAX_HEALTH_LIMIT}"),
        ))
    }
}

fn unavailable_report(store: &ProjectStore, limit: usize, error: &CutError) -> Value {
    let status = if error.code == error_codes::CONFLICT {
        "attention"
    } else {
        "unavailable"
    };
    json!({
        "schema": "shellx-cut/project-health/1",
        "journal": journal_report(store, status, None, Some(error)),
        "media": unavailable_media_page(limit),
    })
}

fn journal_report(
    store: &ProjectStore,
    live_status: &str,
    record_count: Option<usize>,
    identity_error: Option<&CutError>,
) -> Value {
    let open = store.open_health();
    let recovered = open.journal_tail_recovery.is_some()
        || open.cache == cut_core::ProjectCacheHealth::Rebuilt
        || open.snapshot == cut_core::ProjectSnapshotHealth::Rejected;
    let status = if identity_error.is_some() {
        live_status
    } else if recovered {
        "recovered"
    } else {
        live_status
    };
    let mut notices = Vec::new();
    if let Some(recovery) = &open.journal_tail_recovery {
        notices.push(json!({
            "code": "journal_tail_recovered",
            "message": "A malformed final journal record was quarantined while this project opened.",
            "discarded_bytes": recovery.discarded_end.saturating_sub(recovery.discarded_start),
            "discarded_start": recovery.discarded_start,
            "discarded_end": recovery.discarded_end,
        }));
    }
    if open.cache == cut_core::ProjectCacheHealth::Rebuilt {
        notices.push(json!({
            "code": "project_cache_rebuilt",
            "message": "The disposable project cache did not match journal replay and was rebuilt from the journal.",
        }));
    }
    if open.snapshot == cut_core::ProjectSnapshotHealth::Rejected {
        notices.push(json!({
            "code": "history_snapshot_rejected",
            "message": "A disposable history snapshot failed verification and was ignored.",
        }));
    }
    if identity_error.is_some() {
        notices.push(json!({
            "code": "identity_revalidation_failed",
            "message": "The journal cannot be safely revalidated now; close and reopen the project before relying on project health.",
        }));
    }
    let mut journal = Map::new();
    journal.insert("status".into(), json!(status));
    journal.insert("cache".into(), json!(open.cache.as_str()));
    let mut snapshot = Map::new();
    snapshot.insert("status".into(), json!(open.snapshot.as_str()));
    if let Some(prefix_ops) = open.snapshot.prefix_ops() {
        snapshot.insert("prefix_ops".into(), json!(prefix_ops));
    }
    journal.insert("snapshot".into(), Value::Object(snapshot));
    if let Some(record_count) = record_count {
        journal.insert("log_records".into(), json!(record_count));
    }
    journal.insert("notices".into(), Value::Array(notices));
    Value::Object(journal)
}

#[cfg(test)]
mod tests;
