//! Codex CLI argument construction for Agent Chat.

fn toml_str(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Build a normal Codex `exec` turn in Cut's disposable workspace. Automatic
/// approval review lets the headless turn request the restricted Cut MCP edit
/// without silently failing on an interactive approval prompt. Codex still
/// loads the user's configuration, rules, and safeguards; Cut does not bypass
/// them. The turn is ephemeral so Agent Chat does not clutter resumable Codex
/// sessions.
pub(crate) fn args(
    cutd_exe: &str,
    proxy_addr: &str,
    proxy_actor: &str,
    model: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "exec".into(),
        "-".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
        "--ephemeral".into(),
        "--approve-for-me".into(),
        "-c".into(),
        format!("mcp_servers.cutd.command={}", toml_str(cutd_exe)),
        "-c".into(),
        "mcp_servers.cutd.args=[\"mcp\"]".into(),
        "-c".into(),
        format!(
            "mcp_servers.cutd.env={{CUTD_PROXY_ADDR={},CUTD_PROXY_ACTOR={},{}={}}}",
            toml_str(proxy_addr),
            toml_str(proxy_actor),
            crate::chat::capabilities::RESTRICTED_MCP_MARKER,
            toml_str(crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE),
        ),
    ];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".into());
        args.push(model.into());
    }
    args
}
