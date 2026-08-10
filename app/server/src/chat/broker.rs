//! Launch policies for the local subscription CLIs used by `agent.chat`.
//!
//! Claude uses Cut's pinned, contained contract. Codex deliberately keeps its
//! normal user configuration and native sandbox/permission policy; Cut adds its
//! own MCP server without redefining the user's machine permissions.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

#[path = "broker/antigravity.rs"]
mod antigravity;
pub(crate) use antigravity::{
    args as antigravity_args, project_config as antigravity_project_config,
    supported_on_this_platform as antigravity_supported_on_this_platform,
    verify_capability_contract as verify_antigravity_capability_contract,
};
#[path = "broker/codex.rs"]
mod codex;
pub(crate) use codex::args as codex_args;
#[path = "broker/grok.rs"]
mod grok;
pub(crate) use grok::{
    args as grok_args, isolated_environment as isolated_grok_environment,
    project_config as grok_project_config,
    verify_capability_contract as verify_grok_capability_contract,
};
#[path = "broker/verify.rs"]
mod verify;
pub const SUPPORTED_CLAUDE_VERSION: &str = "2.1.224";

const REQUIRED_HELP_TOKENS: &[&str] = &[
    "--print",
    "--output-format",
    "--mcp-config",
    "--setting-sources",
    "--disable-slash-commands",
    "--allowedTools",
    "--strict-mcp-config",
    "--disallowedTools",
    "--permission-mode",
    "--no-session-persistence",
    "--model",
];

const REQUIRED_CODEX_EXEC_HELP_TOKENS: &[&str] = &[
    "--config",
    "--json",
    "--skip-git-repo-check",
    "--ephemeral",
    "--model",
];

const NATIVE_TOOL_DENIES: &str =
    "Read,Write,Edit,NotebookEdit,Bash,BashOutput,KillShell,Task,WebFetch,WebSearch,Skill,mcp__cutd__agent_chat";

/// A private, empty, disposable current directory for one agent turn.
pub struct IsolatedWorkspace(tempfile::TempDir);

impl IsolatedWorkspace {
    pub fn create() -> Result<Self, String> {
        tempfile::Builder::new()
            .prefix("cutd-agent-")
            .tempdir()
            .map(Self)
            .map_err(|error| format!("could not create isolated Agent Chat workspace: {error}"))
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

/// Environment policy for one local-agent launch.
#[derive(Clone, Debug)]
pub struct LaunchEnvironment {
    clear_inherited: bool,
    vars: Vec<(OsString, OsString)>,
}

impl LaunchEnvironment {
    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.vars
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect()
    }

    pub fn apply(&self, command: &mut tokio::process::Command) {
        if self.clear_inherited {
            command.env_clear();
        }
        command.envs(self.vars.iter().cloned());
    }
}

fn named<I>(environment: I, wanted: &str) -> Option<OsString>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    environment.into_iter().find_map(|(name, value)| {
        name.to_string_lossy()
            .eq_ignore_ascii_case(wanted)
            .then_some(value)
    })
}

/// Retain only runtime/auth routing essentials; remove credentials, proxies,
/// plugin settings, MCP variables, and all caller-controlled sentinel values.
pub fn sanitized_environment_from<I>(
    inherited: I,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let entries: Vec<(OsString, OsString)> = inherited.into_iter().collect();
    let path = named(entries.clone(), "PATH").ok_or_else(|| {
        "cannot launch contained Claude: inherited PATH is unavailable".to_string()
    })?;
    let home = named(entries.clone(), "HOME");
    let user_profile = named(entries.clone(), "USERPROFILE");
    if home.is_none() && user_profile.is_none() {
        return Err("cannot launch contained Claude: HOME or USERPROFILE is unavailable for its existing login".into());
    }

    let mut vars = BTreeMap::new();
    vars.insert(OsString::from("PATH"), path);
    if let Some(home) = home {
        vars.insert(OsString::from("HOME"), home);
    }
    if let Some(profile) = user_profile {
        vars.insert(OsString::from("USERPROFILE"), profile);
    }
    for locale in ["LANG", "LC_ALL"] {
        if let Some(value) = named(entries.clone(), locale) {
            vars.insert(OsString::from(locale), value);
        }
    }
    vars.insert(
        OsString::from("CUTD_PROXY_ADDR"),
        OsString::from(proxy_addr),
    );
    vars.insert(
        OsString::from("CUTD_PROXY_ACTOR"),
        OsString::from(proxy_actor),
    );
    vars.insert(
        OsString::from("SHELLX_CUT_AGENT_CONTAINED"),
        OsString::from("1"),
    );
    Ok(LaunchEnvironment {
        clear_inherited: true,
        vars: vars.into_iter().collect(),
    })
}

