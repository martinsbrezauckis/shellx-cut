//! Pure caption-bulk plan construction, evidence checks, and revalidation.

use crate::{dispatch::harvest_timeline_words, state::AppState};
use cut_core::{error_codes, CutError, Track};
use regex::{NoExpand, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub(super) const PREVIEW_SCHEMA: &str = "shellx-cut/caption-bulk-preview/1";
pub(super) const MAX_PREVIEW_ROWS: usize = 100;
/// Full private review target ceiling; larger scopes fail closed before a ticket.
pub(super) const MAX_APPLY_CUES: usize = 5_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct PreviewArgs {
    pub(super) track: String,
    pub(super) find: String,
    pub(super) replace_with: String,
    pub(super) match_mode: MatchMode,
    #[serde(default)]
    pub(super) case_sensitive: bool,
    pub(super) range_ms: Option<[u64; 2]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MatchMode {
    Contains,
    WholeWord,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ApplyArgs {
    pub(super) preview_hash: String,
    #[serde(default)]
    pub(super) refresh_timing: bool,
    pub(super) rationale: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MatchRow {
    cue_id: String,
    range_ms: [u64; 2],
    text: String,
    replacement_text: String,
    /// UTF-16 offsets preserve highlight positions for JavaScript strings.
    match_ranges_utf16: Vec<[usize; 2]>,
    refreshed_range_ms: Option<[u64; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BulkPlan {
    pub(super) track: String,
    find: String,
    replace_with: String,
    match_mode: MatchMode,
    case_sensitive: bool,
    pub(super) range_ms: Option<[u64; 2]>,
    pub(super) total_match_count: usize,
    pub(super) affected_cue_count: usize,
    /// Complete, private, revision-bound target set; output rows are bounded.
    targets: Vec<MatchRow>,
    pub(super) timing_refresh: TimingRefresh,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TimingRefresh {
    pub(super) available: bool,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PreviewTicket {
    pub(super) preview_hash: String,
    pub(super) project_dir: String,
    pub(super) project_revision: String,
    pub(super) plan: BulkPlan,
}

pub(super) fn validate_preview_args(args: &PreviewArgs) -> Result<(), CutError> {
    if args.track.trim().is_empty() || args.find.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "caption find and track are required",
            "choose a caption track and enter at least one search character",
        ));
    }
    if let Some([start, end]) = args.range_ms {
        if end <= start {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "caption replacement range must have an end after its start",
                "use an absolute non-empty [start_ms, end_ms) timeline range",
            ));
        }
    }
    Ok(())
}

pub(super) fn build_plan(
    project: &cut_core::Project,
    args: PreviewArgs,
) -> Result<BulkPlan, CutError> {
    let track = project.track(&args.track).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("caption track '{}' was not found", args.track),
            "read project.state and choose a live caption-kind track",
        )
    })?;
    if track.kind != cut_core::TrackKind::Caption {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("track '{}' is not a caption track", args.track),
            "Caption Find & Replace only changes cues on one caption-kind track",
        ));
    }
    let matcher = matcher_for(&args)?;
    let mut total_match_count = 0usize;
    let mut targets = Vec::new();
    for clip in &track.clips {
        let cut_core::Clip::Caption(cue) = clip else {
            continue;
        };
        if !overlaps_scope(cue.range_ms, args.range_ms) {
            continue;
        }
        let match_ranges_utf16 = match_ranges_utf16(&matcher, &cue.text);
        if match_ranges_utf16.is_empty() {
            continue;
        }
        total_match_count += match_ranges_utf16.len();
        let replacement_text = matcher
            .replace_all(&cue.text, NoExpand(&args.replace_with))
            .into_owned();
        if replacement_text == cue.text {
            continue;
        }
        if targets.len() == MAX_APPLY_CUES {
            return Err(CutError::new(
                error_codes::GUARDRAIL,
                "caption replacement scope is too large to review safely",
                format!("the scope exceeds the {MAX_APPLY_CUES}-cue server review limit; no preview ticket was issued"),
            ).with_suggested_action("choose a narrower timeline range or a more specific search, then preview again"));
        }
        targets.push(MatchRow {
            cue_id: cue.id.clone(),
            range_ms: cue.range_ms,
            text: cue.text.clone(),
            replacement_text,
            match_ranges_utf16,
            refreshed_range_ms: None,
        });
    }
    let affected_cue_count = targets.len();
    Ok(BulkPlan {
        track: args.track, find: args.find, replace_with: args.replace_with,
        match_mode: args.match_mode, case_sensitive: args.case_sensitive, range_ms: args.range_ms,
        total_match_count, affected_cue_count, targets,
        timing_refresh: TimingRefresh { available: false, reason: Some("Timing is preserved by default. No changed caption rows are available for an evidence-backed refresh.".into()) },
    })
}

