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

mod resources;
mod self_test;
pub(crate) use self_test::run as self_test;

/// Protocol version we advertise (matches the office-suite implementation era).
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_TOOLS_LIST_BYTES: usize = 384 * 1024;
#[cfg(test)]
const TARGET_TOOLS_LIST_PERCENT: usize = 80;
// Keep tools/list well below the client ceiling as the public registry grows.
// Tool names, argument names, types, enums, required fields, shared mutation
// constraints, and inline outputSchemas remain intact; long prose lives in
// skill/reference docs and the MCP resources (`shellx-cut://verbs`,
// `shellx-cut://verb/{name}`).
const MAX_TOOL_DESCRIPTION_CHARS: usize = 120;
const MAX_TOOL_RESULT_DESCRIPTION_CHARS: usize = 55;
const MAX_INPUT_SCHEMA_DESCRIPTION_CHARS: usize = 50;
const MAX_DEPRECATED_SCHEMA_DESCRIPTION_CHARS: usize = 35;
const SHARED_MUTATION_CONTROL_NAMES: [&str; 2] = ["request_id", "expected_revision"];

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
        // Notifications (no id) get no reply per JSON-RPC 2.0.
        let Some(id) = id else { continue };
        let restricted_mcp = crate::chat::capabilities::active_broker_environment();
        let reply = match handle_sync_method(&state, mode, &msg, restricted_mcp) {
            Some(reply) => reply,
            None => handle_tools_call(&state, mode, id, &msg, restricted_mcp).await,
        };
        write_msg(&mut out, &reply)?;
    }
    Ok(())
}

/// Handle synchronous, read-only JSON-RPC MCP methods (`initialize`, `ping`,
/// `tools/list`, `resources/list`, `resources/templates/list`, `resources/read`).
/// Returns `None` for `tools/call`, which dispatches asynchronously.
pub(super) fn handle_sync_method(
    state: &AppState,
    _mode: McpMode,
    msg: &Value,
    restricted_mcp: bool,
) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            // ECHO the client's requested protocolVersion when it sends one
            // (MCP version negotiation): a modern client (e.g. Claude Code,
            // which requests 2025-06-18) can drop a server that answers with
            // an OLDER hardcoded version it didn't ask for — the server then
            // shows as "connecting" and exposes no tools. Falling back to our
            // own MCP_PROTOCOL_VERSION when the client omits it keeps older
            // clients working.
            let pv = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or(MCP_PROTOCOL_VERSION);
            Some(rpc_result(
                id,
                json!({
                    "protocolVersion": pv,
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {"name": "shellx-cut", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": resources::INITIALIZE_INSTRUCTIONS,
                }),
            ))
        }
        "ping" => Some(rpc_result(id, json!({}))),
        "tools/list" => Some(rpc_result(
            id,
            json!({"tools": list_tools_for_mcp_environment(state, restricted_mcp)}),
        )),
        "resources/list" => Some(rpc_result(id, resources::list_resources(restricted_mcp))),
        "resources/templates/list" => Some(rpc_result(id, resources::list_resource_templates())),
        "resources/read" => {
            let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                return Some(rpc_error(
                    id,
                    -32602,
                    "invalid params: expected string 'uri'",
                ));
            };
            match resources::read_resource(state, uri, restricted_mcp) {
                Ok(res) => Some(rpc_result(id, res)),
                Err((code, message)) => Some(rpc_error(id, code, &message)),
            }
        }
        "tools/call" => None,
        other => Some(rpc_error(
            id,
            -32601,
            &format!("method '{other}' not found"),
        )),
    }
}

async fn handle_tools_call(
    state: &AppState,
    mode: McpMode,
    id: Value,
    msg: &Value,
    restricted_mcp: bool,
) -> Value {
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let tool = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match state.registry.verb_for_tool(tool) {
        None => rpc_error(id, -32602, &format!("unknown tool '{tool}'")),
        Some(spec) => {
            let verb_name = spec.name.clone();
            if denied_by_restricted_mcp_policy(spec, restricted_mcp) {
                // A contained agent must not regain a dangerous Cut
                // capability merely by guessing its generated tool
                // name. Return an ordinary MCP tool error so the
                // agent sees an actionable, typed rejection.
                rpc_result(id, denied_agent_chat_tool_call(&verb_name))
            } else {
                // Envelope JSON regardless of execution path; isError
                // mirrors envelope.ok so clients can branch.
                let envelope_json = match mode {
                    McpMode::Proxy => match crate::httpc::post_verb(&verb_name, &args) {
                        Ok(v) => v,
                        // Server unreachable → actionable envelope, not
                        // a protocol error (agents read envelopes).
                        Err(e) => json!({"ok": false, "error": e}),
                    },
                    McpMode::Standalone => {
                        let actor = Actor {
                            kind: ActorKind::Agent,
                            name: "mcp-client".into(),
                            via: "mcp".into(),
                            request: None,
                        };
                        serde_json::to_value(dispatch(state, &verb_name, args, actor).await)
                            .unwrap_or_default()
                    }
                };
                rpc_result(id, tool_call_result(envelope_json))
            }
        }
    }
}

