//! Bounded executable capability probes for Agent Chat providers.

use super::{capability_probe_args, verify_agent_capability_probe, LaunchEnvironment};
use std::path::Path;

async fn probe(
    agent: &str,
    executable: &Path,
    arguments: &[String],
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<String, String> {
    let mut command = crate::gen::agent_tokio_command(executable, arguments)
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
    let help = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    Ok(String::from_utf8_lossy(help).into_owned())
}

async fn verify_capability_contract(
    agent: &str,
    display_name: &str,
    executable: &Path,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    let arguments = capability_probe_args(agent, workspace)
        .ok_or_else(|| format!("agent '{agent}' has no Agent Chat launch contract"))?;
    let output = probe(display_name, executable, &arguments, environment, workspace).await?;
    verify_agent_capability_probe(agent, &output)
}

pub(super) async fn installed_agent(
    agent: &str,
    executable: &Path,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), String> {
    match agent {
        "claude" => {
            verify_capability_contract("claude", "Claude", executable, environment, workspace).await
        }
        "codex" => {
            verify_capability_contract("codex", "Codex", executable, environment, workspace).await
        }
        "grok" => {
            verify_capability_contract("grok", "Grok", executable, environment, workspace).await
        }
        "antigravity" => {
            verify_capability_contract(
                "antigravity",
                "Antigravity",
                executable,
                environment,
                workspace,
            )
            .await
        }
        _ => Err(format!("agent '{agent}' has no Agent Chat launch contract")),
    }
}
