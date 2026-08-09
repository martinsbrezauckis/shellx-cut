//! ops.rs — operation records + the append-only op-log (timeline/op-log contract).
//!
//! Role: `ops.jsonl` is the SOURCE OF TRUTH; `project.json` is a cache.
//! Ops are immutable; reject/undo appends a NEW op referencing the old
//! (`edit.restore`); checkpoints are pointers into the log.
//! Dependencies: serde, serde_json, chrono. Primary callers: store.rs
//! (persistence/rebuild), edit.rs (op construction), server verb handlers.

use crate::error::CutError;
use crate::mutation_request::{MutationRequest, RequestIndex};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod journal_index;

pub use journal_index::AppendOutcome;
use journal_index::JournalIndex;
#[cfg(test)]
pub(crate) use journal_index::JournalIndexMetrics;
pub use journal_index::JournalPage;
pub(crate) use journal_index::JournalView;

/// Who performed an op (timeline/op-log contract `actor`). `via` is the surface used:
/// "mcp" | "rest" | "cli" | "ui".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub name: String,
    pub via: String,
    /// Caller-supplied retry identity for an externally retryable mutation.
    /// Absent on legacy/internal calls. Persisting it with the op lets cutd
    /// detect a lost-response retry after restart without applying twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<MutationRequest>,
}

impl Actor {
    /// The server's own actor identity (system-initiated ops, e.g. job completions).
    pub fn system() -> Self {
        Self {
            kind: ActorKind::System,
            name: "cutd".into(),
            via: "internal".into(),
            request: None,
        }
    }

    pub fn with_request(mut self, request: MutationRequest) -> Self {
        self.request = Some(request);
        self
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

/// A durable replay step (`{verb,args}`).
///
/// Snapshot-era records used this shape in `OpRecord::inverse`; current
/// higher-layer verbs also use it for their recorded `lowered` forward steps.
/// New operation records do not write `inverse` payloads.
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
    /// Legacy snapshot-era undo payload. New records omit this field and undo
    /// by recomputing a journal prefix; it remains so historic journals and
    /// their old `edit.restore` records still replay exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<InverseOp>,
    pub status: OpStatus,
}

/// Evidence left when open recovered a malformed final journal record.
/// The discarded bytes remain in `quarantine_file`; `note_file` records the
/// exact byte range and parser cause next to the project journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecovery {
    pub discarded_start: u64,
    pub discarded_end: u64,
    pub quarantine_file: String,
    pub note_file: String,
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

    /// Schema-derived mutation class for this durable record. A journal that
    /// names an unknown verb fails closed: core must never turn a future or
    /// malformed record into an implicit timeline mutation.
    pub fn mutation_class(&self) -> Result<crate::MutationClass, CutError> {
        crate::contract_for_verb(&self.verb)
            .map(|contract| contract.mutation_class)
            .ok_or_else(|| {
                CutError::new(
                    crate::error::codes::INVALID_ARGS,
                    format!("op '{}' uses unknown verb '{}'", self.op_id, self.verb),
                    "journal replay requires generated schema metadata for every verb",
                )
                .with_suggested_action(
                    "restore a build that recognizes this verb or migrate the project with a compatible Cut version",
                )
            })
    }

    /// True when this op mutated the TIMELINE (tracks/markers/caption styles)
    /// and is therefore undoable/rebasable. The generated schema contract,
    /// rather than a local negative list, owns this classification.
    pub fn mutates_timeline(&self) -> Result<bool, CutError> {
        Ok(self.mutation_class()?.mutates_timeline())
    }
}

