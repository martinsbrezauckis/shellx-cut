//! Active running-turn state, live MCP op attribution, and completion guard.

use super::storage;
use super::trim_turns;
use super::types::{ActiveChatTurn, ChatActionRef, ChatTurnRecord};
use crate::events::Event;
use crate::state::AppState;
use cut_core::{Actor, ActorKind, CutError, VerbResult};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) fn active_turn_snapshot(
    state: &AppState,
    project_dir: &Path,
    conversation_id: &str,
) -> Option<ActiveChatTurn> {
    let guard = state.chat_active.lock().ok()?;
    guard
        .turns
        .get(&(project_dir.to_path_buf(), conversation_id.to_string()))
        .cloned()
}

pub(super) fn register_active_turn(state: &AppState, turn: ActiveChatTurn) {
    if let Ok(mut guard) = state.chat_active.lock() {
        guard.turns.insert(
            (turn.project_dir.clone(), turn.conversation_id.clone()),
            turn,
        );
    }
}

/// Record MCP ops applied by an active `agent.chat` turn immediately so UI clients see them live.
pub(crate) fn record_live_action_if_active(
    state: &AppState,
    actor: &Actor,
    verb: &str,
    result: &VerbResult,
) {
    if actor.kind != ActorKind::Agent || actor.via != "agent.chat" || !result.ok {
        return;
    }
    let Some(op_ids) = &result.op_ids else {
        return;
    };
    if op_ids.is_empty() {
        return;
    }
    let mut updated = Vec::new();
    if let Ok(mut guard) = state.chat_active.lock() {
        for active in guard.turns.values_mut() {
            if active.turn_id == actor.name {
                let mut changed = false;
                for op_id in op_ids {
                    let entry = ChatActionRef {
                        op_id: op_id.clone(),
                        verb: verb.to_string(),
                    };
                    if !active.actions.contains(&entry) {
                        active.actions.push(entry);
                        changed = true;
                    }
                }
                if changed {
                    updated.push((active.conversation_id.clone(), active.turn_id.clone()));
                }
            }
        }
    }
    for (conversation_id, turn_id) in updated {
        state.events.publish(Event::ChatUpdated {
            conversation_id,
            state: Some("action".into()),
            turn_id: Some(turn_id),
        });
    }
}

pub(crate) struct ActiveTurnGuard {
    pub(super) state: AppState,
    pub(super) project_dir: PathBuf,
    pub(super) conversation_id: String,
    pub(crate) origin: String,
    pub(crate) turn_id: String,
    pub(super) finished: bool,
}

impl ActiveTurnGuard {
    /// Persist the final agent turn and clear active tracking.
    pub(crate) async fn finish(
        mut self,
        agent: Option<&str>,
        model: Option<&str>,
        result: &mut Value,
    ) -> Result<(), CutError> {
        self.finished = true;
        let _io = self.state.chat_io_lock.lock().await;
        if let Ok(mut guard) = self.state.chat_active.lock() {
            guard
                .turns
                .remove(&(self.project_dir.clone(), self.conversation_id.clone()));
        }

        if let Value::Object(map) = result {
            map.insert(
                "conversation_id".into(),
                Value::String(self.conversation_id.clone()),
            );
            map.insert("origin".into(), Value::String(self.origin.clone()));
        }

        let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let text = result
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let error = result
            .get("error")
            .and_then(Value::as_str)
            .map(String::from);
        let actions: Vec<ChatActionRef> = result
            .get("actions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let attachments: Vec<String> = result
            .get("attachments")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let evidence_ids: Vec<String> = result
            .get("evidence_ids")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let evidence_index_id = result
            .get("evidence_index_id")
            .and_then(Value::as_str)
            .map(String::from);
        let target = result.get("target").filter(|v| !v.is_null()).cloned();
        let plan = result.get("plan").filter(|v| !v.is_null()).cloned();
        let review = result.get("review").filter(|v| !v.is_null()).cloned();
        let cost_usd = result.get("cost_usd").and_then(Value::as_f64);

        if let storage::LoadedRecord::Readable(mut record) =
            storage::read_conversation_file(&self.project_dir, &self.conversation_id)?
        {
            let now = storage::now_ts();
            record.updated_at = now.clone();
            record.turns.push(ChatTurnRecord {
                id: format!("agent_{}", self.turn_id),
                role: "agent".into(),
                text,
                created_at: now,
                origin: None,
                status: Some(if ok { "ok".into() } else { "error".into() }),
                agent: agent.map(String::from).or_else(|| {
                    result
                        .get("agent")
                        .and_then(Value::as_str)
                        .map(String::from)
                }),
                model: model.map(String::from),
                error,
                actions,
                attachments,
                evidence_ids,
                evidence_index_id,
                target,
                plan,
                review,
                cost_usd,
                reverted: false,
            });
            trim_turns(&mut record.turns);
            storage::write_conversation_file(&self.project_dir, &mut record)?;
        }

        self.state.events.publish(Event::ChatUpdated {
            conversation_id: self.conversation_id.clone(),
            state: Some(if ok {
                "completed".into()
            } else {
                "error".into()
            }),
            turn_id: Some(self.turn_id.clone()),
        });
        Ok(())
    }
}

impl Drop for ActiveTurnGuard {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(mut guard) = self.state.chat_active.lock() {
                guard
                    .turns
                    .remove(&(self.project_dir.clone(), self.conversation_id.clone()));
            }
            self.state.events.publish(Event::ChatUpdated {
                conversation_id: self.conversation_id.clone(),
                state: Some("interrupted".into()),
                turn_id: Some(self.turn_id.clone()),
            });
        }
    }
}
