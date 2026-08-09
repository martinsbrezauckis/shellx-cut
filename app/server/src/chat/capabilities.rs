//! Schema-derived least-privilege policy for a contained `agent.chat` turn.
//!
//! `--allowedTools mcp__cutd__*` limits Claude to this server, not to a safe
//! subset of Cut verbs. Schema behavior metadata is total and enforced by
//! `mcp.rs` at discovery and invocation; unknown generated tools are denied.

use crate::registry::VerbSpec;
use cut_core::AgentChatCapability;

pub fn capability(spec: &VerbSpec) -> AgentChatCapability {
    spec.behavior.agent_chat
}

pub fn allows(spec: &VerbSpec) -> bool {
    matches!(
        capability(spec),
        AgentChatCapability::Inspect | AgentChatCapability::Edit
    )
}

/// This marker originates only in the sanitized broker environment. Requiring
/// the attributed proxy actor too avoids limiting a normal user-configured MCP.
pub fn active_broker_environment() -> bool {
    std::env::var("SHELLX_CUT_AGENT_CONTAINED").as_deref() == Ok("1")
        && std::env::var("CUTD_PROXY_ACTOR")
            .map(|actor| actor.starts_with("agent:") && actor.ends_with(":agent.chat"))
            .unwrap_or(false)
}

pub fn denied_message(verb: &str) -> String {
    format!(
        "agent.chat capability policy denied '{verb}': only inspection of the open project and reversible in-project edits are available"
    )
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
