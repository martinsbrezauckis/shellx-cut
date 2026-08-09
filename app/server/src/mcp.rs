//! mcp.rs — MCP server over stdio (server contract), generated from schema/verbs.json.
//!
//! Role: speaks newline-delimited JSON-RPC 2.0 on stdin/stdout — the exact
//! same request contract as the REST and CLI surfaces. The small protocol
//! surface stays local instead of adding an MCP framework dependency. Tools are
//! GENERATED from the verb registry: tool name = verb name with dots →
//! underscores, inputSchema = the verb's args schema verbatim. tools/call
//! funnels into dispatch() with the same envelope as REST, so both transports
//! expose the complete registry by construction.
//!
//! Methods: initialize, tools/list, tools/call, ping. Notifications (no id)
//! get no reply. Dependencies: state/dispatch/registry, tokio (current-thread
//! runtime provided by main). Primary callers: main.rs (`cutd mcp`).

use crate::dispatch::dispatch;
use crate::registry::VerbRegistry;
use crate::state::AppState;
use cut_core::{Actor, ActorKind};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

mod self_test;
pub(crate) use self_test::run as self_test;

/// Protocol version we advertise (matches the office-suite implementation era).
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// How tools/call executes a verb (the public single-state-holder contract: one state holder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpMode {
    /// PROXY every verb to the running `cutd serve` — the default; the server
    /// owns the project. Uses httpc live discovery first and falls back to
    /// 127.0.0.1:6161 when discovery is missing or stale.
    Proxy,
    /// Local in-process dispatch — ONLY legal when no server runs and the
    /// user passed --standalone (this process opens its own project).
    Standalone,
}

/// Run the blocking stdio loop until stdin closes. `state` may already have a
/// project open (cutd mcp --standalone --project <path> pre-opens it).
/// In Proxy mode the local state is used only for the registry (tools/list);
/// verbs go over REST so all surfaces share the server's single state.
pub async fn run_stdio(state: AppState, mode: McpMode) -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => break,
        };
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                // Parse error → JSON-RPC error with null id.
                write_msg(
                    &mut out,
                    &rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
                )?;
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        // Notifications (no id) get no reply per JSON-RPC 2.0.
        let Some(id) = id else { continue };
        let reply = match method {
            "initialize" => {
                // ECHO the client's requested protocolVersion when it sends one
                // (MCP version negotiation): a modern client (e.g. Claude Code,
                // which requests 2025-06-18) can drop a server that answers with
                // an OLDER hardcoded version it didn't ask for — the server then
                // shows as "connecting" and exposes no tools. Falling back to our
                // own MCP_PROTOCOL_VERSION when the client omits it keeps older
                // clients working. cutd is
                // returning 2024-11-05, so the agent-chat MCP handshake hung.)
                let pv = msg
                    .get("params")
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(MCP_PROTOCOL_VERSION);
                rpc_result(
                    id,
                    json!({
                        "protocolVersion": pv,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "shellx-cut", "version": env!("CARGO_PKG_VERSION")}
                    }),
                )
            }
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(
                id,
                json!({"tools": list_tools_for_agent_environment(&state)}),
            ),
            "tools/call" => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                let tool = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match state.registry.verb_for_tool(tool) {
                    None => rpc_error(id, -32602, &format!("unknown tool '{tool}'")),
                    Some(spec) => {
                        let verb_name = spec.name.clone();
                        if crate::chat::capabilities::active_broker_environment()
                            && !crate::chat::capabilities::allows(spec)
                        {
                            // A contained agent must not regain a dangerous Cut
                            // capability merely by guessing its generated tool
                            // name. Return an ordinary MCP tool error so the
                            // agent sees an actionable, typed rejection.
                            rpc_result(id, denied_agent_chat_tool_call(&verb_name))
                        } else {
                            // Envelope JSON regardless of execution path; isError
                            // mirrors envelope.ok so clients can branch.
                            let envelope_json = match mode {
                                McpMode::Proxy => {
                                    match crate::httpc::post_verb(&verb_name, &args) {
                                        Ok(v) => v,
                                        // Server unreachable → actionable envelope, not
                                        // a protocol error (agents read envelopes).
                                        Err(e) => json!({"ok": false, "error": e}),
                                    }
                                }
                                McpMode::Standalone => {
                                    let actor = Actor {
                                        kind: ActorKind::Agent,
                                        name: "mcp-client".into(),
                                        via: "mcp".into(),
                                        request: None,
                                    };
                                    serde_json::to_value(
                                        dispatch(&state, &verb_name, args, actor).await,
                                    )
                                    .unwrap_or_default()
                                }
                            };
                            rpc_result(id, tool_call_result(envelope_json))
                        }
                    }
                }
            }
            other => rpc_error(id, -32601, &format!("method '{other}' not found")),
        };
        write_msg(&mut out, &reply)?;
    }
    Ok(())
}