pub(super) async fn attach_timing_refresh(state: &AppState, plan: &mut BulkPlan) {
    if plan.affected_cue_count == 0 {
        return;
    }
    let words = match harvest_timeline_words(state).await {
        Ok(words) if !words.is_empty() => words,
        Ok(_) => {
            plan.timing_refresh.reason = Some("Timing refresh is unavailable because this timeline has no transcript word ranges.".into());
            return;
        }
        Err(error) => {
            plan.timing_refresh.reason = Some(format!("Timing refresh is unavailable because exact transcript word evidence could not be read: {}", error.message));
            return;
        }
    };
    for row in &mut plan.targets {
        let evidence = words
            .iter()
            .filter(|(start, end, _)| *start < row.range_ms[1] && *end > row.range_ms[0])
            .collect::<Vec<_>>();
        let words_text = evidence
            .iter()
            .map(|(_, _, word)| word.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if evidence.is_empty() || normalized_words(&words_text) != normalized_words(&row.text) {
            plan.timing_refresh.reason = Some(format!("Timing refresh is unavailable because cue '{}' is not covered by an exact transcript word sequence.", row.cue_id));
            return;
        }
        row.refreshed_range_ms = Some([evidence[0].0, evidence.last().expect("non-empty").1]);
    }
    plan.timing_refresh = TimingRefresh {
        available: true,
        reason: Some("Exact transcript word ranges cover every changed cue. Refresh timing will align each cue to those word edges.".into()),
    };
}

pub(super) fn preview_result(plan: &BulkPlan, project_revision: &str, preview_hash: &str) -> Value {
    let omitted_rows = plan.affected_cue_count.saturating_sub(MAX_PREVIEW_ROWS);
    json!({
        "schema": PREVIEW_SCHEMA, "project_revision": project_revision, "preview_hash": preview_hash,
        "scope": {
            "track": plan.track, "range_ms": plan.range_ms,
            "summary": match plan.range_ms {
                Some([start, end]) => format!("{} · {}–{} ms", plan.track, start, end),
                None => format!("{} · entire timeline", plan.track),
            },
        },
        "query": {
            "find": plan.find, "replace_with": plan.replace_with,
            "match_mode": match &plan.match_mode { MatchMode::Contains => "contains", MatchMode::WholeWord => "whole_word" },
            "case_sensitive": plan.case_sensitive,
        },
        "match_count": plan.total_match_count, "affected_cue_count": plan.affected_cue_count,
        "rows": plan.targets.iter().take(MAX_PREVIEW_ROWS).collect::<Vec<_>>(),
        "rows_limit": MAX_PREVIEW_ROWS, "omitted_rows": omitted_rows,
        "can_apply": plan.affected_cue_count > 0, "timing_refresh": plan.timing_refresh,
    })
}

pub(super) fn preview_hash(
    project_dir: &str,
    project_revision: &str,
    plan: &BulkPlan,
) -> Result<String, CutError> {
    let identity = json!({ "schema": PREVIEW_SCHEMA, "project_dir": project_dir, "project_revision": project_revision, "plan": plan });
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&identity)?)
    ))
}

pub(super) fn apply_ticket_to_tracks(
    tracks: &mut [Track],
    plan: &BulkPlan,
    refresh_timing: bool,
) -> Result<(), CutError> {
    if plan.affected_cue_count == 0 || plan.targets.len() != plan.affected_cue_count {
        return Err(preview_unavailable());
    }
    let track = tracks
        .iter_mut()
        .find(|track| track.id == plan.track)
        .ok_or_else(preview_unavailable)?;
    if track.kind != cut_core::TrackKind::Caption {
        return Err(preview_unavailable());
    }
    let expected = plan
        .targets
        .iter()
        .map(|row| (row.cue_id.clone(), row))
        .collect::<BTreeMap<_, _>>();
    if expected.len() != plan.targets.len() {
        return Err(preview_unavailable());
    }
    for row in &plan.targets {
        let cue = track
            .clips
            .iter()
            .filter_map(|clip| match clip {
                cut_core::Clip::Caption(cue) => Some(cue),
                _ => None,
            })
            .find(|cue| cue.id == row.cue_id)
            .ok_or_else(preview_unavailable)?;
        if cue.range_ms != row.range_ms || cue.text != row.text {
            return Err(CutError::new(
                error_codes::CONFLICT,
                format!(
                    "caption cue '{}' changed after its bulk preview",
                    row.cue_id
                ),
                "the exact cue id, timeline range, or text no longer matches the reviewed preview",
            )
            .with_suggested_action("run captions.bulk_preview again before replacing captions"));
        }
    }
    for clip in &mut track.clips {
        let cut_core::Clip::Caption(cue) = clip else {
            continue;
        };
        let Some(row) = expected.get(&cue.id) else {
            continue;
        };
        cue.text.clone_from(&row.replacement_text);
        if refresh_timing {
            cue.range_ms = row.refreshed_range_ms.ok_or_else(|| CutError::new(error_codes::GUARDRAIL, "timing refresh is unavailable for this caption replacement", "the reviewed preview did not contain exact transcript word ranges for every changed cue"))?;
        }
    }
    Ok(())
}

