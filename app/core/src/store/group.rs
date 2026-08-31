//! Bounded compound-action inspection and tip-only group rejection.
//!
//! A `group_id` is an existing durable undo tag carried by adjacent timeline
//! operations. This module deliberately does not introduce a generic
//! transaction language or replay mode: it only resolves one such contiguous
//! action and, when that action is the live history tip, restores the prefix
//! immediately before it in one normal append-only `edit.restore` record.

use super::*;

/// One exact adjacent compound action discovered from the durable journal.
///
/// `op_ids` are ordered oldest to newest. The caller must carry every identity
/// field from a preview into the guarded reject request; a group id alone is
/// not enough because group tags are labels rather than global transaction ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicGroupPreview {
    pub group_id: String,
    pub first_op_id: String,
    pub last_op_id: String,
    pub op_ids: Vec<String>,
    pub sequence_id: String,
    pub reject_status: AtomicGroupRejectStatus,
}

/// Whether the exact group can currently be rejected as one history action.
/// A historical group remains inspectable, but only the live tip may be
/// restored: this preserves the existing no-silent-discard restore contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtomicGroupRejectStatus {
    Ready,
    NotCurrentTip { current_tip_op_id: Option<String> },
}

impl AtomicGroupPreview {
    pub fn is_reject_ready(&self) -> bool {
        matches!(self.reject_status, AtomicGroupRejectStatus::Ready)
    }
}

impl ProjectStore {
    /// Resolve the complete, contiguous compound action containing `op_id`.
    ///
    /// Group tags are only meaningful for two or more adjacent timeline edits
    /// in one active sequence. That is intentionally the same boundary that
    /// existing grouped undo uses. A group can be inspected after later edits,
    /// but it becomes rejectable only when it is the live undo-history tip.
    pub fn atomic_group_preview(&self, op_id: &str) -> Result<AtomicGroupPreview, CutError> {
        let journal = self.log.replay_view()?;
        self.atomic_group_preview_from_records(journal.records(), op_id)
    }

    /// Reject exactly one live compound action in one append-only restore op.
    ///
    /// This is deliberately tip-only. It recomputes the normal journal prefix
    /// before the group's first operation; it does not skip-replay a set of
    /// arbitrary operations or try to rebase a compound action over later work.
    pub fn restore_atomic_group_tip(
        &mut self,
        preview: &AtomicGroupPreview,
        actor: Actor,
        rationale: Option<String>,
    ) -> Result<OpRecord, CutError> {
        let journal = self.log.replay_view()?;
        let all = journal.records();
        let current = self.atomic_group_preview_from_records(all, &preview.first_op_id)?;
        if current != *preview {
            return Err(CutError::new(
                codes::CONFLICT,
                "compound action changed after its preview",
                "the group boundary, active sequence, or history cursor no longer matches the reviewed action",
            )
            .with_suggested_action("preview the compound action again before rejecting it"));
        }
        let current_tip = match &current.reject_status {
            AtomicGroupRejectStatus::Ready => current.last_op_id.clone(),
            AtomicGroupRejectStatus::NotCurrentTip { current_tip_op_id } => {
                return Err(CutError::new(
                    codes::GUARDRAIL,
                    "compound action is no longer the current undo step",
                    format!(
                        "the reviewed group ends at '{}', while the live history tip is '{}'",
                        current.last_op_id,
                        current_tip_op_id.as_deref().unwrap_or("the project baseline")
                    ),
                )
                .with_suggested_action(
                    "review the newer edits; use project.undo for the latest action or preview this group again after returning to it",
                ));
            }
        };
        let first_index = all
            .iter()
            .position(|op| op.op_id == current.first_op_id)
            .ok_or_else(|| {
                CutError::new(
                    codes::CONFLICT,
                    "compound action disappeared from the project journal",
                    "the reviewed first operation is no longer present in the durable log",
                )
            })?;
        // The exact pre-group prefix is the same deterministic replay source
        // used by a normal tip restore. It is one bounded source fact, not a
        // new generic multi-operation replay mechanism.
        let (pre_group, _) = snapshots::rebuild(&self.dir, &journal, first_index)?;
        let snapshot = timeline_snapshot(&pre_group);
        let mut next = self.project.clone();
        edit::apply_set_timeline(&mut next, &snapshot.args)?;
        let rec = OpRecord {
            op_id: self.log.next_id()?,
            ts: OpRecord::now_ts(),
            actor,
            // Keep the durable shape a normal replayable restore. The group
            // scope is recorded in effects, not as a new future replay mode.
            verb: "edit.restore".into(),
            args: json!({
                "op_id": current.first_op_id,
                "mode": "tip",
            }),
            rationale,
            effects: vec![edit::fx(
                None,
                json!({
                    "restored_op": current.first_op_id,
                    "mode": "tip",
                    "restored_group": {
                        "group_id": current.group_id,
                        "first_op_id": current.first_op_id,
                        "last_op_id": current.last_op_id,
                        "op_ids": current.op_ids,
                        "sequence_id": current.sequence_id,
                        "history_tip": current_tip,
                    },
                    "restored_timeline": snapshot.args,
                }),
            )],
            inverse: None,
            status: OpStatus::Applied,
        };
        // The record owns every detail derived from the replay view. Dropping
        // that immutable view before append avoids a full journal Arc COW.
        drop(journal);
        self.commit_staged(next, &rec)?;
        Ok(rec)
    }

