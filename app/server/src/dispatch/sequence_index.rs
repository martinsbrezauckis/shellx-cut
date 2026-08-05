//! Read-only cross-timeline clip and marker metadata search.

use super::*;

mod rows;

use rows::{collect_rows, RowFilters};

#[derive(serde::Deserialize)]
struct Args {
    #[serde(default)]
    query: String,
    #[serde(default = "default_kind")]
    kind: String,
    sequence: Option<String>,
    track_kind: Option<String>,
    #[serde(default = "default_status")]
    status: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_kind() -> String {
    "all".into()
}

fn default_limit() -> usize {
    200
}

fn default_status() -> String {
    "all".into()
}

fn validate_args(args: &Args) -> Result<(), CutError> {
    if !matches!(args.kind.as_str(), "all" | "clip" | "marker") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "kind must be all, clip, or marker",
            "use a project.sequence_index kind from schema/verbs.json",
        ));
    }
    if args.limit == 0 || args.limit > 500 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "limit must be between 1 and 500",
            "omit limit for the 200-row default",
        ));
    }
    if let Some(kind) = args.track_kind.as_deref() {
        if !matches!(kind, "video" | "audio" | "caption") {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "track_kind must be video, audio, or caption",
                "omit track_kind to search clips on every track",
            ));
        }
    }
    if !matches!(
        args.status.as_str(),
        "all" | "issues" | "offline" | "gaps" | "effects" | "hidden" | "locked" | "muted"
    ) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "status must be all, issues, offline, gaps, effects, hidden, locked, or muted",
            "use a project.sequence_index status from schema/verbs.json",
        ));
    }
    Ok(())
}

/// Work from a clone so legacy projects can materialize their implicit Main
/// sequence for this response without changing live state or appending an op.
pub(super) async fn project_sequence_index(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    let args: Args = parse_args(args)?;
    validate_args(&args)?;

    let (project_dir, mut project) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        (store.dir.clone(), store.project.clone())
    };
    project.ensure_sequence_bank();
    project.sync_active_sequence();
    if let Some(id) = args.sequence.as_deref() {
        if !project.sequences.iter().any(|sequence| sequence.id == id) {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!("sequence '{id}' does not exist"),
                "call project.sequence_list and pass one of its sequence ids",
            ));
        }
    }

    let query = args.query.trim().to_string();
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .collect();
    let query_requests_gaps = terms
        .iter()
        .any(|term| matches!(term.as_str(), "gap" | "gaps"));
    let filters = RowFilters {
        kind: &args.kind,
        sequence: args.sequence.as_deref(),
        track_kind: args.track_kind.as_deref(),
        status: &args.status,
        terms: &terms,
        include_gaps: matches!(args.status.as_str(), "issues" | "gaps") || query_requests_gaps,
    };
    let mut rows = collect_rows(&project_dir, &project, filters);
    rows.sort_by(|left, right| {
        (left.0, left.1, left.2, &left.3).cmp(&(right.0, right.1, right.2, &right.3))
    });

    let clip_count = rows
        .iter()
        .filter(|(_, _, _, _, row)| row["kind"] == "clip")
        .count();
    let marker_count = rows.len().saturating_sub(clip_count);
    let issue_count = rows
        .iter()
        .filter(|(_, _, _, _, row)| {
            row.get("issues")
                .and_then(Value::as_array)
                .is_some_and(|issues| !issues.is_empty())
        })
        .count();
    let effect_clip_count = rows
        .iter()
        .filter(|(_, _, _, _, row)| {
            row.get("effect_count")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0)
        })
        .count();
    let total = rows.len();
    let truncated = total > args.limit;
    let results: Vec<Value> = rows
        .into_iter()
        .take(args.limit)
        .map(|(_, _, _, _, row)| row)
        .collect();

    Ok(VerbResult::ok(json!({
        "query": query,
        "kind": args.kind,
        "sequence": args.sequence,
        "track_kind": args.track_kind,
        "status": args.status,
        "total": total,
        "clip_count": clip_count,
        "marker_count": marker_count,
        "issue_count": issue_count,
        "effect_clip_count": effect_clip_count,
        "truncated": truncated,
        "results": results,
    })))
}
