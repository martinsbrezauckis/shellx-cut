//! Grok Build CLI argument and project-MCP construction for Agent Chat.

use super::LaunchEnvironment;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

const REQUIRED_HELP_TOKENS: &[&str] = &[
    "--prompt-file",
    "--output-format",
    "--cwd",
    "--permission-mode",
    "--tools",
    "--allow",
    "--deny",
    "--disable-web-search",
    "--no-memory",
    "--no-subagents",
    "--no-plan",
    "--verbatim",
    "--leader-socket",
    "--model",
];

fn named(entries: &[(OsString, OsString)], wanted: &str) -> Option<OsString> {
    entries.iter().find_map(|(name, value)| {
        name.to_string_lossy()
            .eq_ignore_ascii_case(wanted)
            .then(|| value.clone())
    })
}

fn toml_str(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// One disposable project config. The isolated launch environment points
/// `GROK_HOME` away from the user's normal configuration while
/// `GROK_AUTH_PATH` keeps the existing login file in its original location.
pub(crate) fn project_config(cutd_exe: &str, proxy_addr: &str, proxy_actor: &str) -> String {
    format!(
        "[mcp_servers.cutd]\ncommand = {}\nargs = [\"mcp\"]\nenabled = true\n\n[mcp_servers.cutd.env]\nCUTD_PROXY_ADDR = {}\nCUTD_PROXY_ACTOR = {}\n",
        toml_str(cutd_exe),
        toml_str(proxy_addr),
        toml_str(proxy_actor),
    )
}

/// Build a single, non-persistent Grok turn. Built-in tools are removed, every
/// native permission family is denied defensively, and only the disposable
/// project's `cutd` MCP server is allowed.
pub(crate) fn args(workspace: &str, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--prompt-file".into(),
        "__PROMPT_FILE__".into(),
        "--output-format".into(),
        "json".into(),
        "--cwd".into(),
        workspace.into(),
        "--permission-mode".into(),
        "dontAsk".into(),
        "--tools".into(),
        String::new(),
        "--allow".into(),
        "mcp(cutd/*)".into(),
        "--deny".into(),
        "read_file(*)".into(),
        "--deny".into(),
        "write_file(*)".into(),
        "--deny".into(),
        "command(*)".into(),
        "--deny".into(),
        "read_url(*)".into(),
        "--deny".into(),
        "execute_url(*)".into(),
        "--deny".into(),
        "unsandboxed(*)".into(),
        "--disable-web-search".into(),
        "--no-memory".into(),
        "--no-subagents".into(),
        "--no-plan".into(),
        "--verbatim".into(),
        "--leader-socket".into(),
        std::path::Path::new(workspace)
            .join("grok-leader.sock")
            .to_string_lossy()
            .into_owned(),
    ];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".into());
        args.push(model.into());
    }
    args
}

