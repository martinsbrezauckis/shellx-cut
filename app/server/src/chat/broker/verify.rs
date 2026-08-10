//! Bounded executable capability probes for Agent Chat providers.

use super::{
    verify_antigravity_capability_contract, verify_claude_capability_contract,
    verify_codex_capability_contract, verify_grok_capability_contract, LaunchEnvironment,
};
use std::path::Path;

async fn probe(
    agent: &str,
    executable: &Path,
    arguments: &[&str],
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<String, String> {
    let args = arguments
        .iter()
        .map(|argument| (*argument).to_string())
        .collect::<Vec<_>>();
    let mut command = crate::gen::agent_tokio_command(executable, &args)
        .map_err(|error| format!("cannot probe {agent} CLI: {error}"))?;
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
            format!("{agent} capability probe {} timed out", arguments.join(" "))
        }
        _ => format!(
            "{agent} capability probe {} failed: {error}",
            arguments.join(" ")
        ),
    })?;
    if !output.status.success() {
        return Err(format!(
            "{agent} capability probe {} exited {}",
            arguments.join(" "),
            output.status.code().unwrap_or(-1)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn verify_claude(
    executable: &Path,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    let version = probe("Claude", executable, &["--version"], environment, workspace).await?;
    let help = probe("Claude", executable, &["--help"], environment, workspace).await?;
    verify_claude_capability_contract(&version, &help)
}

pub(super) async fn installed_agent(
    agent: &str,
    executable: &Path,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    match agent {
        "claude" => verify_claude(executable, environment, workspace).await,
        "codex" => {
            let version =
                probe("Codex", executable, &["--version"], environment, workspace).await?;
            let help = probe(
                "Codex",
                executable,
                &["exec", "--help"],
                environment,
                workspace,
            )
            .await?;
            verify_codex_capability_contract(&version, &help)
        }
        "grok" => {
            let version = probe("Grok", executable, &["--version"], environment, workspace).await?;
            let help = probe("Grok", executable, &["--help"], environment, workspace).await?;
            verify_grok_capability_contract(&version, &help)
        }
        "antigravity" => {
            let version = probe(
                "Antigravity",
                executable,
                &["--version"],
                environment,
                workspace,
            )
            .await?;
            let help = probe(
                "Antigravity",
                executable,
                &["--help"],
                environment,
                workspace,
            )
            .await?;
            verify_antigravity_capability_contract(&version, &help)
        }
        _ => Err(format!("agent '{agent}' has no Agent Chat launch contract")),
    }
}
