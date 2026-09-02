//! Closed command and response contract for Motion runtime discovery.
//!
//! This is deliberately a read-only lane. Its command set cannot accept caller
//! ids, provider options, or arbitrary arguments, and its response helpers only
//! admit the exact runtime/catalog/descriptor relationship Cut can project.

use crate::motion_runtime::{
    build_motion_command_without_caller, run_motion_command_spec, MotionCommandSpec,
};
use cut_core::{error_codes, CutError};
use serde_json::Value;

pub(crate) const MOTION_DISCOVERY_CAPABILITY_ID: &str = "connector.template-to-cut@1";
/// Discovery must not inherit the five-minute budget used by render and job
/// commands. This override is deliberately capped rather than allowing an
/// operator to turn a Settings probe into a long-running operation.
const ENV_MOTION_DISCOVERY_TIMEOUT_MS: &str = "SHELLX_MOTION_DISCOVERY_TIMEOUT_MS";
const DEFAULT_MOTION_DISCOVERY_TIMEOUT_MS: u64 = 12_000;
const MIN_MOTION_DISCOVERY_TIMEOUT_MS: u64 = 1_000;
const MAX_MOTION_DISCOVERY_TIMEOUT_MS: u64 = 15_000;

/// The only Motion commands admitted to Cut's read-only discovery lane.
///
/// This is intentionally not a generic argument vector. Discovery commands
/// reject caller ids and other options upstream; making the set closed here
/// keeps this lane unable to become a render, job, or provider-auth transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionDiscoveryCommand {
    RuntimeProbe,
    ConnectorCatalog,
    ConnectorDescribeTemplateToCut,
}

impl MotionDiscoveryCommand {
    fn args(self) -> Vec<String> {
        match self {
            Self::RuntimeProbe => vec!["runtime-probe".to_string()],
            Self::ConnectorCatalog => vec!["connector".to_string(), "catalog".to_string()],
            Self::ConnectorDescribeTemplateToCut => vec![
                "connector".to_string(),
                "describe".to_string(),
                MOTION_DISCOVERY_CAPABILITY_ID.to_string(),
            ],
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Self::RuntimeProbe => "read the ShellX Motion runtime probe",
            Self::ConnectorCatalog => "read the ShellX Motion connector catalog",
            Self::ConnectorDescribeTemplateToCut => {
                "read the ShellX Motion Template-to-Cut connector descriptor"
            }
        }
    }

    fn response_command(self) -> &'static str {
        match self {
            Self::RuntimeProbe => "runtime-probe",
            Self::ConnectorCatalog => "connector catalog",
            Self::ConnectorDescribeTemplateToCut => "connector describe",
        }
    }
}

fn build_motion_discovery_command(command: MotionDiscoveryCommand) -> MotionCommandSpec {
    let mut spec = build_motion_command_without_caller(command.args());
    spec.timeout_ms = configured_motion_discovery_timeout_ms();
    spec
}

pub(crate) async fn run_motion_discovery_command(
    command: MotionDiscoveryCommand,
) -> Result<Value, CutError> {
    run_motion_command_spec(build_motion_discovery_command(command), command.operation()).await
}

/// Clamp the dedicated discovery budget independently from the general Motion
/// command timeout. The pure core makes a high environment override testable
/// without mutating process-wide environment state.
fn clamp_motion_discovery_timeout_ms(configured: Option<u64>) -> u64 {
    configured
        .unwrap_or(DEFAULT_MOTION_DISCOVERY_TIMEOUT_MS)
        .clamp(
            MIN_MOTION_DISCOVERY_TIMEOUT_MS,
            MAX_MOTION_DISCOVERY_TIMEOUT_MS,
        )
}

fn configured_motion_discovery_timeout_ms() -> u64 {
    let configured = std::env::var(ENV_MOTION_DISCOVERY_TIMEOUT_MS)
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    clamp_motion_discovery_timeout_ms(configured)
}

pub(crate) fn motion_discovery_payload<'a>(
    envelope: &'a Value,
    command: MotionDiscoveryCommand,
    field: &str,
) -> Result<&'a Value, CutError> {
    if envelope.get("ok").and_then(Value::as_bool) != Some(true)
        || envelope.get("command").and_then(Value::as_str) != Some(command.response_command())
    {
        return Err(motion_discovery_refusal(format!(
            "the {} response did not identify a successful {} command",
            field,
            command.response_command(),
        )));
    }
    envelope.get(field).ok_or_else(|| {
        motion_discovery_refusal(format!(
            "the {} response omitted its required {} payload",
            command.response_command(),
            field,
        ))
    })
}

pub(crate) fn validate_motion_discovery_descriptor(
    envelope: &Value,
    catalog_schema: &str,
    catalog_fingerprint: &str,
    catalog_descriptor: &Value,
    described_descriptor: &Value,
) -> Result<(), CutError> {
    let describe_catalog = envelope
        .get("catalog")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            motion_discovery_refusal("the connector describe response omitted its catalog identity")
        })?;
    if describe_catalog.len() != 2
        || describe_catalog.get("schema").and_then(Value::as_str) != Some(catalog_schema)
        || describe_catalog.get("fingerprint").and_then(Value::as_str) != Some(catalog_fingerprint)
    {
        return Err(motion_discovery_refusal(
            "the connector describe response does not bind to the validated catalog",
        ));
    }
    if described_descriptor != catalog_descriptor {
        return Err(motion_discovery_refusal(
            "the connector describe descriptor differs from the validated catalog descriptor",
        ));
    }
    Ok(())
}

pub(crate) fn required_motion_discovery_string(
    value: &Value,
    field: &str,
    label: &str,
) -> Result<String, CutError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty() && item.len() <= 128)
        .map(str::to_owned)
        .ok_or_else(|| {
            motion_discovery_refusal(format!(
                "the validated {label} has no bounded {field} string",
            ))
        })
}

pub(crate) fn motion_discovery_refusal(cause: impl Into<String>) -> CutError {
    CutError::new(
        error_codes::SIDECAR,
        "ShellX Motion is not compatible with Cut's read-only discovery contract",
        cause,
    )
    .with_suggested_action(
        "inspect the local ShellX Motion runtime; Cut does not manage Motion installation, repair, update, removal, or connector execution yet",
    )
}

#[cfg(test)]
#[path = "motion_discovery_contract_tests.rs"]
mod tests;
