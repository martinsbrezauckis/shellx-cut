//! ops.rs — operation records + the append-only op-log (timeline/op-log contract).
//!
//! Role: `ops.jsonl` is the SOURCE OF TRUTH; `project.json` is a cache.
//! Ops are immutable; reject/undo appends a NEW op referencing the old
//! (`edit.restore`); checkpoints are pointers into the log.
//! Dependencies: serde, serde_json, chrono. Primary callers: store.rs
//! (persistence/rebuild), edit.rs (op construction), server verb handlers.

use crate::error::CutError;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Who performed an op (timeline/op-log contract `actor`). `via` is the surface used:
/// "mcp" | "rest" | "cli" | "ui".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub name: String,
    pub via: String,
}

impl Actor {
    /// The server's own actor identity (system-initiated ops, e.g. job completions).
    pub fn system() -> Self {
        Self {
            kind: ActorKind::System,
            name: "cutd".into(),
            via: "internal".into(),
        }
    }
}

/// Actor kind discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Agent,
    Human,
    System,
}

/// Op lifecycle status. `Applied` is the normal case; `Rejected` marks ops
/// undone via review (the undoing itself is a new `edit.restore` op).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpStatus {
    Applied,
    Rejected,
}

/// A concrete effect of an op on the timeline (timeline/op-log contract `effects`), e.g.
/// `{"track":"v1","removed_ms":[63200,64900]}`. `track` is typed because
/// every effect names one; the rest is op-specific and kept flexible so
/// new verbs don't need a core change to record what they did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// Op-specific detail keys (removed_ms, added_clip, moved_to, ...).
    #[serde(flatten)]
    pub detail: serde_json::Map<String, serde_json::Value>,
}

/// The inverse of an op — what to dispatch to undo it (timeline/op-log contract `inverse`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InverseOp {
    pub verb: String,
    pub args: serde_json::Value,
}

/// One immutable line of `ops.jsonl` (timeline/op-log contract "Operation record").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpRecord {
    /// Monotonic id "op_NNNNNN" (zero-padded, ordering == log order).
    pub op_id: String,
    /// RFC3339 timestamp with ms.
    pub ts: String,
    pub actor: Actor,
    /// Fully-qualified verb name, e.g. "transcript.cut_words".
    pub verb: String,
    /// The verb args as received.
    pub args: serde_json::Value,
    /// Why the actor did it — surfaced verbatim in the review rail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// What actually changed.
    #[serde(default)]
    pub effects: Vec<OpEffect>,
    /// How to undo (None for non-undoable ops like project.create).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<InverseOp>,
    pub status: OpStatus,
}

impl OpRecord {
    /// Format a monotonic op id from a 0-based sequence number: 41 → "op_000042"
    /// (ids are 1-based in the log to match public contract examples).
    pub fn format_id(seq: u64) -> String {
        format!("op_{:06}", seq + 1)
    }

    /// RFC3339 "now" with millisecond precision (the log's ts format).
    pub fn now_ts() -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// Composite-action group tag, if any — rides as a `group_id`
    /// effect detail (NOT a struct field, to avoid churning every OpRecord
    /// literal; persisted in the log + survives reopen like any effect). Ops
    /// dispatched as ONE logical user action (linked A/V paste, linked delete)
    /// share a tag, so the undo cursor steps over the whole group in one
    /// Ctrl+Z. `None` = an ordinary standalone edit.
    pub fn group_id(&self) -> Option<&str> {
        self.effects
            .iter()
            .find_map(|e| e.detail.get("group_id").and_then(|v| v.as_str()))
    }

