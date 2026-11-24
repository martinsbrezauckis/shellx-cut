//! Serializable schema types for engine-owned project conversations.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) const CHAT_CONVERSATION_SCHEMA: &str = "shellx-cut/chat-conversation/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChatActionRef {
    pub(crate) op_id: String,
    pub(crate) verb: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatTurnRecord {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) actions: Vec<ChatActionRef>,
    #[serde(default)]
    pub(crate) attachments: Vec<String>,
    #[serde(default)]
    pub(crate) evidence_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evidence_index_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) plan: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) review: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cost_usd: Option<f64>,
    #[serde(default)]
    pub(crate) reverted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConversationRecord {
    pub(crate) schema: String,
    pub(crate) id: String,
    pub(crate) title: String,
    #[serde(default = "default_origin")]
    pub(crate) origin: String,
    #[serde(default)]
    pub(crate) created_at: String,
    #[serde(default)]
    pub(crate) updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_legacy_key: Option<String>,
    #[serde(default)]
    pub(crate) turns: Vec<ChatTurnRecord>,
}

fn default_origin() -> String {
    "human".into()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConversationSummary {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) origin: String,
    pub(crate) readable: bool,
    pub(crate) unreadable_reason: Option<String>,
    pub(crate) running: bool,
    pub(crate) turn_count: usize,
    pub(crate) created_at: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) last_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConversationListResult {
    pub(crate) conversations: Vec<ConversationSummary>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) not_shown: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConversationView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) origin: String,
    pub(crate) readable: bool,
    pub(crate) unreadable_reason: Option<String>,
    pub(crate) running: bool,
    pub(crate) created_at: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) turns: Vec<ChatTurnRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveChatTurn {
    pub(crate) project_dir: PathBuf,
    pub(crate) conversation_id: String,
    pub(crate) turn_id: String,
    pub(crate) agent: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) started_at: String,
    pub(crate) actions: Vec<ChatActionRef>,
}

#[derive(Debug, Default)]
pub(crate) struct ActiveChatState {
    pub(super) turns: HashMap<(PathBuf, String), ActiveChatTurn>,
}
