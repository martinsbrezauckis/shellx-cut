use super::*;

#[test]
fn claude_policy_allows_only_cutd_mcp_and_denies_native_tools() {
    let args = claude_args("/tmp/mcp.json", Some("sonnet"));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--allowedTools", "mcp__cutd__*"]));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--setting-sources", ""]));
    assert!(args.contains(&"--disable-slash-commands".to_string()));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--permission-mode", "dontAsk"]));
    let denied = args
        .windows(2)
        .find_map(|pair| (pair[0] == "--disallowedTools").then_some(&pair[1]))
        .unwrap();
    for tool in [
        "Read",
        "Write",
        "Edit",
        "Bash",
        "WebFetch",
        "WebSearch",
        "mcp__cutd__agent_chat",
    ] {
        assert!(denied.contains(tool), "missing explicit denial for {tool}");
    }
    assert!(args.contains(&"--strict-mcp-config".to_string()));
    assert!(!args.contains(&"--tools".to_string()));
    assert!(!args.contains(&"--safe-mode".to_string()));
}

#[test]
fn sanitized_environment_drops_hostile_parent_values() {
    let env = sanitized_environment_from(
        [
            (OsString::from("PATH"), OsString::from("/usr/bin")),
            (OsString::from("HOME"), OsString::from("/safe/home")),
            (
                OsString::from("HTTP_PROXY"),
                OsString::from("http://hostile"),
            ),
            (
                OsString::from("AWS_SECRET_ACCESS_KEY"),
                OsString::from("secret"),
            ),
            (
                OsString::from("CUT_AGENT_SENTINEL"),
                OsString::from("hostile"),
            ),
        ],
        "127.0.0.1:6161",
        "agent:test:agent.chat",
    )
    .unwrap();
    let names = env.names();
    for forbidden in ["HTTP_PROXY", "AWS_SECRET_ACCESS_KEY", "CUT_AGENT_SENTINEL"] {
        assert!(!names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(forbidden)));
    }
    for required in [
        "PATH",
        "HOME",
        "CUTD_PROXY_ADDR",
        "CUTD_PROXY_ACTOR",
        crate::chat::capabilities::RESTRICTED_MCP_MARKER,
    ] {
        assert!(names.iter().any(|name| name.eq_ignore_ascii_case(required)));
    }
}

#[cfg(windows)]
#[test]
fn sanitized_environment_preserves_windows_runtime_root() {
    let env = sanitized_environment_from(
        [
            (
                OsString::from("Path"),
                OsString::from(r"C:\Windows\System32"),
            ),
            (
                OsString::from("USERPROFILE"),
                OsString::from(r"C:\Users\fixture"),
            ),
            (OsString::from("SystemRoot"), OsString::from(r"C:\Windows")),
        ],
        "127.0.0.1:6161",
        "agent:test:agent.chat",
    )
    .unwrap();
    let names = env.names();
    assert!(names
        .iter()
        .any(|name| name.eq_ignore_ascii_case("SystemRoot")));
}

#[test]
fn native_environment_keeps_inheritance_and_adds_restricted_cut_routing() {
    let env = native_environment("127.0.0.1:6161", "agent:test:agent.chat");
    assert!(!env.clear_inherited);
    assert_eq!(
        env.names(),
        [
            "CUTD_PROXY_ADDR",
            "CUTD_PROXY_ACTOR",
            crate::chat::capabilities::RESTRICTED_MCP_MARKER,
        ]
    );
}

#[test]
fn contained_claude_contract_uses_capabilities_not_version_text() {
    let help = REQUIRED_CONTAINED_CLAUDE_HELP_TOKENS.join(" ");
    for version in [
        "",
        "Claude Code",
        "current",
        "2.1.223 (Claude Code)",
        "2.1.224 (Claude Code)",
        "2.1.236 (Claude Code)",
        "3.0.0 (Claude Code)",
        "99.99.99 (Claude Code)",
        "3.0.0-beta.1+build.4 (Claude Code)",
    ] {
        assert!(
            verify_claude_capability_contract(&help).is_ok(),
            "version number alone must not reject a capable Claude CLI: {version}"
        );
    }
    for missing in REQUIRED_CONTAINED_CLAUDE_HELP_TOKENS {
        let incomplete = REQUIRED_CONTAINED_CLAUDE_HELP_TOKENS
            .iter()
            .copied()
            .filter(|token| token != missing)
            .collect::<Vec<_>>()
            .join(" ");
        let error = verify_claude_capability_contract(&incomplete)
            .expect_err("each containment flag must be mandatory");
        assert!(
            error.contains(missing),
            "missing flag must be named: {missing}"
        );
    }
    let renamed_flag = help.replacen("--model", "--model-v2", 1);
    let error = verify_claude_capability_contract(&renamed_flag)
        .expect_err("a longer, similarly named flag must not satisfy containment");
    assert!(error.contains("--model"));
}