    /// True when this op mutated the TIMELINE (tracks/markers/caption_styles)
    /// and is therefore undoable/rebasable, as opposed to metadata/structural
    /// ops that carry no timeline change.
    ///
    /// Recompute-by-replay model: this replaces the old
    /// `inverse.is_some()` predicate. Mutating ops no longer carry a per-op
    /// full-timeline snapshot (that was the O(N²) disk growth); their undo
    /// state is recomputed by replaying the log prefix. So "is this a timeline
    /// op?" is now answered by the verb, not by the presence of a snapshot. The
    /// excluded set below is the metadata/import/checkpoint/comment subset that
    /// does not change tracks, markers, or caption styles.
    pub fn mutates_timeline(&self) -> bool {
        !matches!(
            self.verb.as_str(),
            "project.create"
                | "project.checkpoint"
                | "project.sequence_create"
                | "project.sequence_switch"
                | "project.sequence_rename"
                | "project.sequence_delete"
                | "project.rename" // metadata (the display name), not timeline state
                | "project.format" // metadata (project settings), not timeline state
                | "project.color" // metadata (project settings), not timeline state
                | "project.brand" // metadata (saved delivery constraints), not timeline state
                | "media.import"
                | "media.remove" // asset-map metadata (replay-safe), not timeline state
                | "media.relink" // asset-map metadata: repoints path/hash, replay-safe
                | "motion.link.relink" // source-path metadata; pixels stay unchanged
                | "media.bin_save" // smart-bin metadata (grade.save pattern)
                | "media.bin_delete" // smart-bin metadata
                // Review-comment metadata ops (comment.apply is NOT here —
                // it executes timeline edits and IS a timeline op).
                | "comment.add"
                | "comment.import"
                | "comment.resolve"
                | "comment.draft"
                // grade.save adds a named preset to the project's grade GALLERY
                // (Project::grade_presets) — project metadata, not a timeline edit, so
                // it stays off the undo cursor (grade.apply, which lowers to edit.grade,
                // IS the undoable timeline op).
                | "grade.save"
                // captions.save_style adds a named preset to the caption style
                // GALLERY (Project::caption_style_presets) — same class as
                // grade.save (apply_style, lowering to captions.set_style, IS
                // the undoable timeline op).
                | "captions.save_style"
        )
    }
}

/// Handle to a project's append-only op-log file (`<proj>.cutproj/ops.jsonl`).
/// Append-only by construction: the only mutation is `append`.
#[derive(Debug, Clone)]
pub struct OpLog {
    /// Absolute path of ops.jsonl.
    pub path: std::path::PathBuf,
}

impl OpLog {
    /// Open (creating if absent) the op-log at `path`.
    pub fn open(path: &Path) -> Result<Self, CutError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            std::fs::write(path, b"")?;
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// Append one op as a single JSONL line. Side effect: fsyncs the file so
    /// a crash never loses an acknowledged op.
    pub fn append(&self, op: &OpRecord) -> Result<(), CutError> {
        let mut line = serde_json::to_string(op)?;
        line.push('\n');
        let mut f = OpenOptions::new().append(true).open(&self.path)?;
        f.write_all(line.as_bytes())?;
        f.sync_data()?;
        Ok(())
    }

    /// Read all ops, in log order. O(file), appropriate for project-local logs.
    pub fn read_all(&self) -> Result<Vec<OpRecord>, CutError> {
        let f = std::fs::File::open(&self.path)?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str::<OpRecord>(&line)?);
        }
        Ok(out)
    }

    /// Ops strictly AFTER `since_op_id` (None ⇒ all). Powers `project.ops{since?}`.
    pub fn read_since(&self, since_op_id: Option<&str>) -> Result<Vec<OpRecord>, CutError> {
        let all = self.read_all()?;
        match since_op_id {
            None => Ok(all),
            Some(since) => {
                let idx = all.iter().position(|o| o.op_id == since);
                match idx {
                    Some(i) => Ok(all.into_iter().skip(i + 1).collect()),
                    None => Err(CutError::new(
                        crate::error::codes::NOT_FOUND,
                        format!("op '{since}' not found in log"),
                        "the `since` op id does not exist; call project.ops without `since` to resync",
                    )),
                }
            }
        }
    }

    /// Next op id based on current log length. Count non-empty JSONL records
    /// directly instead of deserializing every historical op on each append.
    pub fn next_id(&self) -> Result<String, CutError> {
        let f = std::fs::File::open(&self.path)?;
        let mut count = 0u64;
        for line in BufReader::new(f).lines() {
            if !line?.trim().is_empty() {
                count += 1;
            }
        }
        Ok(OpRecord::format_id(count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(verb: &str) -> OpRecord {
        OpRecord {
            op_id: "op_000001".into(),
            ts: "2026-07-01T00:00:00.000Z".into(),
            actor: crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "test".into(),
                via: "test".into(),
            },
            verb: verb.into(),
            args: serde_json::json!({}),
            rationale: None,
            effects: Vec::new(),
            inverse: None,
            status: OpStatus::Applied,
        }
    }

    #[test]
    fn project_metadata_ops_are_not_timeline_mutations() {
        assert!(!op("project.rename").mutates_timeline());
        assert!(!op("project.format").mutates_timeline());
        assert!(!op("project.color").mutates_timeline());
        assert!(!op("project.brand").mutates_timeline());
        assert!(!op("comment.import").mutates_timeline());
        assert!(op("edit.insert").mutates_timeline());
    }
}
