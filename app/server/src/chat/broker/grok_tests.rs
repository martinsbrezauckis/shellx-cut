use super::*;

#[test]
fn command_contract_removes_native_tools_and_allows_only_cut_mcp() {
    let args = args("/tmp/cut-chat", Some("grok-code-fast-1"));
    for pair in [
        ["--tools", ""],
        ["--allow", "mcp(cutd/*)"],
        ["--permission-mode", "dontAsk"],
        ["--model", "grok-code-fast-1"],
    ] {
        assert!(args.windows(2).any(|window| window == pair));
    }
    for permission in [
        "read_file(*)",
        "write_file(*)",
        "command(*)",
        "read_url(*)",
        "execute_url(*)",
        "unsandboxed(*)",
    ] {
        assert!(args
            .windows(2)
            .any(|window| window == ["--deny", permission]));
    }
    for flag in [
        "--disable-web-search",
        "--no-memory",
        "--no-subagents",
        "--no-plan",
        "--verbatim",
    ] {
        assert!(args.contains(&flag.to_string()));
    }
    assert!(!args.contains(&"--always-approve".to_string()));
    assert!(!args.contains(&"--dangerously-skip-permissions".to_string()));
}

#[test]
fn project_config_contains_only_the_live_cut_mcp() {
    let config = project_config(
        "C:\\Program Files\\ShellX Cut\\cutd.exe",
        "127.0.0.1:6161",
        "agent:test:agent.chat",
    );
    assert_eq!(config.matches("[mcp_servers.").count(), 2);
    assert!(config.contains("[mcp_servers.cutd]"));
    assert!(config.contains("[mcp_servers.cutd.env]"));
    assert!(config.contains("C:\\\\Program Files\\\\ShellX Cut\\\\cutd.exe"));
    assert!(config.contains("CUTD_PROXY_ACTOR = \"agent:test:agent.chat\""));
}

#[test]
fn isolated_environment_keeps_auth_in_place_and_drops_parent_state() {
    let workspace = tempfile::tempdir().unwrap();
    let environment = isolated_environment_from(
        [
            (OsString::from("PATH"), OsString::from("/usr/bin")),
            (OsString::from("HOME"), OsString::from("/home/editor")),
            (
                OsString::from("ANTHROPIC_API_KEY"),
                OsString::from("secret"),
            ),
            (
                OsString::from("CUT_AGENT_SENTINEL"),
                OsString::from("hostile"),
            ),
        ],
        workspace.path(),
        "127.0.0.1:6161",
        "agent:test:agent.chat",
    )
    .unwrap();
    assert!(environment.clear_inherited);
    let vars: BTreeMap<_, _> = environment.vars.into_iter().collect();
    let expected_auth_path = std::path::Path::new("/home/editor")
        .join(".grok")
        .join("auth.json")
        .into_os_string();
    assert_eq!(
        vars.get(&OsString::from("GROK_AUTH_PATH")),
        Some(&expected_auth_path)
    );
    assert_eq!(
        vars.get(&OsString::from("GROK_CLAUDE_SKILLS_ENABLED")),
        Some(&OsString::from("false"))
    );
    assert!(!vars.contains_key(&OsString::from("ANTHROPIC_API_KEY")));
    assert!(!vars.contains_key(&OsString::from("CUT_AGENT_SENTINEL")));
    assert!(workspace.path().join("os-home").is_dir());
    assert!(workspace.path().join("grok-home").is_dir());
}

#[test]
fn capability_contract_is_flag_based_and_fails_closed() {
    let help = REQUIRED_HELP_TOKENS.join(" ");
    assert!(verify_capability_contract("grok 1.0.0", &help).is_ok());
    assert!(verify_capability_contract("other 1.0.0", &help).is_err());
    let incomplete = help.replace("--deny", "");
    assert!(verify_capability_contract("grok 1.0.0", &incomplete)
        .unwrap_err()
        .contains("--deny"));
}
