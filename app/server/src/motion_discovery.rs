//! Read-only ShellX Motion runtime discovery and management posture.
//!
//! This module coordinates the closed no-caller-id discovery contract and owns
//! its structural projection. It deliberately does not install, repair, update,
//! remove, log in, execute a connector, or turn a local source/PATH/npm runtime
//! into a managed distribution claim.

use crate::motion_discovery_contract::{
    motion_discovery_payload, motion_discovery_refusal, required_motion_discovery_string,
    run_motion_discovery_command, validate_motion_discovery_descriptor, MotionDiscoveryCommand,
    MOTION_DISCOVERY_CAPABILITY_ID,
};
use crate::motion_generic_consumer::ConsumerContract;
use crate::motion_runtime::motion_available;
use cut_core::CutError;
use serde_json::{json, Value};

const MOTION_DISCOVERY_PROJECTION_SCHEMA: &str = "shellx-cut/motion-discovery@1";
const MOTION_MANAGEMENT_STATUS_SCHEMA: &str = "shellx-cut/motion-management-status@1";
const MOTION_DIST_BLOCKER_ID: &str = "MOTION-DIST-01";
const MOTION_DIST_BLOCKER_MESSAGE: &str =
    "A verified immutable Motion distribution manifest and matching platform artifact are not available to Cut.";

/// Read and structurally validate the local Motion discovery contract.
///
/// This performs no provider login, render/job action, package management, or
/// connector execution. A successful result proves only that the local runtime,
/// catalog, and one fixed descriptor agree structurally; distribution and
/// execution remain explicitly unqualified.
async fn discover_motion_compatibility() -> Result<Value, CutError> {
    let runtime_envelope =
        run_motion_discovery_command(MotionDiscoveryCommand::RuntimeProbe).await?;
    let runtime_probe = motion_discovery_payload(
        &runtime_envelope,
        MotionDiscoveryCommand::RuntimeProbe,
        "probe",
    )?;

    let catalog_envelope =
        run_motion_discovery_command(MotionDiscoveryCommand::ConnectorCatalog).await?;
    let catalog = motion_discovery_payload(
        &catalog_envelope,
        MotionDiscoveryCommand::ConnectorCatalog,
        "catalog",
    )?;

    let contract = ConsumerContract::negotiate(runtime_probe, catalog).map_err(|error| {
        motion_discovery_refusal(format!(
            "the runtime probe and catalog failed Cut structural validation: {error:?}"
        ))
    })?;
    let availability = contract
        .availability()
        .into_iter()
        .find(|candidate| candidate.capability_id == MOTION_DISCOVERY_CAPABILITY_ID)
        .ok_or_else(|| {
            motion_discovery_refusal(
                "the expected Template-to-Cut capability is absent from the validated catalog",
            )
        })?;
    let catalog_descriptor = catalog
        .get("descriptors")
        .and_then(Value::as_array)
        .and_then(|descriptors| {
            descriptors.iter().find(|descriptor| {
                descriptor.get("id").and_then(Value::as_str) == Some(MOTION_DISCOVERY_CAPABILITY_ID)
            })
        })
        .ok_or_else(|| {
            motion_discovery_refusal(
                "the validated catalog did not retain the expected Template-to-Cut descriptor",
            )
        })?;
    let catalog_schema = required_motion_discovery_string(catalog, "schema", "catalog")?;
    let catalog_fingerprint = required_motion_discovery_string(catalog, "fingerprint", "catalog")?;

    let describe_envelope =
        run_motion_discovery_command(MotionDiscoveryCommand::ConnectorDescribeTemplateToCut)
            .await?;
    let described_descriptor = motion_discovery_payload(
        &describe_envelope,
        MotionDiscoveryCommand::ConnectorDescribeTemplateToCut,
        "descriptor",
    )?;
    validate_motion_discovery_descriptor(
        &describe_envelope,
        &catalog_schema,
        &catalog_fingerprint,
        catalog_descriptor,
        described_descriptor,
    )?;

    let descriptor_revision = catalog_descriptor
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            motion_discovery_refusal(
                "the validated Template-to-Cut descriptor has no bounded revision",
            )
        })?;
    let descriptor_fingerprint =
        required_motion_discovery_string(catalog_descriptor, "fingerprint", "descriptor")?;

    Ok(json!({
        "schema": MOTION_DISCOVERY_PROJECTION_SCHEMA,
        "status": "discovered-unmanaged",
        "runtime": {
            "engine": runtime_probe["engine"].clone(),
            "cli": runtime_probe["cli"].clone(),
            "platform": runtime_probe["runtime"]["platform"].clone(),
        },
        "catalog": {
            "schema": catalog_schema,
            "fingerprint": catalog_fingerprint,
            "descriptorCount": runtime_probe["catalog"]["descriptorCount"].clone(),
        },
        "connector": {
            "capabilityId": MOTION_DISCOVERY_CAPABILITY_ID,
            "descriptorRevision": descriptor_revision,
            "descriptorFingerprint": descriptor_fingerprint,
            "availability": availability.state,
            "availableOnRuntime": availability.available_on_runtime,
            "execution": "unqualified",
        },
        "provenance": runtime_probe["provenance"].clone(),
        "management": {
            "state": "unmanaged",
            "install": "unavailable",
            "repair": "unavailable",
            "update": "unavailable",
            "remove": "unavailable",
        },
    }))
}