#[test]
fn codex_capability_contract_is_flag_based() {
    let help = REQUIRED_CODEX_EXEC_HELP_TOKENS.join(" ");
    for version in ["", "other-cli 1.0.0", "current build"] {
        assert!(
            verify_codex_capability_contract(&help).is_ok(),
            "version text must not gate a capable Codex CLI: {version:?}"
        );
    }
    for missing in REQUIRED_CODEX_EXEC_HELP_TOKENS {
        let incomplete = REQUIRED_CODEX_EXEC_HELP_TOKENS
            .iter()
            .copied()
            .filter(|token| token != missing)
            .collect::<Vec<_>>()
            .join(" ");
        let error = verify_codex_capability_contract(&incomplete)
            .expect_err("each required launch flag must be advertised");
        assert!(error.contains(missing));
    }
    let error = verify_codex_capability_contract(&help.replacen("--model", "--model-v2", 1))
        .expect_err("a longer, similarly named flag must not satisfy Codex containment");
    assert!(error.contains("--model"));
}

#[test]
fn grok_capability_probe_parses_the_complete_contained_argv_without_a_turn() {
    let workspace = std::path::Path::new("C:\\CutProbe");
    let arguments = capability_probe_args("grok", workspace).unwrap();
    for required in [
        "--prompt-file",
        "--no-subagents",
        "--no-plan",
        "--verbatim",
        "--version",
    ] {
        assert!(
            arguments.contains(&required.to_string()),
            "missing {required}"
        );
    }
    assert!(!arguments.contains(&"--no-memory".to_string()));
    assert!(!arguments.contains(&"--deny".to_string()));
    assert_eq!(arguments.last().map(String::as_str), Some("--version"));
    assert!(verify_agent_capability_probe("grok", "Grok fixture-current").is_ok());
    assert!(verify_agent_capability_probe("grok", "another provider").is_err());
}

#[test]
fn workspace_starts_empty_and_supported_providers_are_explicit() {
    let workspace = IsolatedWorkspace::create().unwrap();
    assert!(std::fs::read_dir(workspace.path())
        .unwrap()
        .next()
        .is_none());
    assert!(supported_headless_agent("claude"));
    assert!(supported_headless_agent("codex"));
    assert!(supported_headless_agent("grok"));
    assert!(supported_headless_agent("antigravity"));
}

#[test]
fn codex_args_keep_native_policy_and_add_cut_mcp() {
    let args = codex_args(
        "C:\\Program Files\\ShellX Cut\\cutd.exe",
        "127.0.0.1:6161",
        "agent:turn:agent.chat",
        Some("gpt-5.6-codex"),
    );
    assert!(args.starts_with(&[
        "exec".into(),
        "-".into(),
        "--json".into(),
        "--skip-git-repo-check".into(),
        "--ephemeral".into(),
        "--approve-for-me".into(),
    ]));
    assert!(args
        .iter()
        .any(|arg| arg.contains("mcp_servers.cutd.command=\"C:\\\\Program Files")));
    assert!(args.iter().any(|arg| {
        arg.contains("CUTD_PROXY_ADDR=\"127.0.0.1:6161\"")
            && arg.contains("CUTD_PROXY_ACTOR=\"agent:turn:agent.chat\"")
            && arg.contains("SHELLX_CUT_AGENT_CONTAINED=\"1\"")
    }));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--model", "gpt-5.6-codex"]));
    assert!(args.contains(&"--approve-for-me".to_string()));
    for forbidden in [
        "danger-full-access",
        "--ignore-user-config",
        "--ignore-rules",
        "approval_policy=\"never\"",
    ] {
        assert!(!args.iter().any(|arg| arg == forbidden));
    }
}

#[test]
fn every_agent_chat_provider_forwards_the_restricted_mcp_marker() {
    let marker = crate::chat::capabilities::RESTRICTED_MCP_MARKER;
    let value = crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE;
    let claude = sanitized_environment_from(
        [
            (OsString::from("PATH"), OsString::from("/usr/bin")),
            (OsString::from("HOME"), OsString::from("/safe/home")),
        ],
        "127.0.0.1:6161",
        "agent:turn:agent.chat",
    )
    .unwrap();
    assert!(claude.names().iter().any(|name| name == marker));
    assert!(
        codex_args("cutd", "127.0.0.1:6161", "agent:turn:agent.chat", None)
            .iter()
            .any(|arg| arg.contains(marker) && arg.contains("\"1\""))
    );
    assert!(
        grok_project_config("cutd", "127.0.0.1:6161", "agent:turn:agent.chat")
            .contains(&format!("{marker} = \"{value}\""))
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&antigravity_project_config(
            "cutd",
            "127.0.0.1:6161",
            "agent:turn:agent.chat",
        ))
        .unwrap()["mcpServers"]["cutd"]["env"][marker],
        value
    );
}
