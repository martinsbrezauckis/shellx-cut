//! Bounded executable capability probes for Agent Chat providers.

use super::{capability_probe_args, verify_agent_capability_probe, LaunchEnvironment};
use std::path::Path;
use std::time::Duration;

/// How long the first `--help` capability probe may run. A probe that is still
/// running then is not a verdict on the provider: it is retried once with the
/// longer limit below before the check reports a timeout.
#[cfg(not(test))]
const CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Limit of the single retry after a first probe timed out (a loaded machine,
/// a virus scanner, a cold start of the provider CLI).
#[cfg(not(test))]
const CAPABILITY_PROBE_RETRY_TIMEOUT: Duration = Duration::from_secs(30);
/// Tests that import generated media or cancel a probe do not measure how fast
/// a child starts, so a busy machine gets a minute instead of ten seconds.
#[cfg(test)]
const CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const CAPABILITY_PROBE_RETRY_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a capability check did not pass. A timeout says nothing about what the
/// provider can do, so it is kept apart from a real refusal: callers must not
/// report a slow probe as "unsupported" or "unavailable", and nothing may cache
/// it as a negative answer (the check is stateless; every request probes again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityFailure {
    /// The probe did not finish within its limits, twice. The provider may be
    /// fine; the person can try again.
    TimedOut(String),
    /// The probe ran and the provider cannot do what Agent Chat needs.
    Unsupported(String),
}

impl CapabilityFailure {
    pub fn timed_out(&self) -> bool {
        matches!(self, Self::TimedOut(_))
    }

    /// Plain-words message for the person or the agent, including the action.
    pub fn message(&self) -> &str {
        match self {
            Self::TimedOut(message) | Self::Unsupported(message) => message,
        }
    }
}

impl std::fmt::Display for CapabilityFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Test hook: limits of the first probe and of its retry, in milliseconds
/// (0 = use the constants). Lets a test make a probe time out quickly.
#[cfg(test)]
pub(crate) static TEST_PROBE_LIMITS_MS: std::sync::Mutex<(u64, u64)> =
    std::sync::Mutex::new((0, 0));

fn probe_limits() -> (Duration, Duration) {
    #[cfg(test)]
    {
        let (first, retry) = *TEST_PROBE_LIMITS_MS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if first > 0 {
            return (Duration::from_millis(first), Duration::from_millis(retry));
        }
    }
    (CAPABILITY_PROBE_TIMEOUT, CAPABILITY_PROBE_RETRY_TIMEOUT)
}

/// Message for a probe that timed out in both attempts.
fn timeout_message(agent: &str, first: Duration, retry: Duration) -> String {
    format!(
        "{agent} did not answer its readiness check in time ({} s, then {} s). \
         This is not a verdict on {agent}: the machine may be busy. Try again in a moment.",
        first.as_secs_f32().ceil(),
        retry.as_secs_f32().ceil()
    )
}

enum ProbeFailure {
    TimedOut,
    Failed(String),
}

async fn probe(
    agent: &str,
    executable: &crate::gen::ProviderChildCommand,
    arguments: &[String],
    environment: &LaunchEnvironment,
    workspace: &Path,
    limit: Duration,
) -> Result<String, ProbeFailure> {
    let mut command = executable
        .tokio_command(arguments)
        .map_err(|error| ProbeFailure::Failed(format!("cannot probe {agent} CLI: {error}")))?;
    environment.apply_with_admitted_environment(&mut command, executable.admitted_environment());
    command.current_dir(workspace);
    let output = crate::jobs::run_owned(
        &mut command,
        None,
        &crate::jobs::ProcessControl::for_operation(limit),
    )
    .await
    .map_err(|error| match error.termination() {
        Some(crate::jobs::ProcessTermination::DeadlineExceeded) => ProbeFailure::TimedOut,
        _ => ProbeFailure::Failed(format!(
            "{agent} capability probe {} failed: {error}",
            arguments.join(" ")
        )),
    })?;
    if !output.status.success() {
        return Err(ProbeFailure::Failed(format!(
            "{agent} capability probe {} exited {}",
            arguments.join(" "),
            output.status.code().unwrap_or(-1)
        )));
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
    executable: &crate::gen::ProviderChildCommand,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), CapabilityFailure> {
    let arguments = capability_probe_args(agent, workspace).ok_or_else(|| {
        CapabilityFailure::Unsupported(format!("agent '{agent}' has no Agent Chat launch contract"))
    })?;
    let (first, retry) = probe_limits();
    let mut outcome = probe(
        display_name,
        executable,
        &arguments,
        environment,
        workspace,
        first,
    )
    .await;
    if matches!(outcome, Err(ProbeFailure::TimedOut)) {
        // One retry with a longer limit: a probe that only needed longer than
        // the first limit must not make the provider look unavailable.
        outcome = probe(
            display_name,
            executable,
            &arguments,
            environment,
            workspace,
            retry,
        )
        .await;
    }
    match outcome {
        Ok(output) => {
            verify_agent_capability_probe(agent, &output).map_err(CapabilityFailure::Unsupported)
        }
        Err(ProbeFailure::TimedOut) => Err(CapabilityFailure::TimedOut(timeout_message(
            display_name,
            first,
            retry,
        ))),
        Err(ProbeFailure::Failed(reason)) => Err(CapabilityFailure::Unsupported(reason)),
    }
}

pub(super) async fn installed_agent(
    agent: &str,
    executable: &crate::gen::ProviderChildCommand,
    environment: &LaunchEnvironment,
    workspace: &Path,
) -> Result<(), CapabilityFailure> {
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
        _ => Err(CapabilityFailure::Unsupported(format!(
            "agent '{agent}' has no Agent Chat launch contract"
        ))),
    }
}