fn is_abbreviation_dot(s: &str, dot_byte: usize) -> bool {
    let before = &s[..=dot_byte];
    ["e.g.", "i.e.", "etc.", "vs."].iter().any(|abbr| {
        if before.len() < abbr.len() {
            return false;
        }
        let start = before.len() - abbr.len();
        before.is_char_boundary(start)
            && before[start..].eq_ignore_ascii_case(abbr)
            && (start == 0
                || before[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| !ch.is_alphanumeric()))
    })
}

fn first_sentence_break(s: &str) -> Option<usize> {
    for (idx, _) in s.match_indices(". ") {
        if !is_abbreviation_dot(s, idx) {
            return Some(idx + 1);
        }
    }
    None
}

fn last_sentence_break(s: &str, min_bytes: usize) -> Option<usize> {
    for (idx, _) in s.rmatch_indices(". ") {
        if idx >= min_bytes && !is_abbreviation_dot(s, idx) {
            return Some(idx + 1);
        }
    }
    None
}

/// Trim a long verb description to a concise MCP tool description at a sentence/
/// word boundary (char-safe). The full prose lives in verbs.json / MCP resources
/// (`shellx-cut://verbs`, `shellx-cut://verb/{name}`) and `skill/shellx-cut/`.
fn concise(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let limit = max.saturating_sub(1);
    let end_byte = s
        .char_indices()
        .nth(limit)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let truncated = &s[..end_byte];
    let cut = last_sentence_break(truncated, 12)
        .or_else(|| truncated.rfind(' '))
        .unwrap_or(truncated.len());
    format!("{}…", truncated[..cut].trim_end())
}

fn summarize_tool_description(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if let Some(end) = first_sentence_break(s) {
        let first = &s[..end];
        if (24..=max).contains(&first.chars().count()) {
            return first.to_string();
        }
    }
    concise(s, max)
}

fn summarize_result_shape(s: &str, max: usize) -> String {
    let base = s.split(" — ").next().unwrap_or(s);
    concise(base, max)
}

