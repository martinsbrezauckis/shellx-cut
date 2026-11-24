//! Project conversation listing, detail reads, legacy browser migration, and patching.

use super::{
    active_turn_snapshot, make_id, overlay_turns, storage, trim_turns, ChatTurnRecord,
    ConversationListResult, ConversationRecord, ConversationSummary, ConversationView,
    CHAT_CONVERSATION_SCHEMA,
};
use crate::events::Event;
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

pub(crate) async fn list_conversations(
    state: &AppState,
    project_dir: &Path,
) -> Result<ConversationListResult, CutError> {
    list_conversations_bounded(state, project_dir, storage::MAX_CHAT_LIST_REPLY_BYTES).await
}

pub(crate) async fn list_conversations_bounded(
    state: &AppState,
    project_dir: &Path,
    max_reply_bytes: usize,
) -> Result<ConversationListResult, CutError> {
    let _io = state.chat_io_lock.lock().await;
    let (mut ids, mut extra_unscanned) =
        storage::scan_chat_ids_bounded(project_dir, storage::MAX_CONVERSATION_FILES)?;
    if let Ok(guard) = state.chat_active.lock() {
        for (dir, cid) in guard.turns.keys() {
            if dir == project_dir && !ids.iter().any(|existing| existing == cid) {
                ids.push(cid.clone());
                extra_unscanned = extra_unscanned.saturating_sub(1);
            }
        }
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let live = active_turn_snapshot(state, project_dir, &id);
        match storage::read_conversation_file(project_dir, &id)? {
            storage::LoadedRecord::Readable(rec) => {
                let (turns, running) = overlay_turns(&rec, live.as_ref());
                out.push(storage::summary_from_record(&rec, &turns, running));
            }
            storage::LoadedRecord::Unreadable { id, title, reason } => {
                out.push(ConversationSummary {
                    id,
                    title,
                    origin: "unknown".into(),
                    readable: false,
                    unreadable_reason: Some(reason),
                    running: false,
                    turn_count: 0,
                    created_at: None,
                    updated_at: None,
                    last_preview: None,
                })
            }
            storage::LoadedRecord::Missing => {}
        }
    }
    out.sort_by(|a, b| {
        b.running
            .cmp(&a.running)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(cap_list_result_bytes(out, extra_unscanned, max_reply_bytes))
}

fn cap_list_result_bytes(
    out: Vec<ConversationSummary>,
    extra_unscanned: usize,
    max_reply_bytes: usize,
) -> ConversationListResult {
    // Base wrapper `{"conversations":[]}` is 20 bytes; truncation suffix
    // `,"truncated":true,"not_shown":<N>,"note":"<N> more conversations not shown"`
    // fits well inside 128 bytes.
    const BASE_OVERHEAD: usize = 20;
    const TRUNCATED_OVERHEAD: usize = 128;

    let item_sizes: Vec<usize> = out
        .iter()
        .map(|item| serde_json::to_vec(item).map(|v| v.len()).unwrap_or(512))
        .collect();
    let total_items_bytes: usize = item_sizes
        .iter()
        .enumerate()
        .map(|(idx, len)| len + usize::from(idx > 0))
        .sum();
    if extra_unscanned == 0 && BASE_OVERHEAD + total_items_bytes <= max_reply_bytes {
        return ConversationListResult {
            conversations: out,
            truncated: false,
            not_shown: None,
            note: None,
        };
    }

    let budget = max_reply_bytes.saturating_sub(BASE_OVERHEAD + TRUNCATED_OVERHEAD);
    let mut used = 0usize;
    let mut keep_count = 0usize;
    for (idx, item_len) in item_sizes.iter().enumerate() {
        let needed = item_len + usize::from(idx > 0);
        if used + needed > budget {
            break;
        }
        used += needed;
        keep_count += 1;
    }

    let total_available = out.len().saturating_add(extra_unscanned);
    let mut kept: Vec<ConversationSummary> = out.into_iter().take(keep_count).collect();
    loop {
        let not_shown = total_available.saturating_sub(kept.len()).max(1);
        let result = ConversationListResult {
            conversations: kept,
            truncated: true,
            not_shown: Some(not_shown),
            note: Some(format!("{not_shown} more conversations not shown")),
        };
        let encoded_len = serde_json::to_vec(&result)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if encoded_len <= max_reply_bytes || result.conversations.is_empty() {
            return result;
        }
        let mut next = result.conversations;
        next.pop();
        kept = next;
    }
}

pub(crate) async fn get_conversation(
    state: &AppState,
    project_dir: &Path,
    conversation_id: &str,
) -> Result<ConversationView, CutError> {
    let _io = state.chat_io_lock.lock().await;
    let live = active_turn_snapshot(state, project_dir, conversation_id);
    match storage::read_conversation_file(project_dir, conversation_id)? {
        storage::LoadedRecord::Readable(rec) => {
            let (turns, running) = overlay_turns(&rec, live.as_ref());
            Ok(ConversationView {
                id: rec.id,
                title: rec.title,
                origin: rec.origin,
                readable: true,
                unreadable_reason: None,
                running,
                created_at: Some(rec.created_at),
                updated_at: Some(rec.updated_at),
                turns,
            })
        }
        storage::LoadedRecord::Unreadable { id, title, reason } => Ok(ConversationView {
            id,
            title,
            origin: "unknown".into(),
            readable: false,
            unreadable_reason: Some(reason),
            running: false,
            created_at: None,
            updated_at: None,
            turns: Vec::new(),
        }),
        storage::LoadedRecord::Missing => Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("conversation '{conversation_id}' was not found in this project"),
            "no conversation with this id is stored in the open project",
        )
        .with_suggested_action(
            "call agent.chat_list to see the conversation ids of this project, then retry with one of them",
        )),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct MigrateConversationInput {
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) origin: Option<String>,
    #[serde(default)]
    pub(crate) turns: Vec<Value>,
}

pub(crate) async fn migrate_legacy(
    state: &AppState,
    project_dir: &Path,
    legacy_key: &str,
    entries: Vec<MigrateConversationInput>,
) -> Result<Vec<String>, CutError> {
    let _io = state.chat_io_lock.lock().await;
    if legacy_key.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "legacy_key must not be empty",
            "provide the source localStorage key string",
        ));
    }
    let existing_ids = storage::scan_chat_ids(project_dir)?;
    for id in &existing_ids {
        if let storage::LoadedRecord::Readable(rec) =
            storage::read_conversation_file(project_dir, id)?
        {
            if rec.source_legacy_key.as_deref() == Some(legacy_key) {
                return Ok(Vec::new());
            }
        }
    }
    let mut created = Vec::new();
    for entry in entries {
        if entry.turns.is_empty() {
            continue;
        }
        let turns: Vec<ChatTurnRecord> = entry
            .turns
            .iter()
            .enumerate()
            .map(|(idx, raw)| convert_legacy_turn(idx, raw))
            .collect();
        let now = storage::now_ts();
        let first_user = turns
            .iter()
            .find(|t| t.role == "user")
            .map(|t| t.text.as_str())
            .unwrap_or("Migrated chat");
        let title = entry
            .title
            .as_deref()
            .map(storage::make_title)
            .unwrap_or_else(|| storage::make_title(first_user));
        let origin = if entry.origin.as_deref() == Some("agent") {
            "agent"
        } else {
            "human"
        };
        let id = make_id("chat");
        let mut record = ConversationRecord {
            schema: CHAT_CONVERSATION_SCHEMA.into(),
            id: id.clone(),
            title,
            origin: origin.into(),
            created_at: now.clone(),
            updated_at: now,
            source_legacy_key: Some(legacy_key.to_string()),
            turns,
        };
        trim_turns(&mut record.turns);
        storage::write_conversation_file(project_dir, &mut record)?;
        created.push(id);
    }
    if !created.is_empty() {
        state.events.publish(Event::ChatUpdated {
            conversation_id: created[0].clone(),
            state: Some("migrated".into()),
            turn_id: None,
        });
    }
    Ok(created)
}