fn matcher_for(args: &PreviewArgs) -> Result<Regex, CutError> {
    let escaped = regex::escape(&args.find);
    let pattern = match args.match_mode {
        MatchMode::Contains => escaped,
        MatchMode::WholeWord => format!(r"\b(?:{escaped})\b"),
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!args.case_sensitive)
        .build()
        .map_err(|error| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "caption search is invalid",
                error.to_string(),
            )
        })
}

fn overlaps_scope(cue: [u64; 2], scope: Option<[u64; 2]>) -> bool {
    scope.is_none_or(|range| cue[0] < range[1] && cue[1] > range[0])
}

fn match_ranges_utf16(matcher: &Regex, text: &str) -> Vec<[usize; 2]> {
    let mut byte_cursor = 0;
    let mut utf16_cursor = 0;
    matcher
        .find_iter(text)
        .map(|found| {
            // Ordered, nonoverlapping matches let each gap and match contribute
            // once, rather than recounting the whole prefix for every match.
            utf16_cursor += text[byte_cursor..found.start()].encode_utf16().count();
            let start = utf16_cursor;
            utf16_cursor += text[found.start()..found.end()].encode_utf16().count();
            byte_cursor = found.end();
            [start, utf16_cursor]
        })
        .collect()
}

fn normalized_words(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn preview_unavailable() -> CutError {
    CutError::new(error_codes::CONFLICT, "caption replacement preview is unavailable", "the preview was consumed, superseded, or belongs to a previous server run; no cues were changed").with_suggested_action("run captions.bulk_preview again before replacing captions")
}

#[cfg(test)]
mod utf16_tests {
    use super::*;

    fn matcher(find: &str, mode: MatchMode, case_sensitive: bool) -> Regex {
        matcher_for(&PreviewArgs {
            track: "cap1".into(),
            find: find.into(),
            replace_with: "$1".into(),
            match_mode: mode,
            case_sensitive,
            range_ms: None,
        })
        .unwrap()
    }

    #[test]
    fn offsets_preserve_unicode_literal_case_and_word_semantics() {
        let contains = MatchMode::Contains;
        assert_eq!(
            match_ranges_utf16(&matcher("😀", contains.clone(), true), "a😀😀z"),
            vec![[1, 3], [3, 5]]
        );
        assert_eq!(
            match_ranges_utf16(&matcher("k", contains.clone(), false), "😀KkK"),
            vec![[2, 3], [3, 4], [4, 5]]
        );
        assert_eq!(
            match_ranges_utf16(&matcher("k", contains.clone(), true), "😀KkK"),
            vec![[3, 4]]
        );
        assert_eq!(
            match_ranges_utf16(&matcher("a", MatchMode::WholeWord, true), "a aa éa a"),
            vec![[0, 1], [8, 9]]
        );
        assert_eq!(
            match_ranges_utf16(&matcher(".*", contains.clone(), true), "😀.*.*"),
            vec![[2, 4], [4, 6]]
        );
        assert_eq!(
            match_ranges_utf16(&matcher("aa", contains.clone(), true), "aaa"),
            vec![[0, 2]]
        );
        assert!(match_ranges_utf16(&matcher("z", contains, true), "😀").is_empty());
    }

    #[test]
    fn dense_megabyte_cue_preserves_all_offsets() {
        let text = "a".repeat(1024 * 1024);
        let ranges = match_ranges_utf16(&matcher("a", MatchMode::Contains, true), &text);
        assert_eq!(ranges.len(), text.len());
        assert!(ranges
            .iter()
            .enumerate()
            .all(|(index, range)| *range == [index, index + 1]));
    }
}