fn summarize_arg_description(s: &str, max: usize, deprecated: bool) -> String {
    if deprecated {
        return concise(s, MAX_DEPRECATED_SCHEMA_DESCRIPTION_CHARS);
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    if let Some(end) = first_sentence_break(s) {
        let first = &s[..end];
        if (20..=max).contains(&first.chars().count()) {
            return first.to_string();
        }
    }
    if let Some(semi) = s.find("; ") {
        let clause = &s[..semi];
        if (20..=max).contains(&clause.chars().count()) {
            return clause.to_string();
        }
    }
    concise(s, max)
}

/// Recursively trim every `description` string inside a JSON Schema to `max`
/// chars (char-safe), preserving structure (types, enums, required, items,
/// nesting). Keeps the args schema USABLE (the agent still sees arg names,
/// types, enums, and which are required) while cutting the `tools/list` payload
/// to a size stdio MCP clients accept.
fn trim_schema_descriptions(v: &Value, max: usize) -> Value {
    match v {
        Value::Object(map) => {
            let deprecated = map.get("deprecated").and_then(Value::as_bool) == Some(true);
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                if k == "description" && val.is_string() {
                    if max == 0 {
                        continue;
                    }
                    out.insert(
                        k.clone(),
                        json!(summarize_arg_description(
                            val.as_str().unwrap_or(""),
                            max,
                            deprecated
                        )),
                    );
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

/// The registry injects these two shared controls into every verb input schema
/// before validation. Their descriptions repeat across every entry in a
/// full MCP list, while JSON Schema treats `description` as an annotation. Keep
/// every machine-readable control constraint and compact only that duplicate
/// prose; the complete annotation remains in the canonical verb registry.
fn compact_mcp_input_schema(v: &Value) -> Value {
    let mut schema = trim_schema_descriptions(v, MAX_INPUT_SCHEMA_DESCRIPTION_CHARS);
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    for name in SHARED_MUTATION_CONTROL_NAMES {
        if let Some(control) = properties.get_mut(name).and_then(Value::as_object_mut) {
            control.remove("description");
        }
    }
    schema
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
/// generated from it"). The description is a concise purpose summary (+ shape
/// when no inline `outputSchema` is attached); full docs live in MCP resources
/// and `skill/shellx-cut/`.
pub fn list_tools(state: &AppState) -> Vec<Value> {
    list_tools_matching(state, |_| true)
}

/// The brokered Agent Chat gets a deliberately smaller tool list than a normal
/// user-configured MCP client. The same policy is checked again at tools/call.
pub(crate) fn list_agent_chat_tools(state: &AppState) -> Vec<Value> {
    list_tools_matching(state, crate::chat::capabilities::allows)
}

fn list_tools_for_mcp_environment(state: &AppState, restricted_mcp: bool) -> Vec<Value> {
    if restricted_mcp {
        list_agent_chat_tools(state)
    } else {
        list_tools(state)
    }
}

pub(super) fn denied_by_restricted_mcp_policy(
    spec: &crate::registry::VerbSpec,
    restricted_mcp: bool,
) -> bool {
    restricted_mcp && !crate::chat::capabilities::allows(spec)
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
            let inline_output_schema = v.result_schema.as_ref().filter(|rs| !json_contains_ref(rs));
            let summary = summarize_tool_description(&v.description, MAX_TOOL_DESCRIPTION_CHARS);
            let description = if inline_output_schema.is_some() {
                summary
            } else {
                format!(
                    "{} Returns: {}",
                    summary,
                    summarize_result_shape(&v.result, MAX_TOOL_RESULT_DESCRIPTION_CHARS),
                )
            };
            let mut tool = json!({
                "name": VerbRegistry::mcp_tool_name(&v.name),
                "description": description,
                "inputSchema": compact_mcp_input_schema(&v.args),
            });
            // Forward the machine-readable result contract as MCP outputSchema
            // when the verb declares one, but only if it is self-contained.
            // A schema using `$ref` (ours point at a custom `shellx-cut://`
            // receipts-schema URI) is UNRESOLVABLE by a generic MCP client;
            // Claude Code rejects the ENTIRE tools/list on it ("Connected · tools
            // fetch failed"), which silently broke agent.chat. Inline schemas are
            // fine.
            if let Some(rs) = inline_output_schema {
                tool["outputSchema"] = rs.clone();
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
    fn tools_list_reply_stays_within_the_supported_client_budget() {
        let state = AppState::new();
        let reply = rpc_result(json!(3), json!({"tools": list_tools(&state)}));
        let bytes = reply.to_string().len();
        assert!(
            bytes <= MAX_TOOLS_LIST_BYTES,
            "MCP tools/list reply is {bytes} bytes; supported client budget is {MAX_TOOLS_LIST_BYTES}"
        );
    }

    #[test]
    fn tools_list_reply_stays_under_eighty_percent_of_budget() {
        let state = AppState::new();
        let reply = rpc_result(json!(3), json!({"tools": list_tools(&state)}));
        let bytes = reply.to_string().len();
        let pct = (bytes as f64 / MAX_TOOLS_LIST_BYTES as f64) * 100.0;
        println!("MCP tools/list budget usage: {bytes}/{MAX_TOOLS_LIST_BYTES} bytes ({pct:.2}%)");
        assert!(
            bytes * 100 < MAX_TOOLS_LIST_BYTES * TARGET_TOOLS_LIST_PERCENT,
            "MCP tools/list reply is {bytes} bytes ({pct:.2}% of {MAX_TOOLS_LIST_BYTES}); target is under {TARGET_TOOLS_LIST_PERCENT}%"
        );
    }

    #[test]
    fn render_final_tool_summary_leads_with_full_render() {
        let state = AppState::new();
        let tool = list_tools(&state)
            .into_iter()
            .find(|tool| tool["name"] == "render_final")
            .expect("render.final must be exposed through MCP");
        let desc = tool["description"].as_str().unwrap_or("");
        assert!(
            desc.starts_with("Full render"),
            "render_final summary must lead with what it does, got: {desc}"
        );
    }

    #[test]
    fn mcp_input_schemas_keep_shared_mutation_control_constraints_without_repeating_prose() {
        let state = AppState::new();
        for tool in list_tools(&state) {
            let properties = tool["inputSchema"]["properties"]
                .as_object()
                .expect("every MCP tool must retain an input properties object");
            let request_id = &properties["request_id"];
            assert_eq!(request_id["type"], json!("string"));
            assert_eq!(request_id["minLength"], json!(8));
            assert_eq!(request_id["maxLength"], json!(128));
            assert_eq!(
                request_id["pattern"],
                json!("^[A-Za-z0-9][A-Za-z0-9._:-]*$")
            );
            assert!(
                request_id.get("description").is_none(),
                "{} repeated request_id prose in tools/list",
                tool["name"]
            );

            let expected_revision = &properties["expected_revision"];
            assert_eq!(expected_revision["type"], json!("string"));
            assert_eq!(expected_revision["pattern"], json!("^op_[0-9]{6,}$"));
            assert!(
                expected_revision.get("description").is_none(),
                "{} repeated expected_revision prose in tools/list",
                tool["name"]
            );
        }
    }

    #[test]
    fn marker_tool_exposes_absolute_timeline_coordinate() {
        let state = AppState::new();
        let tool = list_agent_chat_tools(&state)
            .into_iter()
            .find(|tool| tool["name"] == "edit_add_marker")
            .expect("Agent Chat must expose edit.add_marker");
        assert!(tool["description"].as_str().is_some_and(
            |description| description.contains("absolute position on the project timeline")
        ));
        assert!(tool["inputSchema"]["properties"]["at_ms"]["description"]
            .as_str()
            .is_some_and(|description| description
                .contains("Absolute project timeline milliseconds from zero")));
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
        let restricted = crate::chat::capabilities::restricted_mcp_environment(
            Some(crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE),
            Some("agent:test:agent.chat"),
        );
        assert!(restricted, "the broker marker must activate server policy");
        let tools = list_tools_for_mcp_environment(&state, restricted);
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
        let project_open = state
            .registry
            .get("project.open")
            .expect("registered denied verb");
        assert!(
            denied_by_restricted_mcp_policy(project_open, restricted),
            "a guessed blocked tool must be denied at tools/call as well as hidden at tools/list"
        );
        assert!(
            !denied_by_restricted_mcp_policy(project_open, false),
            "an unrestricted user-configured MCP retains the full surface"
        );
        assert!(
            list_tools_for_mcp_environment(&state, false)
                .iter()
                .any(|tool| tool["name"] == "project_open"),
            "the restricted marker must not alter a normal user-configured MCP"
        );
    }

    #[test]
    fn recorder_readiness_tools_are_listed_for_mcp_but_denied_to_agent_chat() {
        let state = AppState::new();
        let tools = list_tools(&state);
        let recovery = tools
            .iter()
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
        let audio = tools
            .iter()
            .find(|tool| tool["name"] == json!("screen_record_system_audio_probe"))
            .expect("generated system-audio probe must be MCP-visible");
        assert_eq!(
            audio["inputSchema"]["properties"]["max_ms"]["maximum"],
            5000
        );
        assert!(
            !list_agent_chat_tools(&state)
                .iter()
                .any(|tool| tool["name"] == json!("screen_record_system_audio_probe")),
            "Agent Chat cannot open an audio stream or permission prompt"
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

    #[test]
    fn summarizer_handles_abbreviations_and_multibyte_boundaries_cleanly() {
        let sample = "List workflows (e.g. podcast-repurpose, screen-demo) across the open project. Detailed notes continue.";
        let summarized = summarize_tool_description(sample, 100);
        assert_eq!(
            summarized,
            "List workflows (e.g. podcast-repurpose, screen-demo) across the open project."
        );
        let utf8 =
            "Trim clip — keep first boundary cleanly at 0–100 ms (e.g. fade, zoom) without panic.";
        assert!(concise(utf8, 45).ends_with('…'));
    }

    #[test]
    fn agent_chat_tools_and_all_summaries_stay_bounded_and_non_empty() {
        let state = AppState::new();
        let chat_reply = rpc_result(json!(3), json!({"tools": list_agent_chat_tools(&state)}));
        let chat_bytes = chat_reply.to_string().len();
        assert!(chat_bytes * 100 < MAX_TOOLS_LIST_BYTES * 40);
        for tool in list_tools(&state) {
            let desc = tool["description"].as_str().unwrap_or("");
            assert!(
                desc.chars().count() >= 20,
                "{} summary unexpectedly short: {desc}",
                tool["name"]
            );
        }
    }
}