/// Trim a long verb description to a concise MCP tool description at a sentence/
/// word boundary (char-safe). The full prose lives in verbs.json / the registry;
/// the MCP `tools/list` only needs enough for the agent to pick the tool (name +
/// args schema carry the rest). cutd's full
/// descriptions made the 168-tool `tools/list` ~234 KB, and Claude Code's stdio
/// MCP transport reported "Connected · tools fetch failed" on it (the agent-chat
/// MCP handshake then exposed no tools). Concise descriptions cut it to a size
/// the client accepts, with no loss the agent can't recover from the schema.
fn concise(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    let cut = truncated
        .rfind(". ")
        .map(|i| i + 1)
        .or_else(|| truncated.rfind(' '))
        .unwrap_or(truncated.len());
    format!("{}…", truncated[..cut].trim_end())
}

/// Recursively trim every `description` string inside a JSON Schema to `max`
/// chars (char-safe), preserving structure (types, enums, required, items,
/// nesting). Keeps the args schema USABLE (the agent still sees arg names,
/// types, enums, and which are required) while cutting the `tools/list` payload
/// to a size stdio MCP clients accept (see [`concise`] / the fix).
fn trim_schema_descriptions(v: &Value, max: usize) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                if k == "description" && val.is_string() {
                    // max == 0 → DROP the nested arg-description entirely (the
                    // arg name + type + enum + required already tell the agent
                    // how to call it, and verb errors guide the rest); a positive
                    // max truncates instead. This is what brings tools/list under
                    // the stdio client's size ceiling.
                    if max == 0 {
                        continue;
                    }
                    out.insert(k.clone(), json!(concise(val.as_str().unwrap_or(""), max)));
                    continue;
                }
                out.insert(k.clone(), trim_schema_descriptions(val, max));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|x| trim_schema_descriptions(x, max))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// True if a JSON value contains a `$ref` anywhere (a schema referencing an
/// external/custom URI a generic MCP client can't resolve).
fn json_contains_ref(v: &Value) -> bool {
    match v {
        Value::Object(map) => map.contains_key("$ref") || map.values().any(json_contains_ref),
        Value::Array(arr) => arr.iter().any(json_contains_ref),
        _ => false,
    }
}

/// Generate the MCP tool list from the registry (public verb contract: "MCP tools are
/// generated from it"). The description is the verb's purpose (concise) + the
/// success shape so agents know it without a second lookup; arg-schema prose is
/// trimmed so the whole `tools/list` stays within stdio MCP client limits.
pub fn list_tools(state: &AppState) -> Vec<Value> {
    list_tools_matching(state, |_| true)
}

/// The brokered Agent Chat gets a deliberately smaller tool list than a normal
/// user-configured MCP client. The same policy is checked again at tools/call.
pub(crate) fn list_agent_chat_tools(state: &AppState) -> Vec<Value> {
    list_tools_matching(state, crate::chat::capabilities::allows)
}

fn list_tools_for_agent_environment(state: &AppState) -> Vec<Value> {
    if crate::chat::capabilities::active_broker_environment() {
        list_agent_chat_tools(state)
    } else {
        list_tools(state)
    }
}

fn list_tools_matching<F>(state: &AppState, include: F) -> Vec<Value>
where
    F: Fn(&crate::registry::VerbSpec) -> bool,
{
    state
        .registry
        .verbs
        .iter()
        .filter(|v| include(v))
        .map(|v| {
            let mut tool = json!({
                "name": VerbRegistry::mcp_tool_name(&v.name),
                "description": format!("{} Returns: {}", concise(&v.description, 400), concise(&v.result, 200)),
                "inputSchema": trim_schema_descriptions(&v.args, 160),
            });
            // Forward the machine-readable result contract as MCP outputSchema
            // when the verb declares one, but only if it is self-contained.
            // A schema using `$ref` (ours point at a custom `shellx-cut://`
            // receipts-schema URI) is UNRESOLVABLE by a generic MCP client;
            // Claude Code rejects the ENTIRE tools/list on it ("Connected · tools
            // fetch failed"), which silently broke agent.chat. Inline schemas are
            // fine.
            if let Some(rs) = &v.result_schema {
                if !json_contains_ref(rs) {
                    tool["outputSchema"] = rs.clone();
                }
            }
            tool
        })
        .collect()
}

fn denied_agent_chat_tool_call(verb: &str) -> Value {
    tool_call_result(json!({
        "ok": false,
        "error": {
            "code": "agent_capability_denied",
            "message": crate::chat::capabilities::denied_message(verb),
        },
    }))
}

