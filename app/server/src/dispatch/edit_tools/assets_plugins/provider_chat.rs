//! Exact provider selection and child policy for isolated Cut turns.

use crate::chat::broker::{
    isolated_grok_environment_for_provider, native_environment, sanitized_environment_for_provider,
    LaunchEnvironment,
};
use crate::gen::ProviderChildCommand;
use cut_core::{error_codes, CutError};
use std::path::Path;

/// Resolve a generation provider before any ordinary discovery can hide a bad
/// Runner context behind an "absent CLI" result.
pub(super) fn generation_provider(
    provider: &str,
    executable: &str,
) -> Result<ProviderChildCommand, CutError> {
    crate::gen::provider_child_command(provider, executable).map_err(|reason| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("{provider} was not admitted by the provider runtime context: {reason}"),
            "repair the enrolled provider selection before running assets.generate",
        )
    })
}

/// Build the narrowly scoped policy after provider admission and before either
/// the capability probe or the actual Agent Chat turn.
pub(super) fn launch_environment(
    agent: &str,
    provider_child: &ProviderChildCommand,
    workspace: &Path,
    proxy_addr: &str,
    proxy_actor: &str,
) -> Result<LaunchEnvironment, String> {
    let environment = match agent {
        "claude" => sanitized_environment_for_provider(
            provider_child.admitted_environment(),
            proxy_addr,
            proxy_actor,
        )?,
        "codex" | "antigravity" => native_environment(proxy_addr, proxy_actor),
        "grok" => isolated_grok_environment_for_provider(
            provider_child.admitted_environment(),
            workspace,
            proxy_addr,
            proxy_actor,
        )?,
        _ => return Err(format!("agent '{agent}' has no launch environment")),
    };
    if provider_child.admitted_environment().is_some() {
        environment.with_owned_temporary_directory(&workspace.join("tmp"))
    } else {
        Ok(environment)
    }
}
