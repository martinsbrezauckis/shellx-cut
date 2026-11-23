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

#[cfg(unix)]
const TEST_ADDR: &str = "127.0.0.1:6260";

#[cfg(unix)]
fn transcript(state: &AppState, doctor_bytes: usize) -> String {
    let mut tools = vec![json!({"name": "unused"}); state.registry.verbs.len()];
    tools[0] = json!({"name": "system_doctor"});
    tools[1] = json!({"name": "system_mcp_test"});
    let mut list = json!({"jsonrpc": "2.0", "id": 3, "result": {"tools": tools}, "padding": ""});
    // Keep the independent tools/list client budget exactly at its boundary.
    let padding = MAX_TOOLS_LIST_BYTES - list.to_string().len();
    list["padding"] = json!("x".repeat(padding));
    assert_eq!(list.to_string().len(), MAX_TOOLS_LIST_BYTES);
    let mut doctor = json!({"jsonrpc": "2.0", "id": 4, "result": {
        "structuredContent": {"ok": true, "result": {"addr": TEST_ADDR}}, "padding": ""
    }});
    let padding = doctor_bytes - doctor.to_string().len();
    doctor["result"]["padding"] = json!("x".repeat(padding));
    assert_eq!(doctor.to_string().len(), doctor_bytes);
    [
        json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": PROTOCOL_VERSION}}),
        json!({"jsonrpc": "2.0", "id": 2, "result": {}}),
        list,
        doctor,
    ]
    .iter()
    .map(|reply| format!("{reply}\n"))
    .collect()
}

#[cfg(unix)]
async fn fake_process(state: &AppState, stdout: &str) -> Result<VerbResult, CutError> {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("replies.jsonl");
    std::fs::write(&path, stdout).unwrap();
    let mut command = tokio::process::Command::new("sh");
    // Consume the requests before returning the stand-in's four replies.
    command.args(["-c", "cat >/dev/null; cat \"$1\"", "sh"]);
    command.arg(path);
    run_with_command(
        state,
        TEST_ADDR,
        std::path::Path::new("stand-in-mcp"),
        &mut command,
        b"{}\n{}\n{}\n{}\n",
    )
    .await
}

#[cfg(unix)]
#[tokio::test]
async fn self_test_accepts_combined_replies_larger_than_default_capture() {
    let state = AppState::new();
    let stdout = transcript(&state, 200_000);
    assert!(stdout.len() > 524_288);
    let result = fake_process(&state, &stdout).await.unwrap();
    assert!(result.ok);
    let value = result.result.unwrap();
    assert!(value["elapsed_ms"].is_u64());
}

#[cfg(unix)]
#[tokio::test]
async fn self_test_accepts_a_cold_doctor_slower_than_twelve_seconds() {
    let state = AppState::new();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("replies.jsonl");
    std::fs::write(&path, transcript(&state, 200_000)).unwrap();
    let mut command = tokio::process::Command::new("sh");
    command
        .args(["-c", "cat >/dev/null; sleep 13; cat \"$1\"", "sh"])
        .arg(path);
    let result = run_with_command(
        &state,
        TEST_ADDR,
        std::path::Path::new("cold-mcp"),
        &mut command,
        b"{}\n",
    )
    .await
    .unwrap();
    assert!(result.ok);
    assert!(result.result.unwrap()["elapsed_ms"].as_u64().unwrap() >= 13_000);
}

#[cfg(unix)]
#[tokio::test]
async fn self_test_refuses_output_over_its_capture_budget() {
    let state = AppState::new();
    // A single reply also exceeds the self-test's new 4 MiB bound.
    let stdout = transcript(&state, 4 * 1024 * 1024 + 1);
    let error = fake_process(&state, &stdout).await.unwrap_err();
    assert_eq!(
        error.message,
        "the installed MCP process returned too much output"
    );
    assert!(error.cause.contains("4194304-byte limit"));
}

#[cfg(unix)]
#[tokio::test]
async fn self_test_still_rejects_truncated_and_malformed_json() {
    let state = AppState::new();
    let stdout = transcript(&state, 200_000);
    for broken in [&stdout[..stdout.len() - 3], "{not json}\n"] {
        let error = fake_process(&state, broken).await.unwrap_err();
        assert_eq!(
            error.message,
            "the installed MCP process returned invalid JSON-RPC"
        );
    }
}