/// Build the `tools/call` result body from a verb envelope (dispatch's
/// `{ok, result?, error?, ...}`). DUAL output:
///   - `content` text block = back-compat fallback (clients that only read
///     `content` get the envelope as a JSON string);
///   - `structuredContent` = the SAME envelope as typed JSON (MCP structured
///     tool output) so an agent reasons over the receipt / fix_actions / diff
///     directly without re-parsing a stringified blob.
/// `isError` mirrors `envelope.ok` so clients can branch without inspecting the
/// payload. Pure (no I/O) so the shape is unit-testable.
fn tool_call_result(envelope_json: Value) -> Value {
    let is_error = !envelope_json
        .get("ok")
        .and_then(|o| o.as_bool())
        .unwrap_or(false);
    json!({
        "content": [{"type": "text", "text": envelope_json.to_string()}],
        "structuredContent": envelope_json,
        "isError": is_error,
    })
}

/// JSON-RPC 2.0 success frame.
fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// JSON-RPC 2.0 error frame.
fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One newline-delimited frame out.
fn write_msg(out: &mut impl Write, msg: &Value) -> std::io::Result<()> {
    writeln!(out, "{msg}")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registry verb materializes as exactly one MCP tool with a valid
    /// MCP tool name — the MCP half of the 100%-coverage invariant.
    #[test]
    fn tools_cover_all_verbs() {
        let state = AppState::new();
        let tools = list_tools(&state);
        assert_eq!(tools.len(), state.registry.verbs.len());
        for t in &tools {
            let name = t["name"].as_str().unwrap();
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "invalid MCP tool name {name}"
            );
            assert!(t["inputSchema"].is_object());
        }
    }

    #[test]
    fn deprecated_legacy_inverse_option_survives_mcp_schema_projection() {
        let state = AppState::new();
        let tool = list_tools(&state)
            .into_iter()
            .find(|tool| tool["name"] == "edit_add_marker")
            .expect("edit.add_marker must be exposed through MCP");
        let option = &tool["inputSchema"]["properties"]["include_inverse"];
        assert_eq!(option["type"], "boolean");
        assert_eq!(option["deprecated"], true);
        assert!(
            option["description"]
                .as_str()
                .is_some_and(|description| description.contains("Deprecated compatibility")),
            "MCP must expose the compatibility meaning, not stale inverse semantics: {option}"
        );
    }

    #[test]
    fn agent_chat_tools_hide_and_reject_disallowed_cut_verbs() {
        let state = AppState::new();
        let tools = list_agent_chat_tools(&state);
        let exposed: Vec<_> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(exposed.contains(&"edit_add_marker"));
        assert!(exposed.contains(&"project_health"));
        for blocked in [
            "project_open",
            "media_import",
            "assets_fetch",
            "export_frame",
            "system_fetch_tool",
            "agent_chat",
            "project_revert",
        ] {
            assert!(
                !exposed.contains(&blocked),
                "blocked tool leaked: {blocked}"
            );
        }
        let denied = denied_agent_chat_tool_call("project.open");
        assert_eq!(denied["isError"], json!(true));
        assert_eq!(
            denied["structuredContent"]["error"]["code"],
            json!("agent_capability_denied")
        );
    }

    #[test]
    fn recovery_status_is_listed_for_mcp_but_denied_to_agent_chat() {
        let state = AppState::new();
        let recovery = list_tools(&state)
            .into_iter()
            .find(|tool| tool["name"] == json!("screen_record_recovery_status"))
            .expect("generated recovery-status tool must be MCP-visible");
        assert_eq!(
            recovery["inputSchema"]["properties"]["limit"]["maximum"],
            100
        );
        assert!(
            !list_agent_chat_tools(&state)
                .iter()
                .any(|tool| tool["name"] == json!("screen_record_recovery_status")),
            "read-only recovery receipts are deliberately not agent-chat capabilities"
        );
    }

    /// tools/call returns BOTH a text fallback and structuredContent carrying
    /// the same envelope (so an agent can read fix_actions/receipt typed). The
    /// text block stays a valid JSON string of the envelope (back-compat).
    #[test]
    fn tool_call_result_carries_structured_and_text() {
        let env = json!({
            "ok": true,
            "result": {"render_id": "render_001", "pass": false,
                       "fix_actions": [{"check": "lufs", "fix_verb": "render.final", "auto_fixable": true}]},
        });
        let r = tool_call_result(env.clone());
        assert_eq!(r["isError"], json!(false));
        // structuredContent IS the envelope, typed — agent reads it directly.
        assert_eq!(r["structuredContent"], env);
        assert_eq!(
            r["structuredContent"]["result"]["fix_actions"][0]["fix_verb"],
            json!("render.final")
        );
        // text block remains a parseable JSON string of the same envelope.
        let text = r["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed, env);
    }

    /// isError mirrors envelope.ok=false so clients branch without parsing.
    #[test]
    fn tool_call_result_flags_error_envelope() {
        let env = json!({"ok": false, "error": {"code": "no_project", "message": "x"}});
        let r = tool_call_result(env);
        assert_eq!(r["isError"], json!(true));
    }
}
