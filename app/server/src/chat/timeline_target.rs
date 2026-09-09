//! Immutable timeline context for an Agent Chat turn.
//!
//! The UI may choose a range, clips, playhead, or persisted comment anchor, but
//! it must freeze that context before opening Chat. This module rejects a copied
//! target from another project, a stale revision, or clip spans that no longer
//! match the materialized timeline before any provider process is launched.

use cut_core::{Clip, Project};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const SCHEMA: &str = "shellx-cut/chat-timeline-target/1";
const MAX_CLIPS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectIdentity {
    pub schema: String,
    pub origin_path_sha256: String,
    pub project_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetClip {
    pub track_id: String,
    pub clip_id: String,
    pub timeline_range_ms: [u64; 2],
    pub target_range_ms: [u64; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineTarget {
    pub schema: String,
    pub kind: String,
    pub project_identity: ProjectIdentity,
    pub project_revision: String,
    #[serde(default)]
    pub comment_id: Option<String>,
    pub range_ms: [u64; 2],
    #[serde(default)]
    pub position_ms: Option<u64>,
    #[serde(default)]
    pub clips: Vec<TargetClip>,
    /// Display-only client text. It is never inserted into an agent prompt.
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalClip {
    track_id: String,
    clip_id: String,
    range_ms: [u64; 2],
}

fn canonical_clips(project: &Project) -> Vec<CanonicalClip> {
    let mut result = Vec::new();
    for track in &project.tracks {
        let mut cursor = 0u64;
        let mut previous_media_duration = None;
        for clip in &track.clips {
            let duration = clip.timeline_duration_ms();
            let xfade = match (previous_media_duration, clip) {
                (Some(previous), Clip::Media(media)) if media.xfade_in_ms > 0 => {
                    media.xfade_in_ms.min(previous).min(duration)
                }
                _ => 0,
            };
            let start = cursor.saturating_sub(xfade);
            let end = start.saturating_add(duration);
            if let Some(clip_id) = clip.id() {
                result.push(CanonicalClip {
                    track_id: track.id.clone(),
                    clip_id: clip_id.to_owned(),
                    range_ms: [start, end],
                });
            }
            cursor = end;
            previous_media_duration = match clip {
                Clip::Media(_) => Some(duration),
                _ => None,
            };
        }
    }
    result
}

fn equal_identity(expected: &ProjectIdentity, actual: &Value) -> bool {
    actual.get("schema").and_then(Value::as_str) == Some(expected.schema.as_str())
        && actual.get("origin_path_sha256").and_then(Value::as_str)
            == Some(expected.origin_path_sha256.as_str())
        && actual.get("project_name").and_then(Value::as_str)
            == Some(expected.project_name.as_str())
}

/// Validate and return the received snapshot unchanged. The server only uses
/// canonical fields in its prompt; `label` survives solely for the UI's stored
/// conversation trace.
pub fn validate(
    target: TimelineTarget,
    project: &Project,
    current_identity: &Value,
    current_revision: Option<&str>,
) -> Result<TimelineTarget, String> {
    if target.schema != SCHEMA {
        return Err(format!("timeline target schema must be {SCHEMA}"));
    }
    if !equal_identity(&target.project_identity, current_identity) {
        return Err("timeline target belongs to a different open project".into());
    }
    if target.project_revision.trim().is_empty()
        || current_revision != Some(target.project_revision.as_str())
    {
        return Err("timeline target was prepared against a different project revision".into());
    }
    let position_kind = target.kind == "position";
    let comment_kind = target.kind == "comment";
    if !matches!(
        target.kind.as_str(),
        "range" | "selection" | "comment" | "position"
    ) {
        return Err("timeline target kind must be range, selection, comment, or position".into());
    }
    if target.range_ms[1] < target.range_ms[0]
        || (matches!(target.kind.as_str(), "range" | "selection")
            && target.range_ms[1] == target.range_ms[0])
    {
        return Err("timeline target range is invalid for its kind".into());
    }
    if target.range_ms[1] > project.duration_ms() {
        return Err("timeline target range is outside the current project timeline".into());
    }
    if position_kind || comment_kind {
        if target.position_ms != Some(target.range_ms[0]) {
            return Err("comment or position target must name its exact position_ms".into());
        }
    } else if target.position_ms.is_some() {
        return Err("only comment or position targets may carry position_ms".into());
    }
    if position_kind && target.comment_id.is_some() {
        return Err("a playhead target cannot name a comment".into());
    }
    if let Some(id) = target.comment_id.as_deref() {
        if id.trim().is_empty() {
            return Err("timeline target comment_id cannot be empty".into());
        }
        if !project.comments.iter().any(|comment| comment.id == id) {
            return Err("the requested comment no longer exists in the open project".into());
        }
    } else if comment_kind {
        return Err("a comment target requires comment_id".into());
    }
    if target.clips.len() > MAX_CLIPS {
        return Err(format!(
            "timeline target may name at most {MAX_CLIPS} clips"
        ));
    }

    let canonical = canonical_clips(project);
    let mut seen = BTreeSet::new();
    for requested in &target.clips {
        if requested.track_id.trim().is_empty() || requested.clip_id.trim().is_empty() {
            return Err("timeline target clip identity cannot be empty".into());
        }
        if !seen.insert((&requested.track_id, &requested.clip_id)) {
            return Err("timeline target names a clip more than once".into());
        }
        if requested.timeline_range_ms[1] < requested.timeline_range_ms[0]
            || requested.target_range_ms[1] < requested.target_range_ms[0]
        {
            return Err("timeline target clip range is reversed".into());
        }
        let live = canonical
            .iter()
            .find(|candidate| {
                candidate.track_id == requested.track_id && candidate.clip_id == requested.clip_id
            })
            .ok_or_else(|| {
                format!(
                    "timeline target clip '{}' no longer exists",
                    requested.clip_id
                )
            })?;
        if live.range_ms != requested.timeline_range_ms {
            return Err(format!(
                "timeline target clip '{}' moved or changed",
                requested.clip_id
            ));
        }
        if requested.target_range_ms[0] < live.range_ms[0]
            || requested.target_range_ms[1] > live.range_ms[1]
            || requested.target_range_ms[0] < target.range_ms[0]
            || requested.target_range_ms[1] > target.range_ms[1]
        {
            return Err(format!(
                "timeline target range for '{}' is outside the requested context",
                requested.clip_id
            ));
        }
    }
    Ok(target)
}

/// Prompt data excludes the client display label. The target remains opaque data
/// rather than a second instruction channel to the provider.
pub fn prompt_value(target: &TimelineTarget) -> Value {
    serde_json::json!({
        "schema": target.schema,
        "kind": target.kind,
        "project_identity": target.project_identity,
        "project_revision": target.project_revision,
        "comment_id": target.comment_id,
        "range_ms": target.range_ms,
        "position_ms": target.position_ms,
        "clips": target.clips,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::{edit::make_media_clip, Comment, ProjectSettings};

    fn project() -> Project {
        let mut project = Project::new("target", ProjectSettings::default());
        project
            .track_mut("v1")
            .expect("default video track")
            .clips
            .push(Clip::Media(make_media_clip("c1", "a1", 0, 4_000)));
        project
    }

    fn target() -> TimelineTarget {
        TimelineTarget {
            schema: SCHEMA.into(),
            kind: "selection".into(),
            project_identity: ProjectIdentity {
                schema: "shellx-cut/project-identity/1".into(),
                origin_path_sha256:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                project_name: "target".into(),
            },
            project_revision: "op_1".into(),
            comment_id: None,
            range_ms: [0, 4_000],
            position_ms: None,
            clips: vec![TargetClip {
                track_id: "v1".into(),
                clip_id: "c1".into(),
                timeline_range_ms: [0, 4_000],
                target_range_ms: [0, 4_000],
            }],
            label: "Selected clip".into(),
        }
    }

    fn identity() -> Value {
        serde_json::json!({"schema":"shellx-cut/project-identity/1","origin_path_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","project_name":"target"})
    }

    fn project_with_comment() -> Project {
        let mut project = project();
        project.comments.push(Comment {
            id: "cm1".into(),
            at_ms: 1_000,
            end_ms: Some(2_000),
            anchor: None,
            text: "Tighten this section".into(),
            author: "reviewer".into(),
            status: "open".into(),
            ts: "2026-09-08T00:00:00Z".into(),
            review_source: None,
            draft: None,
        });
        project
    }

    #[test]
    fn accepts_exact_live_selection_and_omits_display_label_from_prompt_data() {
        let accepted = validate(target(), &project(), &identity(), Some("op_1")).unwrap();
        assert_eq!(prompt_value(&accepted)["clips"][0]["clip_id"], "c1");
        assert!(prompt_value(&accepted).get("label").is_none());
    }

    #[test]
    fn refuses_different_project_stale_revision_and_moved_span_before_provider_launch() {
        let mut other = target();
        other.project_identity.project_name = "other".into();
        assert!(validate(other, &project(), &identity(), Some("op_1"))
            .unwrap_err()
            .contains("different open project"));
        assert!(validate(target(), &project(), &identity(), Some("op_2"))
            .unwrap_err()
            .contains("different project revision"));
        let mut moved = target();
        moved.clips[0].timeline_range_ms = [1, 4_000];
        assert!(validate(moved, &project(), &identity(), Some("op_1"))
            .unwrap_err()
            .contains("moved or changed"));
    }

    #[test]
    fn refuses_malformed_points_and_ranges_outside_the_current_timeline() {
        let mut point = target();
        point.kind = "position".into();
        point.range_ms = [1_000, 1_000];
        point.position_ms = Some(999);
        point.clips.clear();
        assert!(validate(point, &project(), &identity(), Some("op_1"))
            .unwrap_err()
            .contains("exact position_ms"));

        let mut outside = target();
        outside.range_ms = [0, 4_001];
        assert!(validate(outside, &project(), &identity(), Some("op_1"))
            .unwrap_err()
            .contains("outside the current project timeline"));
    }

    #[test]
    fn accepts_a_ranged_comment_or_selected_range_linked_to_a_live_comment() {
        let mut comment = target();
        comment.kind = "comment".into();
        comment.comment_id = Some("cm1".into());
        comment.range_ms = [1_000, 2_000];
        comment.position_ms = Some(1_000);
        comment.clips[0].target_range_ms = [1_000, 2_000];
        assert!(validate(comment, &project_with_comment(), &identity(), Some("op_1")).is_ok());

        let mut selected_range = target();
        selected_range.kind = "range".into();
        selected_range.comment_id = Some("cm1".into());
        assert!(validate(
            selected_range,
            &project_with_comment(),
            &identity(),
            Some("op_1")
        )
        .is_ok());
    }
}