/// Return Cut's current read-only Motion-management posture.
///
/// This stays separate from Doctor's synchronous cached scan: the runtime probe
/// owns a bounded async child process and must not be copied into a blocking
/// Doctor card. No distribution manifest is admitted yet, so every lifecycle
/// operation remains unavailable even when local discovery succeeds.
pub(crate) async fn motion_management_status() -> Value {
    match discover_motion_compatibility().await {
        Ok(discovery) => motion_management_status_from_discovery(&discovery),
        Err(_) => motion_management_status_without_discovery(motion_available()),
    }
}

fn motion_management_status_from_discovery(discovery: &Value) -> Value {
    json!({
        "schema": MOTION_MANAGEMENT_STATUS_SCHEMA,
        "readOnly": true,
        "runtime": {
            "status": "discovered-unmanaged",
            "engine": discovery["runtime"]["engine"].clone(),
            "cli": discovery["runtime"]["cli"].clone(),
            "platform": discovery["runtime"]["platform"].clone(),
            "provenance": discovery["provenance"].clone(),
        },
        "connector": {
            "catalog": "verified",
            "descriptor": "verified",
            "capabilityId": discovery["connector"]["capabilityId"].clone(),
            "descriptorRevision": discovery["connector"]["descriptorRevision"].clone(),
            "descriptorFingerprint": discovery["connector"]["descriptorFingerprint"].clone(),
            "availability": discovery["connector"]["availability"].clone(),
            "availableOnRuntime": discovery["connector"]["availableOnRuntime"].clone(),
            "execution": "unqualified",
        },
        "distribution": motion_distribution_blocker(),
    })
}

fn motion_management_status_without_discovery(candidate_present: bool) -> Value {
    let runtime_status = if candidate_present {
        "unverified"
    } else {
        "not-discovered"
    };
    json!({
        "schema": MOTION_MANAGEMENT_STATUS_SCHEMA,
        "readOnly": true,
        "runtime": {
            "status": runtime_status,
            "engine": null,
            "cli": null,
            "platform": null,
            "provenance": null,
        },
        "connector": {
            "catalog": "not-verified",
            "descriptor": "not-verified",
            "capabilityId": MOTION_DISCOVERY_CAPABILITY_ID,
            "descriptorRevision": null,
            "descriptorFingerprint": null,
            "availability": null,
            "availableOnRuntime": false,
            "execution": "not-executed",
        },
        "distribution": motion_distribution_blocker(),
    })
}

/// This is the single lifecycle authority until MOTION-DIST-01 lands. Do not
/// infer a candidate, version, source URL, digest, or installed state from a
/// source checkout / npm package / PATH discovery: none is a managed artifact.
fn motion_distribution_blocker() -> Value {
    json!({
        "status": "blocked",
        "blocker": {
            "id": MOTION_DIST_BLOCKER_ID,
            "message": MOTION_DIST_BLOCKER_MESSAGE,
        },
        "manifest": {
            "status": "absent",
            "candidate": null,
            "version": null,
        },
        "artifact": {
            "platform": std::env::consts::OS,
            "status": "absent",
            "version": null,
        },
        "installed": {
            "status": "not-managed",
            "candidate": null,
            "version": null,
        },
        "actions": {
            "install": { "available": false, "prerequisite": MOTION_DIST_BLOCKER_ID },
            "repair": { "available": false, "prerequisite": MOTION_DIST_BLOCKER_ID },
            "update": { "available": false, "prerequisite": MOTION_DIST_BLOCKER_ID },
            "remove": { "available": false, "prerequisite": MOTION_DIST_BLOCKER_ID },
        },
    })
}

#[cfg(test)]
#[path = "motion_discovery_tests.rs"]
mod tests;
