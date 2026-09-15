use super::*;

#[test]
fn command_contract_removes_native_tools_and_allows_only_cut_mcp() {
    let args = args("/tmp/cut-chat", Some("grok-code-fast-1"));
    for pair in [
        ["--tools", ""],
        ["--allow", "MCPTool(cutd__*)"],
        ["--permission-mode", "dontAsk"],
        ["--model", "grok-code-fast-1"],
    ] {
        assert!(args.windows(2).any(|window| window == pair));
    }
    for flag in [
        "--trust",
        "--disable-web-search",
        "--no-subagents",
        "--no-plan",
        "--verbatim",
    ] {
        assert!(args.contains(&flag.to_string()));
    }
    assert!(!args.contains(&"--always-approve".to_string()));
    assert!(!args.contains(&"--dangerously-skip-permissions".to_string()));
    assert!(!args.contains(&"--deny".to_string()));
    assert!(!args.contains(&"--no-memory".to_string()));
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
    assert_eq!(
        vars.get(&OsString::from("HOME")),
        Some(&workspace.path().join("os-home").into_os_string())
    );
    assert_eq!(
        vars.get(&OsString::from("GROK_HOME")),
        Some(&workspace.path().join("grok-home").into_os_string())
    );
}

#[test]
fn provider_context_grok_policy_derives_login_from_admitted_home() {
    let workspace = tempfile::Builder::new()
        .prefix("provider-context-grok-")
        .tempdir_in(crate::provider_runtime::test_fixture_root())
        .unwrap();
    let environment = isolated_environment_for_provider(
        Some(&BTreeMap::from([
            ("PATH".into(), "/runner/bin".into()),
            ("HOME".into(), "/runner/canonical-home".into()),
        ])),
        workspace.path(),
        "127.0.0.1:6161",
        "agent:test:agent.chat",
    )
    .unwrap();
    let vars: BTreeMap<_, _> = environment.vars.into_iter().collect();
    assert_eq!(
        vars.get(&OsString::from("GROK_AUTH_PATH")),
        Some(
            &std::path::Path::new("/runner/canonical-home")
                .join(".grok")
                .join("auth.json")
                .into_os_string()
        )
    );
    assert_eq!(
        vars.get(&OsString::from("HOME")),
        Some(&workspace.path().join("os-home").into_os_string())
    );
}

#[test]
fn capability_contract_is_flag_based_and_fails_closed() {
    let help = REQUIRED_HELP_TOKENS.join(" ");
    for version in ["", "other CLI", "current Grok build"] {
        assert!(
            verify_capability_contract(&help).is_ok(),
            "version text must not gate a capable Grok CLI: {version:?}"
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
            .expect_err("each isolated Agent Chat flag must be mandatory");
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
