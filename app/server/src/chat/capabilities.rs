//! Schema-derived least-privilege policy for a contained `agent.chat` turn.
//!
//! `--allowedTools mcp__cutd__*` limits Claude to this server, not to a safe
//! subset of Cut verbs. Schema behavior metadata is total and enforced by
//! `mcp.rs` at discovery and invocation; unknown generated tools are denied.

use crate::registry::VerbSpec;
use cut_core::AgentChatCapability;

/// Broker-owned marker forwarded only to the `cutd mcp` child for an Agent Chat
/// turn. A normal user-configured MCP server never receives this marker and
/// therefore retains the full, machine-local MCP surface.
pub const RESTRICTED_MCP_MARKER: &str = "SHELLX_CUT_AGENT_CONTAINED";
pub const RESTRICTED_MCP_MARKER_VALUE: &str = "1";

pub fn capability(spec: &VerbSpec) -> AgentChatCapability {
    spec.behavior.agent_chat
}

pub fn allows(spec: &VerbSpec) -> bool {
    matches!(
        capability(spec),
        AgentChatCapability::Inspect | AgentChatCapability::Edit
    )
}

/// Requiring the attributed proxy actor too avoids limiting a normal
/// user-configured MCP. The marker is provider-independent: every Agent Chat
/// broker route must pass this same pair to its `cutd mcp` child.
pub fn restricted_mcp_environment(marker: Option<&str>, actor: Option<&str>) -> bool {
    marker == Some(RESTRICTED_MCP_MARKER_VALUE)
        && actor.is_some_and(|actor| actor.starts_with("agent:") && actor.ends_with(":agent.chat"))
}

pub fn active_broker_environment() -> bool {
    let marker = std::env::var(RESTRICTED_MCP_MARKER).ok();
    let actor = std::env::var("CUTD_PROXY_ACTOR").ok();
    restricted_mcp_environment(marker.as_deref(), actor.as_deref())
}

pub fn denied_message(verb: &str) -> String {
    format!(
        "agent.chat capability policy denied '{verb}': only inspection of the open project and reversible in-project edits are available"
    )
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
