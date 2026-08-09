//! Durable linear undo/redo history reconstruction and live bookkeeping.

use super::*;

#[derive(Default)]
struct HistoryState {
    ids: Vec<String>,
    pos: usize,
    last_group: Option<String>,
}

impl ProjectStore {
    /// Rebuild a sequence's forward-edit history and logical navigation cursor
    /// from the immutable journal.
    pub(super) fn history_state_from_log(
        log: &OpLog,
        active_sequence: &str,
    ) -> Result<(Vec<String>, usize), CutError> {
        let journal = log.replay_view()?;
        Self::history_state_from_records(journal.records(), active_sequence, None)
    }

    /// Derive post-append navigation before the append becomes durable. The
    /// record is already schema-validated by the caller, so this eliminates a
    /// fallible history-rebuild path after `sync_data`.
    pub(super) fn history_state_after_append(
        log: &OpLog,
        active_sequence: &str,
        appended: &OpRecord,
    ) -> Result<(Vec<String>, usize), CutError> {
        let journal = log.replay_view()?;
        Self::history_state_from_records(journal.records(), active_sequence, Some(appended))
    }

    fn history_state_from_records(
        ops: &[OpRecord],
        active_sequence: &str,
        appended: Option<&OpRecord>,
    ) -> Result<(Vec<String>, usize), CutError> {
        let mut histories: BTreeMap<String, HistoryState> = BTreeMap::new();
        let mut current = DEFAULT_SEQUENCE_ID.to_string();
        if let Some(first) = ops.first() {
            if first.verb != "project.create" {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    "op log does not start with project.create",
                    format!("first op is '{}'", first.verb),
                ));
            }
            histories.insert(
                current.clone(),
                HistoryState {
                    ids: vec![first.op_id.clone()],
                    ..HistoryState::default()
                },
            );
        }
        for op in ops.iter().chain(appended) {
            match op.verb.as_str() {
                "project.sequence_create" => {
                    let id = op
                        .effects
                        .iter()
                        .find_map(|effect| effect.detail.get("sequence"))
                        .and_then(|sequence| sequence.get("id"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            CutError::new(
                                codes::INVALID_ARGS,
                                "sequence-create op is missing its recorded sequence id",
                                format!("op '{}' cannot seed sequence history", op.op_id),
                            )
                        })?;
                    current = id.to_string();
                    histories.insert(
                        current.clone(),
                        HistoryState {
                            ids: vec![op.op_id.clone()],
                            ..HistoryState::default()
                        },
                    );
                    continue;
                }
                "project.sequence_switch" => {
                    let id = op.args.get("id").and_then(Value::as_str).ok_or_else(|| {
                        CutError::new(
                            codes::INVALID_ARGS,
                            "sequence-switch op is missing id",
                            format!("op '{}' cannot reconstruct sequence history", op.op_id),
                        )
                    })?;
                    current = id.to_string();
                    histories.entry(current.clone()).or_default().last_group = None;
                    continue;
                }
                "project.undo" | "project.redo" => {
                    Self::apply_navigation_record(
                        histories.entry(current.clone()).or_default(),
                        op,
                    )?;
                    continue;
                }
                "project.revert" => {
                    let history = histories.entry(current.clone()).or_default();
                    if let Some(cursor) = Self::recorded_cursor(op) {
                        Self::validate_cursor(history, op, cursor, "revert")?;
                        history.pos = cursor;
                    }
                    history.last_group = None;
                    continue;
                }
                _ => {}
            }
            if !Self::is_history_edit(op)? {
                continue;
            }
            let history = histories.entry(current.clone()).or_default();
            history.ids.truncate(history.pos.saturating_add(1));
            let group = op.group_id();
            let merge = group.is_some()
                && group == history.last_group.as_deref()
                && !history.ids.is_empty();
            if merge {
                if let Some(last) = history.ids.last_mut() {
                    *last = op.op_id.clone();
                }
            } else {
                history.ids.push(op.op_id.clone());
                history.pos = history.ids.len() - 1;
            }
            history.last_group = group.map(str::to_string);
        }
        let history = histories.remove(active_sequence).unwrap_or_default();
        Ok((history.ids, history.pos))
    }

    fn recorded_cursor(op: &OpRecord) -> Option<usize> {
        op.effects
            .iter()
            .find_map(|effect| effect.detail.get("cursor"))
            .and_then(Value::as_u64)
            .map(|cursor| cursor as usize)
    }

    fn validate_cursor(
        history: &HistoryState,
        op: &OpRecord,
        cursor: usize,
        kind: &str,
    ) -> Result<(), CutError> {
        if cursor < history.ids.len() {
            return Ok(());
        }
        Err(CutError::new(
            codes::INVALID_ARGS,
            format!("{kind} cursor is outside the reconstructed history"),
            format!(
                "op '{}' records cursor {cursor} for {} history entries",
                op.op_id,
                history.ids.len()
            ),
        ))
    }

    fn apply_navigation_record(history: &mut HistoryState, op: &OpRecord) -> Result<(), CutError> {
        let cursor = Self::recorded_cursor(op)
            .or_else(|| {
                op.args
                    .get("to_op")
                    .and_then(Value::as_str)
                    .and_then(|target| history.ids.iter().position(|id| id == target))
            })
            .ok_or_else(|| {
                CutError::new(
                    codes::INVALID_ARGS,
                    "history navigation op has no reconstructable cursor",
                    format!(
                        "op '{}' does not identify its target history position",
                        op.op_id
                    ),
                )
            })?;
        Self::validate_cursor(history, op, cursor, "history navigation")?;
        history.pos = cursor;
        history.last_group = None;
        Ok(())
    }

    /// True when `op` is a forward timeline edit that advances history. The
    /// generated mutation class folds metadata and durable navigation records
    /// out of the cursor without another local verb list.
    pub(super) fn is_history_edit(op: &OpRecord) -> Result<bool, CutError> {
        Ok(op.mutation_class()? == crate::MutationClass::Timeline)
    }

    pub(super) fn record_history_commit(&mut self, op: &OpRecord, is_history_edit: bool) {
        if !is_history_edit {
            return;
        }
        self.undo_history.truncate(self.undo_pos + 1);
        let group = op.group_id();
        let merge =
            group.is_some() && group == self.tip_group.as_deref() && !self.undo_history.is_empty();
        if merge {
            if let Some(last) = self.undo_history.last_mut() {
                *last = op.op_id.clone();
            }
        } else {
            self.undo_history.push(op.op_id.clone());
            self.undo_pos = self.undo_history.len() - 1;
        }
        self.tip_group = group.map(str::to_string);
    }
}