    fn atomic_group_preview_from_records(
        &self,
        all: &[OpRecord],
        op_id: &str,
    ) -> Result<AtomicGroupPreview, CutError> {
        let selected = all.iter().position(|op| op.op_id == op_id).ok_or_else(|| {
            CutError::new(
                codes::NOT_FOUND,
                format!("no op '{op_id}' in the log"),
                "op ids come from project.ops",
            )
        })?;
        let group_id = all[selected]
            .group_id()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                CutError::new(
                    codes::INVALID_ARGS,
                    format!("op '{op_id}' is not part of a compound action"),
                    "compound preview accepts an operation with a durable group_id effect",
                )
                .with_suggested_action("use edit.restore for one standalone tip operation")
            })?
            .to_owned();

        let mut first = selected;
        while first > 0 && all[first - 1].group_id() == Some(group_id.as_str()) {
            first -= 1;
        }
        let mut last = selected;
        while last + 1 < all.len() && all[last + 1].group_id() == Some(group_id.as_str()) {
            last += 1;
        }
        if last == first {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("compound action '{group_id}' has only one operation"),
                "a group reject is reserved for an adjacent multi-operation action",
            )
            .with_suggested_action("use edit.restore for this standalone tip operation"));
        }

        let assignments = op_sequence_assignments(all)?;
        let sequence_id = assignments[first].clone();
        let op_ids = all[first..=last]
            .iter()
            .map(|op| op.op_id.clone())
            .collect::<Vec<_>>();
        for (op, sequence) in all[first..=last].iter().zip(&assignments[first..=last]) {
            if !op.mutates_timeline()? {
                return Err(CutError::new(
                    codes::GUARDRAIL,
                    format!(
                        "compound action '{group_id}' includes non-timeline op '{}'",
                        op.op_id
                    ),
                    "group rejection only restores one contiguous timeline action",
                )
                .with_suggested_action("review each non-timeline record individually"));
            }
            if sequence != &sequence_id {
                return Err(CutError::new(
                    codes::GUARDRAIL,
                    format!("compound action '{group_id}' crosses project sequences"),
                    "group rejection is sequence-scoped and cannot combine independent timelines",
                )
                .with_suggested_action(
                    "switch to the owning sequence and review its actions separately",
                ));
            }
        }
        if sequence_id != self.project.active_sequence {
            return Err(CutError::new(
                codes::GUARDRAIL,
                format!(
                    "compound action belongs to sequence '{sequence_id}', not the active sequence '{}'",
                    self.project.active_sequence
                ),
                "group rejection is sequence-scoped",
            )
            .with_suggested_action(format!("switch to sequence '{sequence_id}' before reviewing it")));
        }

        let current_tip_op_id = self.undo_history.get(self.undo_pos).cloned();
        let reject_status = if current_tip_op_id.as_deref() == Some(all[last].op_id.as_str()) {
            AtomicGroupRejectStatus::Ready
        } else {
            AtomicGroupRejectStatus::NotCurrentTip { current_tip_op_id }
        };
        Ok(AtomicGroupPreview {
            group_id,
            first_op_id: all[first].op_id.clone(),
            last_op_id: all[last].op_id.clone(),
            op_ids,
            sequence_id,
            reject_status,
        })
    }
}

#[cfg(test)]
#[path = "group_tests.rs"]
mod tests;