/// Handle to a project's append-only op-log file (`<proj>.cutproj/ops.jsonl`).
/// Append-only by construction: the only mutation is `append`.
#[derive(Debug, Clone)]
pub struct OpLog {
    /// Absolute path of ops.jsonl.
    pub path: std::path::PathBuf,
    index: Arc<Mutex<JournalIndex>>,
    recovery: Option<JournalRecovery>,
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
        let scan = crate::journal::open_and_recover(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            index: Arc::new(Mutex::new(JournalIndex::new(
                scan.next_seq,
                path,
                scan.requests,
                scan.records,
            )?)),
            recovery: scan.recovery,
        })
    }

    /// Recovery evidence from this open, if a malformed final record was
    /// quarantined. Middle-record corruption still fails open entirely.
    pub fn recovery(&self) -> Option<&JournalRecovery> {
        self.recovery.as_ref()
    }

    /// Append and fsync one JSONL record so a crash never loses an acknowledgement.
    pub fn append(&self, op: &OpRecord) -> Result<AppendOutcome, CutError> {
        let encoded = serde_json::to_vec(op)?;
        let mut line = encoded.clone();
        line.push(b'\n');
        let mut index = self.index.lock().map_err(|_| {
            CutError::new(
                crate::error::codes::IO,
                "operation journal index is unavailable",
                "the in-memory journal index lock was poisoned",
            )
        })?;
        index.ensure_unmodified(&self.path)?;
        let expected = OpRecord::format_id(index.next_seq);
        if op.op_id != expected {
            return Err(CutError::new(
                crate::error::codes::CONFLICT,
                format!("operation id '{}' is not the next journal id", op.op_id),
                format!("the validated journal index requires '{expected}'"),
            ));
        }
        let revision = revision_for_next_sequence(index.next_seq);
        index.requests.validate_append(op, revision.as_deref())?;
        let mut f = OpenOptions::new().append(true).open(&self.path)?;
        f.write_all(&line)?;
        f.sync_data()?;
        // Post-sync index updates are infallible; identity degradation is status, never Err.
        index.next_seq += 1;
        index.record_durable_append(op.clone(), &encoded);
        index.requests.record(op);
        let identity_degraded = index.refresh_stamp_after_durable_append(&self.path);
        Ok(AppendOutcome { identity_degraded })
    }

    /// Clone all validated records in log order without rereading `ops.jsonl`.
    pub fn read_all(&self) -> Result<Vec<OpRecord>, CutError> {
        let index = self.index.lock().map_err(|_| journal_index_unavailable())?;
        index.ensure_unmodified(&self.path)?;
        Ok(index.clone_records())
    }

    /// Read the current opaque revision and validated record count from the
    /// same identity check. Health consumers use this instead of composing
    /// separate reads that could observe a filesystem change between them.
    pub fn current_revision_and_count(&self) -> Result<(Option<String>, usize), CutError> {
        let index = self.index.lock().map_err(|_| journal_index_unavailable())?;
        index.ensure_unmodified(&self.path)?;
        Ok((
            revision_for_next_sequence(index.next_seq),
            index.view().records().len(),
        ))
    }

    /// Borrow validated records and prefix identities without rereading or
    /// hashing `ops.jsonl`; each borrow checks its on-disk identity first.
    pub(crate) fn replay_view(&self) -> Result<JournalView, CutError> {
        let index = self.index.lock().map_err(|_| journal_index_unavailable())?;
        index.ensure_unmodified(&self.path)?;
        Ok(index.view())
    }

    #[cfg(test)]
    pub(crate) fn reset_replay_metrics(&self) {
        if let Ok(mut index) = self.index.lock() {
            index.reset_metrics();
        }
    }

    #[cfg(test)]
    pub(crate) fn replay_metrics(&self) -> JournalIndexMetrics {
        self.index
            .lock()
            .map(|index| index.metrics())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn inject_next_stamp_refresh_failure(&self) {
        if let Ok(mut index) = self.index.lock() {
            index.inject_next_stamp_refresh_failure();
        }
    }

    /// Return one bounded page after `cursor` from the validated in-memory
    /// index. No journal bytes are reread and only returned records are cloned.
    pub fn page_after(
        &self,
        cursor: Option<&str>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<JournalPage, CutError> {
        let mut index = self.index.lock().map_err(|_| journal_index_unavailable())?;
        index.ensure_unmodified(&self.path)?;
        index.page_after(cursor, limit, max_bytes)
    }

    /// Next op id from the validated open-time index. Appends update this in
    /// constant time; external journal changes fail closed in [`Self::append`].
    pub fn next_id(&self) -> Result<String, CutError> {
        let index = self.index.lock().map_err(|_| {
            CutError::new(
                crate::error::codes::IO,
                "operation journal index is unavailable",
                "the in-memory journal index lock was poisoned",
            )
        })?;
        index.ensure_unmodified(&self.path)?;
        Ok(OpRecord::format_id(index.next_seq))
    }

    /// Current durable project revision: the latest committed operation ID.
    pub fn current_revision(&self) -> Result<Option<String>, CutError> {
        let index = self.index.lock().map_err(|_| journal_index_unavailable())?;
        index.ensure_unmodified(&self.path)?;
        Ok(revision_for_next_sequence(index.next_seq))
    }

    /// Existing op IDs for this exact caller request. Reusing a request ID
    /// with a different payload is a deterministic conflict.
    pub fn request_ops(&self, actor: &Actor) -> Result<Option<Vec<String>>, CutError> {
        let index = self.index.lock().map_err(|_| journal_index_unavailable())?;
        index.ensure_unmodified(&self.path)?;
        index.requests.op_ids(actor)
    }
}

fn revision_for_next_sequence(next_seq: u64) -> Option<String> {
    next_seq.checked_sub(1).map(OpRecord::format_id)
}

fn journal_index_unavailable() -> CutError {
    CutError::new(
        crate::error::codes::IO,
        "operation journal index is unavailable",
        "the in-memory journal index lock was poisoned",
    )
}

#[cfg(test)]
mod tests;