/// Give Grok a disposable OS/config home while pointing only its explicit auth
/// path at the user's existing login file. The source auth file stays in place.
pub(crate) fn isolated_environment_from<I>(
    inherited: I,
    workspace: &Path,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let entries: Vec<_> = inherited.into_iter().collect();
    let path = named(&entries, "PATH")
        .ok_or_else(|| "cannot launch isolated Grok: inherited PATH is unavailable".to_string())?;
    let source_home = named(&entries, "HOME")
        .or_else(|| named(&entries, "USERPROFILE"))
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            "cannot launch isolated Grok: HOME or USERPROFILE is unavailable for its existing login"
                .to_string()
        })?;
    let auth_path = named(&entries, "GROK_AUTH_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| source_home.join(".grok").join("auth.json"));
    let os_home = workspace.join("os-home");
    let grok_home = workspace.join("grok-home");
    let temp = workspace.join("tmp");
    for dir in [&os_home, &grok_home, &temp] {
        std::fs::create_dir_all(dir).map_err(|error| {
            format!(
                "could not create isolated Grok runtime directory {}: {error}",
                dir.display()
            )
        })?;
    }

    let mut vars = BTreeMap::new();
    vars.insert(OsString::from("PATH"), path);
    for name in [
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = named(&entries, name) {
            vars.insert(OsString::from(name), value);
        }
    }
    for name in ["HOME", "USERPROFILE"] {
        vars.insert(OsString::from(name), os_home.as_os_str().to_owned());
    }
    for name in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        vars.insert(
            OsString::from(name),
            os_home.join(name.to_ascii_lowercase()).into_os_string(),
        );
    }
    vars.insert(
        OsString::from("GROK_HOME"),
        grok_home.as_os_str().to_owned(),
    );
    vars.insert(OsString::from("GROK_AUTH_PATH"), auth_path.into_os_string());
    vars.insert(
        OsString::from("GROK_LEADER_SOCKET"),
        workspace.join("grok-leader.sock").into_os_string(),
    );
    vars.insert(
        OsString::from("GROK_LOG_FILE"),
        workspace.join("grok.log").into_os_string(),
    );
    for name in ["TMPDIR", "TMP", "TEMP"] {
        vars.insert(OsString::from(name), temp.as_os_str().to_owned());
    }
    for name in [
        "GROK_CLAUDE_SKILLS_ENABLED",
        "GROK_CLAUDE_RULES_ENABLED",
        "GROK_CLAUDE_AGENTS_ENABLED",
        "GROK_CLAUDE_MCPS_ENABLED",
        "GROK_CLAUDE_HOOKS_ENABLED",
        "GROK_CLAUDE_SESSIONS_ENABLED",
        "GROK_CODEX_SKILLS_ENABLED",
        "GROK_CODEX_RULES_ENABLED",
        "GROK_CODEX_AGENTS_ENABLED",
        "GROK_CODEX_MCPS_ENABLED",
        "GROK_CODEX_HOOKS_ENABLED",
        "GROK_CODEX_SESSIONS_ENABLED",
        "GROK_CURSOR_SKILLS_ENABLED",
        "GROK_CURSOR_RULES_ENABLED",
        "GROK_CURSOR_AGENTS_ENABLED",
        "GROK_CURSOR_MCPS_ENABLED",
        "GROK_CURSOR_HOOKS_ENABLED",
        "GROK_CURSOR_SESSIONS_ENABLED",
        "GROK_OFFICIAL_MARKETPLACE_AUTO_REGISTER",
        "GROK_MANAGED_MCPS_ENABLED",
        "GROK_MANAGED_MCP_GATEWAY_TOOLS_ENABLED",
        "GROK_TELEMETRY_ENABLED",
    ] {
        vars.insert(OsString::from(name), OsString::from("false"));
    }
    for (name, value) in [
        ("CUTD_PROXY_ADDR", proxy_addr),
        ("CUTD_PROXY_ACTOR", proxy_actor),
        ("SHELLX_CUT_AGENT_CONTAINED", "1"),
    ] {
        vars.insert(OsString::from(name), OsString::from(value));
    }
    Ok(LaunchEnvironment {
        clear_inherited: true,
        vars: vars.into_iter().collect(),
    })
}

pub(crate) fn isolated_environment(
    workspace: &Path,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String> {
    isolated_environment_from(std::env::vars_os(), workspace, proxy_addr, proxy_actor)
}

pub(crate) fn verify_capability_contract(version: &str, help: &str) -> Result<(), String> {
    if !version.to_ascii_lowercase().contains("grok") {
        return Err(format!(
            "the resolved Grok executable returned an unexpected version string: {:?}",
            version.trim()
        ));
    }
    let missing: Vec<&str> = REQUIRED_HELP_TOKENS
        .iter()
        .copied()
        .filter(|token| !help.contains(token))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the installed Grok CLI does not advertise required isolated Agent Chat flags: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        assert_eq!(
            vars.get(&OsString::from("GROK_AUTH_PATH")),
            Some(&OsString::from("/home/editor/.grok/auth.json"))
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
}