pub fn sanitized_environment(
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String> {
    sanitized_environment_from(std::env::vars_os(), proxy_addr, proxy_actor)
}

/// Preserve the user's normal CLI environment and auth/config routing for
/// Codex. Cut only adds the exact live-engine proxy values consumed by its MCP
/// child; it does not copy, move, or rewrite the CLI's credential files.
pub fn native_environment(proxy_addr: &str, proxy_actor: &str) -> LaunchEnvironment {
    LaunchEnvironment {
        clear_inherited: false,
        vars: vec![
            (
                OsString::from("CUTD_PROXY_ADDR"),
                OsString::from(proxy_addr),
            ),
            (
                OsString::from("CUTD_PROXY_ACTOR"),
                OsString::from(proxy_actor),
            ),
        ],
    }
}

/// Providers with an implemented local Agent Chat route.
pub fn supported_headless_agent(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "grok")
        || (agent == "antigravity" && antigravity_supported_on_this_platform())
}

pub fn security_posture(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("contained: pinned Claude Code 2.1.224"),
        "codex" => Some("native CLI: uses your Codex settings and permissions"),
        "grok" => Some("isolated turn: only Cut MCP, existing Grok login"),
        "antigravity" if antigravity_supported_on_this_platform() => {
            Some("native CLI: uses your Antigravity sandbox and permissions")
        }
        "antigravity" => Some("disabled: Antigravity sandbox unavailable on Windows"),
        _ => None,
    }
}

/// Build the fixed CLI flags for the pinned Claude contract.
pub fn claude_args(mcp_config_path: &str, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--print".into(),
        "--output-format".into(),
        "json".into(),
        "--mcp-config".into(),
        mcp_config_path.into(),
        // Preserve subscription auth from HOME while refusing every user,
        // project, and local setting source (hooks/plugins/commands included).
        "--setting-sources".into(),
        "".into(),
        "--disable-slash-commands".into(),
        "--strict-mcp-config".into(),
        "--permission-mode".into(),
        "dontAsk".into(),
        "--allowedTools".into(),
        "mcp__cutd__*".into(),
        "--disallowedTools".into(),
        NATIVE_TOOL_DENIES.into(),
        "--no-session-persistence".into(),
    ];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        args.push("--model".into());
        args.push(model.into());
    }
    args
}

pub fn is_supported_claude_version(version: &str) -> bool {
    version.split_whitespace().next() == Some(SUPPORTED_CLAUDE_VERSION)
}

pub fn verify_claude_capability_contract(version: &str, help: &str) -> Result<(), String> {
    let found = version.split_whitespace().next().unwrap_or_default();
    if !is_supported_claude_version(version) {
        return Err(format!(
            "contained Agent Chat requires Claude Code {SUPPORTED_CLAUDE_VERSION}; found {found:?}. Cut refuses to launch an unverified CLI capability contract."
        ));
    }
    let missing: Vec<&str> = REQUIRED_HELP_TOKENS
        .iter()
        .copied()
        .filter(|token| !help.contains(token))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Claude Code {SUPPORTED_CLAUDE_VERSION} did not advertise required containment flags: {}. Cut refuses the turn.",
            missing.join(", ")
        ));
    }
    Ok(())
}

pub fn verify_codex_capability_contract(version: &str, exec_help: &str) -> Result<(), String> {
    if !version.to_ascii_lowercase().contains("codex") {
        return Err(format!(
            "the resolved Codex executable returned an unexpected version string: {:?}",
            version.trim()
        ));
    }
    let missing: Vec<&str> = REQUIRED_CODEX_EXEC_HELP_TOKENS
        .iter()
        .copied()
        .filter(|token| !exec_help.contains(token))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the installed Codex CLI does not advertise required Agent Chat flags: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

pub async fn verify_installed_agent(
    agent: &str,
    executable: &Path,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    verify::installed_agent(agent, executable, environment, workspace).await
}

#[cfg(test)]
#[path = "broker/tests.rs"]
mod tests;
