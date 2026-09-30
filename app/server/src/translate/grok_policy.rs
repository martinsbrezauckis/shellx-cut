//! Grok translation uses the planners' capability-admitted no-tool contract.
//!
//! Allow then remove a nonempty native tool set rather than assuming empty
//! `--tools` semantics. Bare MCPTool denies model-originated MCP invocations;
//! trusted configured MCP startup remains the provider's lifecycle policy.

use crate::gen::ProviderChildCommand;
use crate::jobs::{run_owned, ProcessControl, ProcessTermination};
use cut_core::{error_codes, CutError};
use std::{path::Path, time::Duration};

const REQUIRED_FLAGS: &[&str] = &[
    "--tools",
    "--disallowed-tools",
    "--deny",
    "--no-subagents",
    "--disable-web-search",
    "--sandbox",
    "--permission-mode",
    "--prompt-file",
    "--output-format",
    "--no-memory",
    "--max-turns",
    "--model",
];

pub(super) fn arguments() -> Vec<String> {
    [
        "--tools",
        "read_file,grep,list_dir",
        "--disallowed-tools",
        "read_file,grep,list_dir,Agent",
        "--deny",
        "MCPTool",
        "--no-subagents",
        "--disable-web-search",
        "--sandbox",
        "read-only",
        "--permission-mode",
        "dontAsk",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn refused(reason: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        "the Grok CLI does not meet the translation no-tool contract",
        reason,
    )
    .with_suggested_action("update the selected CLI or choose another translation backend")
}

/// Probe only help, under the selected child environment and existing job
/// cancellation. Charge admission against the translation's existing budget.
pub(super) async fn admit(
    provider: &ProviderChildCommand,
    workspace: &Path,
    budget: Duration,
) -> Result<Duration, CutError> {
    let started = std::time::Instant::now();
    let mut command = provider
        .tokio_command(&["--help".into()])
        .map_err(refused)?;
    provider
        .apply_admitted_environment(&mut command, workspace)
        .map_err(refused)?;
    command.current_dir(workspace);
    let output = run_owned(
        &mut command,
        None,
        &ProcessControl::for_operation(budget.min(Duration::from_secs(10))),
    )
    .await
    .map_err(|error| match error.termination() {
        Some(ProcessTermination::Cancelled(reason)) => CutError::new(
            "job_cancelled",
            format!("translation CLI cancelled ({})", reason.label()),
            "the owning background job stopped capability admission",
        ),
        _ => refused(format!("Grok capability help probe failed: {error}")),
    })?;
    if !output.status.success() || output.diagnostics_truncated() {
        return Err(refused(
            "Grok capability help probe failed or was truncated",
        ));
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let missing = crate::chat::broker::missing_required_help_tokens(&help, REQUIRED_FLAGS);
    if !missing.is_empty() {
        return Err(refused(format!(
            "Grok does not advertise required translation flags: {}",
            missing.join(", ")
        )));
    }
    budget
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| refused("no translation time remains after capability admission"))
}

#[cfg(test)]
#[path = "grok_policy_tests.rs"]
mod tests;
