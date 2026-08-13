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
        "[mcp_servers.cutd]\ncommand = {}\nargs = [\"mcp\"]\nenabled = true\n\n[mcp_servers.cutd.env]\nCUTD_PROXY_ADDR = {}\nCUTD_PROXY_ACTOR = {}\n{} = {}\n",
        toml_str(cutd_exe),
        toml_str(proxy_addr),
        toml_str(proxy_actor),
        crate::chat::capabilities::RESTRICTED_MCP_MARKER,
        toml_str(crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE),
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
        (
            crate::chat::capabilities::RESTRICTED_MCP_MARKER,
            crate::chat::capabilities::RESTRICTED_MCP_MARKER_VALUE,
        ),
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
#[path = "grok_tests.rs"]
mod tests;
