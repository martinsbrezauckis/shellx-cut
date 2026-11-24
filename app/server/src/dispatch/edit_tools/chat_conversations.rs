//! Dispatch handlers for engine-owned project conversation history.

use super::*;
use crate::chat::conversations::{
    get_conversation, list_conversations, migrate_legacy, patch_conversation,
    MigrateConversationInput,
};
use cut_core::CutError;
use serde::Deserialize;
use std::path::PathBuf;

async fn open_project_dir(state: &AppState) -> Result<PathBuf, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    Ok(store.dir.clone())
}

pub(in crate::dispatch) async fn agent_chat_list(state: &AppState) -> Result<VerbResult, CutError> {
    let dir = open_project_dir(state).await?;
    let result = list_conversations(state, &dir).await?;
    Ok(VerbResult::ok(serde_json::to_value(result)?))
}

pub(in crate::dispatch) async fn agent_chat_get(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    struct Args {
        conversation_id: String,
    }
    let a: Args = parse_args(args)?;
    let dir = open_project_dir(state).await?;
    let conversation = get_conversation(state, &dir, &a.conversation_id).await?;
    Ok(VerbResult::ok(json!({ "conversation": conversation })))
}

pub(in crate::dispatch) async fn agent_chat_migrate(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    struct Args {
        legacy_key: String,
        conversations: Vec<MigrateConversationInput>,
    }
    let a: Args = parse_args(args)?;
    let dir = open_project_dir(state).await?;
    let ids = migrate_legacy(state, &dir, &a.legacy_key, a.conversations).await?;
    Ok(VerbResult::ok(json!({
        "migrated": ids.len(),
        "conversation_ids": ids,
    })))
}

pub(in crate::dispatch) async fn agent_chat_patch(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(Deserialize)]
    struct Args {
        conversation_id: String,
        op: String,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        title: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let dir = open_project_dir(state).await?;
    let updated = patch_conversation(
        state,
        &dir,
        &a.conversation_id,
        &a.op,
        a.turn_id.as_deref(),
        a.title.as_deref(),
    )
    .await?;
    let mut reply = json!({ "updated": updated.updated });
    if updated.already_reverted {
        reply["already_reverted"] = json!(true);
    }
    Ok(VerbResult::ok(reply))
}
