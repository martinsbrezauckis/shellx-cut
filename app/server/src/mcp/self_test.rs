//! Read-only installed MCP proxy self-test used by Settings > Agent control.

use super::MAX_TOOLS_LIST_BYTES;
use crate::state::AppState;
use cut_core::{error_codes, CutError, VerbResult};
use serde_json::{json, Value};
use std::time::Duration;

const PROTOCOL_VERSION: &str = "2025-06-18";
/// Exercise the exact current executable's MCP stdio proxy against this served
/// engine. Matching the doctor report's bound address proves that the child
/// reached this state holder rather than opening an independent runtime.
pub(crate) async fn run(state: &AppState) -> Result<VerbResult, CutError> {
    let addr = state.addr.read().await.clone().ok_or_else(|| {
        test_error(
            "MCP proxy self-test requires a running Cut app or `cutd serve`",
            "this dispatch surface has no served engine address",
        )
    })?;
    let executable = std::env::current_exe().map_err(|error| {
        test_error(
            "could not resolve this Cut engine executable",
            &error.to_string(),
        )
    })?;
    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "shellx-cut-settings", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
        json!({"jsonrpc": "2.0", "id": 2, "method": "ping", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {}}),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "system_doctor", "arguments": {}}
        }),
    ];
    let mut input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    input.push('\n');

    let mut command = tokio::process::Command::new(&executable);
    command
        .arg("mcp")
        .env("CUTD_PROXY_ADDR", &addr)
        .env("CUTD_PROXY_ACTOR", "agent:settings:mcp-self-test");
    let output = crate::jobs::run_owned(
        &mut command,
        Some(input.as_bytes()),
        &crate::jobs::ProcessControl::for_operation(Duration::from_secs(12)),
    )
    .await
    .map_err(|error| match error.termination() {
        Some(crate::jobs::ProcessTermination::DeadlineExceeded) => test_error(
            "MCP self-test timed out",
            "the installed MCP stdio process did not complete within 12 seconds",
        ),
        _ => test_error("MCP self-test could not complete", &error.to_string()),
    })?;
    if !output.status.success() {
        return Err(test_error(
            "the installed MCP process exited with an error",
            &bounded_stderr(&output.stderr),
        ));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        test_error(
            "the installed MCP process returned non-UTF-8 output",
            &error.to_string(),
        )
    })?;
    let messages = stdout
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).map_err(|error| {
                test_error(
                    "the installed MCP process returned invalid JSON-RPC",
                    &format!("{error}: {}", line.chars().take(200).collect::<String>()),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let reply = |id: u64| {
        messages
            .iter()
            .find(|message| message.get("id").and_then(Value::as_u64) == Some(id))
            .ok_or_else(|| {
                test_error(
                    "the installed MCP process omitted a self-test reply",
                    &format!("missing JSON-RPC response id {id}"),
                )
            })
    };

    let initialized = reply(1)?;
    let protocol_version = initialized
        .pointer("/result/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if protocol_version != PROTOCOL_VERSION {
        return Err(test_error(
            "MCP protocol negotiation did not echo the requested version",
            &format!("requested {PROTOCOL_VERSION}, received {protocol_version:?}"),
        ));
    }
    if reply(2)?.get("result") != Some(&json!({})) {
        return Err(test_error(
            "MCP ping returned an unexpected response",
            &reply(2)?.to_string(),
        ));
    }

    let tools_reply = reply(3)?;
    let tools = tools_reply
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            test_error(
                "MCP tools/list returned no tool array",
                &tools_reply.to_string(),
            )
        })?;
    let expected_tools = state.registry.verbs.len();
    if tools.len() != expected_tools
        || !tools
            .iter()
            .any(|tool| tool.get("name") == Some(&json!("system_doctor")))
        || !tools
            .iter()
            .any(|tool| tool.get("name") == Some(&json!("system_mcp_test")))
    {
        return Err(test_error(
            "MCP tools/list does not match the live verb registry",
            &format!("expected {expected_tools} tools, received {}", tools.len()),
        ));
    }
    let tools_list_bytes = tools_reply.to_string().len();
    if tools_list_bytes > MAX_TOOLS_LIST_BYTES {
        return Err(test_error(
            "MCP tools/list is too large for the supported client budget",
            &format!("{tools_list_bytes} bytes exceeds the {MAX_TOOLS_LIST_BYTES}-byte limit"),
        ));
    }

    let call_reply = reply(4)?;
    let doctor_envelope = call_reply
        .pointer("/result/structuredContent")
        .ok_or_else(|| {
            test_error(
                "MCP tools/call returned no structured verb envelope",
                &call_reply.to_string(),
            )
        })?;
    if doctor_envelope.get("ok") != Some(&json!(true)) {
        return Err(test_error(
            "MCP could not read the running Cut engine",
            &doctor_envelope.to_string(),
        ));
    }
    let proxied_addr = doctor_envelope
        .pointer("/result/addr")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if proxied_addr != addr {
        return Err(test_error(
            "MCP resolved a different Cut engine",
            &format!("expected {addr}, received {proxied_addr:?}"),
        ));
    }

    Ok(VerbResult::ok(json!({
        "schema": "shellx-cut/mcp-self-test/1",
        "mode": "proxy",
        "read_only": true,
        "executable": executable,
        "command": [executable, "mcp"],
        "protocol_version": protocol_version,
        "ping": true,
        "tools": tools.len(),
        "expected_tools": expected_tools,
        "tools_list_bytes": tools_list_bytes,
        "tools_list_max_bytes": MAX_TOOLS_LIST_BYTES,
        "proxy_addr": proxied_addr,
        "same_engine": true
    })))
}

fn bounded_stderr(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .take(1_000)
        .collect::<String>()
}

fn test_error(message: &str, cause: &str) -> CutError {
    CutError::new(error_codes::JOB_FAILED, message, cause)
        .with_suggested_action("keep ShellX Cut open, then retry from Settings > Agent control")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn self_test_refuses_a_surface_without_a_served_engine() {
        let error = run(&AppState::new()).await.unwrap_err();
        assert_eq!(error.code, error_codes::JOB_FAILED);
        assert!(error.message.contains("running Cut app"));
        assert!(error
            .suggested_action
            .unwrap_or_default()
            .contains("Agent control"));
    }
}
