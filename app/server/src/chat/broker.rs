//! Launch policies for the local subscription CLIs used by `agent.chat`.
//!
//! Claude uses Cut's contained capability contract. Codex keeps the user's
//! configuration, rules, and safeguards while routing headless approval prompts
//! through its native automatic reviewer in Cut's disposable workspace.

use std::path::Path;

#[path = "broker/antigravity.rs"]
mod antigravity;
pub(crate) use antigravity::{
    args as antigravity_args, plugin_manifest as antigravity_plugin_manifest,
    project_config as antigravity_project_config,
    verify_capability_contract as verify_antigravity_capability_contract,
};
#[path = "broker/codex.rs"]
mod codex;
pub(crate) use codex::args as codex_args;
#[path = "broker/flags.rs"]
mod flags;
pub(crate) use flags::missing_required_help_tokens;
#[path = "broker/claude.rs"]
mod claude;
#[cfg(test)]
pub(crate) use claude::REQUIRED_CONTAINED_CLAUDE_HELP_TOKENS;
pub(crate) use claude::{
    verify_capability_contract as verify_claude_capability_contract,
    CONTAINED_CLAUDE_CAPABILITY_POSTURE,
};
#[path = "broker/grok.rs"]
mod grok;
pub(crate) use grok::{
    args as grok_args, isolated_environment_for_provider as isolated_grok_environment_for_provider,
    project_config as grok_project_config,
    verify_capability_contract as verify_grok_capability_contract,
};
#[path = "broker/environment.rs"]
mod environment;
pub use environment::{native_environment, sanitized_environment_for_provider, LaunchEnvironment};
#[cfg(test)]
pub(crate) use environment::{sanitized_environment_from, sanitized_environment_from_admitted};
#[path = "broker/verify.rs"]
mod verify;
const REQUIRED_CODEX_EXEC_HELP_TOKENS: &[&str] = &[
    "--config",
    "--json",
    "--skip-git-repo-check",
    "--ephemeral",
    "--approve-for-me",
    "--model",
];

const CLAUDE_CAPABILITY_HELP_ARGS: &[&str] = &["--help"];
const CODEX_CAPABILITY_HELP_ARGS: &[&str] = &["exec", "--help"];
const GROK_CAPABILITY_HELP_ARGS: &[&str] = &["--help"];
const ANTIGRAVITY_CAPABILITY_HELP_ARGS: &[&str] = &["--help"];

const NATIVE_TOOL_DENIES: &str = "Read,Write,Edit,NotebookEdit,Bash,BashOutput,KillShell,Task,WebFetch,WebSearch,Skill,mcp__cutd__agent_chat";

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

/// Providers with an implemented local Agent Chat route.
pub fn supported_headless_agent(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "grok" | "antigravity")
}

pub fn security_posture(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some(CONTAINED_CLAUDE_CAPABILITY_POSTURE),
        "codex" => Some("native CLI: disposable workspace with automatic approval review"),
        "grok" => Some("isolated turn: only Cut MCP, existing Grok login"),
        "antigravity" => Some("sandboxed unattended turn: disposable Cut-only MCP plugin"),
        _ => None,
    }
}

/// Build the fixed CLI flags for the contained Claude capability contract.
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

pub fn verify_codex_capability_contract(exec_help: &str) -> Result<(), String> {
    let missing = missing_required_help_tokens(exec_help, REQUIRED_CODEX_EXEC_HELP_TOKENS);
    if !missing.is_empty() {
        return Err(format!(
            "the installed Codex CLI does not advertise required Agent Chat flags: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

/// Exact help invocation used to verify one resolved local Agent Chat route.
pub(crate) fn capability_help_args(agent: &str) -> Option<&'static [&'static str]> {
    match agent {
        "claude" => Some(CLAUDE_CAPABILITY_HELP_ARGS),
        "codex" => Some(CODEX_CAPABILITY_HELP_ARGS),
        "grok" => Some(GROK_CAPABILITY_HELP_ARGS),
        "antigravity" => Some(ANTIGRAVITY_CAPABILITY_HELP_ARGS),
        _ => None,
    }
}

/// Side-effect-free executable admission for one resolved Agent Chat route.
/// Most providers publish every required flag in help. Grok parses the complete
/// contained argv but does not publish every compatibility flag; appending
/// `--version` makes that exact parser invocation exit before reading the prompt,
/// login, model, MCP, or session state. Admission never depends on a version pin.
pub(crate) fn capability_probe_args(agent: &str, workspace: &Path) -> Option<Vec<String>> {
    if agent == "grok" {
        let mut arguments = grok_args(
            &workspace.to_string_lossy(),
            Some("__shellx_cut_capability_probe__"),
        );
        arguments.push("--version".into());
        return Some(arguments);
    }
    capability_help_args(agent).map(|arguments| {
        arguments
            .iter()
            .map(|argument| (*argument).to_string())
            .collect()
    })
}

/// Validate the output of [`capability_probe_args`]. For Grok, successful
/// parsing of the complete contained argv is the capability proof and the
/// banner only binds that success to the expected executable identity. Other
/// providers retain exact advertised-help token verification.
pub(crate) fn verify_agent_capability_probe(agent: &str, output: &str) -> Result<(), String> {
    if agent == "grok" {
        let normalized = output.to_ascii_lowercase();
        if missing_required_help_tokens(&normalized, &["grok"]).is_empty() {
            return Ok(());
        }
        return Err("the installed Grok capability probe did not identify Grok".into());
    }
    verify_agent_capability_contract(agent, output)
}

/// Version banners are informational only. Admission depends exclusively on the
/// resolved executable's successful required-help output and exact flag contract.
pub(crate) fn verify_agent_capability_contract(agent: &str, help: &str) -> Result<(), String> {
    match agent {
        "claude" => verify_claude_capability_contract(help),
        "codex" => verify_codex_capability_contract(help),
        "grok" => verify_grok_capability_contract(help),
        "antigravity" => verify_antigravity_capability_contract(help),
        _ => Err(format!("agent '{agent}' has no Agent Chat launch contract")),
    }
}

pub async fn verify_installed_agent(
    agent: &str,
    executable: &crate::gen::ProviderChildCommand,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    verify::installed_agent(agent, executable, environment, workspace).await
}

#[cfg(test)]
#[path = "broker/tests.rs"]
mod tests;
