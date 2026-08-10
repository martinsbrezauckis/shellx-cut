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
    for required in ["PATH", "HOME", "CUTD_PROXY_ADDR", "CUTD_PROXY_ACTOR"] {
        assert!(names.iter().any(|name| name.eq_ignore_ascii_case(required)));
    }
}

#[test]
fn native_codex_environment_keeps_inheritance_and_adds_only_cut_routing() {
    let env = native_environment("127.0.0.1:6161", "agent:test:agent.chat");
    assert!(!env.clear_inherited);
    assert_eq!(env.names(), ["CUTD_PROXY_ADDR", "CUTD_PROXY_ACTOR"]);
}

#[test]
fn exact_version_and_help_contract_are_required() {
    let help = REQUIRED_HELP_TOKENS.join(" ");
    assert!(verify_claude_capability_contract("2.1.224 (Claude Code)", &help).is_ok());
    assert!(is_supported_claude_version("2.1.224 (Claude Code)"));
    assert!(!is_supported_claude_version("2.1.225 (Claude Code)"));
    assert!(verify_claude_capability_contract("2.1.225 (Claude Code)", &help).is_err());
    for missing in REQUIRED_HELP_TOKENS {
        let incomplete = REQUIRED_HELP_TOKENS
            .iter()
            .copied()
            .filter(|token| token != missing)
            .collect::<Vec<_>>()
            .join(" ");
        let error = verify_claude_capability_contract("2.1.224", &incomplete)
            .expect_err("each containment flag must be mandatory");
        assert!(
            error.contains(missing),
            "missing flag must be named: {missing}"
        );
    }
}

#[test]
fn codex_capability_contract_is_flag_based_not_version_pinned() {
    let help = REQUIRED_CODEX_EXEC_HELP_TOKENS.join(" ");
    assert!(verify_codex_capability_contract("codex-cli 0.147.0", &help).is_ok());
    assert!(verify_codex_capability_contract("codex-cli 0.148.0", &help).is_ok());
    assert!(verify_codex_capability_contract("other-cli 1.0.0", &help).is_err());
    for missing in REQUIRED_CODEX_EXEC_HELP_TOKENS {
        let incomplete = REQUIRED_CODEX_EXEC_HELP_TOKENS
            .iter()
            .copied()
            .filter(|token| token != missing)
            .collect::<Vec<_>>()
            .join(" ");
        let error = verify_codex_capability_contract("codex-cli 0.147.0", &incomplete)
            .expect_err("each required launch flag must be advertised");
        assert!(error.contains(missing));
    }
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
    assert_eq!(supported_headless_agent("antigravity"), !cfg!(windows));
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
    ]));
    assert!(args
        .iter()
        .any(|arg| arg.contains("mcp_servers.cutd.command=\"C:\\\\Program Files")));
    assert!(args.iter().any(|arg| {
        arg.contains("CUTD_PROXY_ADDR=\"127.0.0.1:6161\"")
            && arg.contains("CUTD_PROXY_ACTOR=\"agent:turn:agent.chat\"")
    }));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--model", "gpt-5.6-codex"]));
    for forbidden in [
        "danger-full-access",
        "--ignore-user-config",
        "--ignore-rules",
        "approval_policy=\"never\"",
    ] {
        assert!(!args.iter().any(|arg| arg == forbidden));
    }
}
