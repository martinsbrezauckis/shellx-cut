//! Contained Claude Code launch policy for `agent.chat`.
//!
//! This is deliberately pinned to the locally verified Claude Code contract.
//! A different upstream version is not assumed to preserve tool-denial
//! semantics: it fails closed before a chat prompt is sent.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

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

/// The intentionally small environment inherited by a contained Claude turn.
#[derive(Clone, Debug)]
pub struct SanitizedEnvironment {
    vars: Vec<(OsString, OsString)>,
}

impl SanitizedEnvironment {
    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        self.vars
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect()
    }

    pub fn apply(&self, command: &mut tokio::process::Command) {
        command.env_clear();
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
) -> Result<SanitizedEnvironment, String>
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
    Ok(SanitizedEnvironment {
        vars: vars.into_iter().collect(),
    })
}

pub fn sanitized_environment(
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<SanitizedEnvironment, String> {
    sanitized_environment_from(std::env::vars_os(), proxy_addr, proxy_actor)
}

/// The only headless Agent Chat route that has an executable containment
/// contract today. Codex needs danger-full-access for MCP calls and Grok has no
/// verified native-tool deny mode, so neither is launched headlessly.
pub fn supported_headless_agent(agent: &str) -> bool {
    agent == "claude"
}

pub fn unavailable_reason(agent: &str) -> Option<&'static str> {
    match agent {
        "codex" => Some(
            "Codex Agent Chat is disabled: its headless MCP route requires danger-full-access, so Cut cannot enforce native file, shell, or network denial. Use the pinned contained Claude route instead.",
        ),
        "grok" => Some(
            "Grok Agent Chat is disabled: Cut has no verified upstream native-tool deny contract for its headless CLI. Use the pinned contained Claude route instead.",
        ),
        _ => None,
    }
}

pub fn security_posture(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("contained: pinned Claude Code 2.1.224"),
        "codex" | "grok" => Some("disabled: no enforceable containment"),
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

async fn probe(
    executable: &Path,
    argument: &str,
    environment: &SanitizedEnvironment,
    workspace: &Path,
) -> Result<String, String> {
    let args = vec![argument.to_string()];
    let mut command = crate::gen::agent_tokio_command(executable, &args)
        .map_err(|error| format!("cannot probe Claude CLI safely: {error}"))?;
    environment.apply(&mut command);
    command.current_dir(workspace);
    let output = crate::jobs::run_owned(
        &mut command,
        None,
        &crate::jobs::ProcessControl::for_operation(std::time::Duration::from_secs(10)),
    )
    .await
    .map_err(|error| match error.termination() {
        Some(crate::jobs::ProcessTermination::DeadlineExceeded) => {
            format!("Claude capability probe {argument} timed out")
        }
        _ => format!("Claude capability probe {argument} failed: {error}"),
    })?;
    if !output.status.success() {
        return Err(format!(
            "Claude capability probe {argument} exited {}",
            output.status.code().unwrap_or(-1)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub async fn verify_installed_claude(
    executable: &Path,
    environment: &SanitizedEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    let version = probe(executable, "--version", environment, workspace).await?;
    let help = probe(executable, "--help", environment, workspace).await?;
    verify_claude_capability_contract(&version, &help)
}

#[cfg(test)]
#[path = "broker/tests.rs"]
mod tests;