fn convert_legacy_turn(idx: usize, raw: &Value) -> ChatTurnRecord {
    let str_at = |k: &str| raw.get(k).and_then(Value::as_str).map(String::from);
    let role = if str_at("role").as_deref() == Some("agent") {
        "agent"
    } else {
        "user"
    };
    let id = str_at("id").unwrap_or_else(|| format!("legacy_{idx}"));
    let status = if role == "agent" {
        str_at("status").or_else(|| {
            raw.get("ok").and_then(Value::as_bool).map(|ok| {
                if ok {
                    "ok".into()
                } else {
                    "error".into()
                }
            })
        })
    } else {
        None
    };
    ChatTurnRecord {
        id,
        role: role.into(),
        text: str_at("text").unwrap_or_default(),
        created_at: str_at("created_at").unwrap_or_else(storage::now_ts),
        origin: str_at("origin"),
        status,
        agent: str_at("agent"),
        model: str_at("model"),
        error: str_at("error"),
        actions: raw
            .get("actions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        attachments: raw
            .get("attachments")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        evidence_ids: raw
            .get("evidence_ids")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        evidence_index_id: str_at("evidence_index_id"),
        target: raw.get("target").filter(|v| !v.is_null()).cloned(),
        plan: raw.get("plan").filter(|v| !v.is_null()).cloned(),
        review: raw.get("review").filter(|v| !v.is_null()).cloned(),
        cost_usd: raw.get("cost_usd").and_then(Value::as_f64),
        reverted: raw
            .get("reverted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

pub(crate) async fn patch_conversation(
    state: &AppState,
    project_dir: &Path,
    conversation_id: &str,
    op: &str,
    turn_id: Option<&str>,
    title: Option<&str>,
) -> Result<PatchOutcome, CutError> {
    let _io = state.chat_io_lock.lock().await;
    if op == "delete" {
        if active_turn_snapshot(state, project_dir, conversation_id).is_some() {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "cannot delete a conversation while a turn is running",
                "wait for the running turn to finish",
            ));
        }
        let removed = storage::delete_conversation_file(project_dir, conversation_id)?;
        if removed {
            state.events.publish(Event::ChatUpdated {
                conversation_id: conversation_id.into(),
                state: Some("deleted".into()),
                turn_id: None,
            });
        }
        return Ok(PatchOutcome {
            updated: removed,
            already_reverted: false,
        });
    }
    let storage::LoadedRecord::Readable(mut record) =
        storage::read_conversation_file(project_dir, conversation_id)?
    else {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("conversation '{conversation_id}' was not found in this project"),
            "no conversation with this id is stored in the open project",
        )
        .with_suggested_action(
            "call agent.chat_list to see the conversation ids of this project, then retry with one of them",
        ));
    };
    match op {
        "mark_reverted" => {
            let target_id = turn_id.ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "turn_id is required for mark_reverted",
                    "pass turn_id",
                )
            })?;
            let Some(turn) = record.turns.iter_mut().find(|t| t.id == target_id) else {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!(
                        "turn '{target_id}' is not in conversation '{conversation_id}'"
                    ),
                    "mark_reverted only accepts a turn id that belongs to this conversation",
                )
                .with_suggested_action(
                    "call agent.chat_get with this conversation_id to read its turn ids, then retry with one of them",
                ));
            };
            if turn.reverted {
                return Ok(PatchOutcome {
                    updated: false,
                    already_reverted: true,
                });
            }
            turn.reverted = true;
            record.updated_at = storage::now_ts();
            storage::write_conversation_file(project_dir, &mut record)?;
        }
        "set_title" => {
            let next = title.ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "title is required for set_title",
                    "pass title",
                )
            })?;
            record.title = storage::make_title(next);
            record.updated_at = storage::now_ts();
            storage::write_conversation_file(project_dir, &mut record)?;
        }
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown chat patch operation '{other}'"),
                "use mark_reverted, set_title, or delete",
            ));
        }
    }
    state.events.publish(Event::ChatUpdated {
        conversation_id: conversation_id.into(),
        state: Some("updated".into()),
        turn_id: turn_id.map(String::from),
    });
    Ok(PatchOutcome {
        updated: true,
        already_reverted: false,
    })
}

/// What `patch_conversation` did; `already_reverted` is set only when
/// `mark_reverted` met a known turn that was reverted before.
pub(crate) struct PatchOutcome {
    pub(crate) updated: bool,
    pub(crate) already_reverted: bool,
}
