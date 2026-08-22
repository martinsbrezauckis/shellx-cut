//! Antigravity CLI argument and workspace-MCP construction for Agent Chat.

use super::missing_required_help_tokens;

const REQUIRED_HELP_TOKENS: &[&str] = &[
    "--print",
    "--output-format",
    "--disable-slash-commands",
    "--sandbox",
    "--log-file",
    "--print-timeout",
    "--model",
];

/// Add only the live Cut MCP server to this turn's disposable workspace.
/// Antigravity keeps its normal login, settings, sandbox, and permission policy;
/// Cut neither copies credentials nor rewrites global MCP configuration.
pub(crate) fn project_config(cutd_exe: &str, proxy_addr: &str, proxy_actor: &str) -> String {
    let environment = serde_json::Map::from_iter([
        (
            "CUTD_PROXY_ADDR".into(),
            serde_json::Value::String(proxy_addr.into()),
        ),
        (
            "CUTD_PROXY_ACTOR".into(),
            serde_json::Value::String(proxy_actor.into()),
        ),
        (
            crate::chat::capabilities::RESTRICTED_MCP_MARKER.into(),
            serde_json::Value::String(
                crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE.into(),
            ),
        ),
    ]);
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "cutd": {
                "command": cutd_exe,
                "args": ["mcp"],
                "env": environment,
            }
        }
    }))
    .expect("Antigravity MCP config is serializable")
}

/// Build one non-interactive turn. The prompt must be the final `--print`
/// argument under the current CLI contract, so dispatch substitutes it only
/// after all provider-independent prompt construction is complete.
pub(crate) fn args(workspace: &str, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--sandbox".into(),
        "--disable-slash-commands".into(),
        "--output-format".into(),
        "json".into(),
        "--print-timeout".into(),
        "10m0s".into(),
        "--log-file".into(),
        std::path::Path::new(workspace)
            .join("antigravity.log")
            .to_string_lossy()
            .into_owned(),
    ];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".into());
        args.push(model.into());
    }
    args.push("--print".into());
    args.push("__PROMPT_TEXT__".into());
    args
}

pub(crate) fn verify_capability_contract(help: &str) -> Result<(), String> {
    let missing = missing_required_help_tokens(help, REQUIRED_HELP_TOKENS);
    if !missing.is_empty() {
        return Err(format!(
            "the installed Antigravity CLI does not advertise required native Agent Chat flags: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_uses_native_sandbox_and_prompt_is_last() {
        let args = args("/tmp/cut-chat", Some("Gemini 3.5 Flash"));
        assert!(args.contains(&"--sandbox".into()));
        assert!(args.contains(&"--disable-slash-commands".into()));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--output-format", "json"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--model", "Gemini 3.5 Flash"]));
        assert_eq!(&args[args.len() - 2..], ["--print", "__PROMPT_TEXT__"]);
        assert!(!args.contains(&"--dangerously-skip-permissions".into()));
    }

    #[test]
    fn workspace_config_contains_only_the_live_cut_mcp() {
        let config = project_config(
            "C:\\Program Files\\ShellX Cut\\cutd.exe",
            "127.0.0.1:6161",
            "agent:test:agent.chat",
        );
        let value: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(value["mcpServers"].as_object().unwrap().len(), 1);
        assert_eq!(
            value["mcpServers"]["cutd"]["command"],
            "C:\\Program Files\\ShellX Cut\\cutd.exe"
        );
        assert_eq!(
            value["mcpServers"]["cutd"]["env"]["CUTD_PROXY_ACTOR"],
            "agent:test:agent.chat"
        );
    }

    #[test]
    fn capability_contract_is_help_based_and_rejects_drift() {
        let help = REQUIRED_HELP_TOKENS.join(" ");
        for version in ["", "Antigravity unknown", "current build"] {
            assert!(
                verify_capability_contract(&help).is_ok(),
                "version text must not gate a capable Antigravity CLI: {version:?}"
            );
        }
        for missing in REQUIRED_HELP_TOKENS {
            let incomplete = REQUIRED_HELP_TOKENS
                .iter()
                .copied()
                .filter(|token| token != missing)
                .collect::<Vec<_>>()
                .join(" ");
            let error = verify_capability_contract(&incomplete)
                .expect_err("each native Agent Chat flag must be mandatory");
            assert!(
                error.contains(missing),
                "missing flag must be named: {missing}"
            );
        }
        assert!(
            verify_capability_contract(&help.replacen("--model", "--model-v2", 1))
                .unwrap_err()
                .contains("--model")
        );
    }
}
