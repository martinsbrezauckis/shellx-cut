//! Read-only MCP resources exposing the untruncated verb registry and bundled agent docs.

use super::denied_by_restricted_mcp_policy;
use crate::packaged_resources::agent_docs_root;
use crate::registry::VerbRegistry;
use crate::state::AppState;
use serde_json::{json, Value};

const VERBS_URI: &str = "shellx-cut://verbs";
const VERB_URI_PREFIX: &str = "shellx-cut://verb/";
const SKILL_DOC_URI: &str = "shellx-cut://docs/skill";
const REFERENCE_DOC_URI: &str = "shellx-cut://docs/reference";

pub(super) const INITIALIZE_INSTRUCTIONS: &str =
    "tools/list returns concise tool summaries within client size budgets. Full untruncated verb descriptions, argument prose, and shared mutation controls (request_id, expected_revision) live in MCP resources (shellx-cut://verbs, shellx-cut://verb/{name}, shellx-cut://docs/skill, shellx-cut://docs/reference).";

/// List the static MCP resources visible to the current client environment.
pub(super) fn list_resources(restricted_mcp: bool) -> Value {
    let mut resources = vec![json!({
        "uri": VERBS_URI,
        "name": "ShellX Cut full verb registry",
        "description": "Complete untruncated verb descriptions, input schemas, result contracts, and shared mutation controls.",
        "mimeType": "application/json",
    })];
    if !restricted_mcp {
        if let Some(root) = agent_docs_root() {
            if root.join("skill/shellx-cut/SKILL.md").is_file() {
                resources.push(json!({
                    "uri": SKILL_DOC_URI,
                    "name": "ShellX Cut agent skill guide",
                    "description": "End-to-end agent workflow guide, terminology table, and operational rules.",
                    "mimeType": "text/markdown",
                }));
            }
            if root.join("skill/shellx-cut/reference.md").is_file() {
                resources.push(json!({
                    "uri": REFERENCE_DOC_URI,
                    "name": "ShellX Cut full verb reference",
                    "description": "Complete 333-verb table with arguments, returns, ops/job facts, and requirements.",
                    "mimeType": "text/markdown",
                }));
            }
        }
    }
    json!({ "resources": resources })
}

/// List parameterized MCP resource templates for per-verb full contracts.
pub(super) fn list_resource_templates() -> Value {
    json!({
        "resourceTemplates": [
            {
                "uriTemplate": "shellx-cut://verb/{name}",
                "name": "ShellX Cut verb contract",
                "description": "Full untruncated description, input schema, result contract, and behavior metadata for one verb (dot or underscore name).",
                "mimeType": "application/json",
            }
        ]
    })
}

/// Read one MCP resource, enforcing the same restricted policy as `tools/list`.
pub(super) fn read_resource(
    state: &AppState,
    uri: &str,
    restricted_mcp: bool,
) -> Result<Value, (i64, String)> {
    if uri == VERBS_URI {
        let text = full_verbs_json(state, restricted_mcp)?;
        return Ok(text_contents(uri, "application/json", text));
    }
    if uri == SKILL_DOC_URI {
        return read_agent_doc(uri, "skill/shellx-cut/SKILL.md", restricted_mcp);
    }
    if uri == REFERENCE_DOC_URI {
        return read_agent_doc(uri, "skill/shellx-cut/reference.md", restricted_mcp);
    }
    if let Some(raw_name) = uri.strip_prefix(VERB_URI_PREFIX) {
        if raw_name.is_empty() || raw_name.contains('/') || raw_name.contains('\\') {
            return Err((-32002, format!("MCP resource not found: {uri}")));
        }
        let Some(spec) = state
            .registry
            .get(raw_name)
            .or_else(|| state.registry.verb_for_tool(raw_name))
        else {
            return Err((-32002, format!("MCP resource not found: {uri}")));
        };
        if denied_by_restricted_mcp_policy(spec, restricted_mcp) {
            return Err((
                -32002,
                format!("resource denied by agent.chat policy: {uri}"),
            ));
        }
        let verb_json = state
            .registry
            .public_json()
            .get("verbs")
            .and_then(Value::as_array)
            .and_then(|verbs| {
                verbs
                    .iter()
                    .find(|entry| entry.get("name").and_then(Value::as_str) == Some(&spec.name))
            })
            .cloned()
            .ok_or_else(|| (-32603, format!("missing verb payload for '{}'", spec.name)))?;
        let mut payload = verb_json;
        if let Some(map) = payload.as_object_mut() {
            map.insert(
                "mcp_tool_name".into(),
                json!(VerbRegistry::mcp_tool_name(&spec.name)),
            );
        }
        let text = serde_json::to_string(&payload)
            .map_err(|error| (-32603, format!("failed to encode verb resource: {error}")))?;
        return Ok(text_contents(uri, "application/json", text));
    }
    Err((-32002, format!("MCP resource not found: {uri}")))
}

fn full_verbs_json(state: &AppState, restricted_mcp: bool) -> Result<String, (i64, String)> {
    let public = state.registry.public_json();
    if !restricted_mcp {
        return serde_json::to_string(public)
            .map_err(|error| (-32603, format!("failed to encode verbs resource: {error}")));
    }
    let mut filtered = public.clone();
    if let Some(verbs) = filtered.get_mut("verbs").and_then(Value::as_array_mut) {
        verbs.retain(|entry| {
            entry
                .get("name")
                .and_then(Value::as_str)
                .and_then(|name| state.registry.get(name))
                .is_some_and(|spec| !denied_by_restricted_mcp_policy(spec, true))
        });
    }
    serde_json::to_string(&filtered)
        .map_err(|error| (-32603, format!("failed to encode verbs resource: {error}")))
}

fn read_agent_doc(
    uri: &str,
    relative_path: &str,
    restricted_mcp: bool,
) -> Result<Value, (i64, String)> {
    if restricted_mcp {
        return Err((
            -32002,
            format!("resource denied by agent.chat policy: {uri}"),
        ));
    }
    let Some(root) = agent_docs_root() else {
        return Err((-32002, format!("agent docs are not bundled: {uri}")));
    };
    let text = std::fs::read_to_string(root.join(relative_path))
        .map_err(|_| (-32002, format!("MCP resource not found: {uri}")))?;
    Ok(text_contents(uri, "text/markdown", text))
}

fn text_contents(uri: &str, mime_type: &str, text: String) -> Value {
    json!({
        "contents": [{
            "uri": uri,
            "mimeType": mime_type,
            "text": text,
        }]
    })
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
