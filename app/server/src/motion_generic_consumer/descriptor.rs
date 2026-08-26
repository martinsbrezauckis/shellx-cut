use super::catalog::REQUIRED_CONTROLS;
use super::contract::Descriptor;
use super::schema::{parse_outputs, parse_request_schema};
use super::syntax::*;
use super::ConsumerError;
use serde_json::Value;
use std::collections::BTreeMap;

const DESCRIPTOR_SCHEMA: &str = "shellx-motion/capability-descriptor@2";
const JOB_SCHEMA: &str = "shellx-motion/connector-job@2";

pub(super) fn parse_descriptor(
    value: &Value,
    resources: &BTreeMap<String, String>,
) -> Result<(String, Descriptor), ConsumerError> {
    let descriptor = required_object(
        value,
        &[
            "schema",
            "id",
            "revision",
            "fingerprint",
            "title",
            "summary",
            "category",
            "documentation",
            "availability",
            "request",
            "invocation",
            "outputs",
            "requirements",
        ],
        "capability descriptor",
    )?;
    required_exact_string(
        descriptor,
        "schema",
        DESCRIPTOR_SCHEMA,
        "capability descriptor",
    )?;
    let id = capability_id(&required_string_from(
        descriptor,
        "id",
        "capability descriptor id",
        129,
    )?)?;
    let fingerprint = checked_fingerprint(descriptor, "capability descriptor")?;
    let revision = integer(
        descriptor.get("revision").unwrap_or(&Value::Null),
        "capability descriptor revision",
        1,
        1_000_000,
    )? as u64;
    required_string_from(descriptor, "title", "capability descriptor title", 128)?;
    required_string_from(descriptor, "summary", "capability descriptor summary", 320)?;
    if !matches!(
        required_string_from(descriptor, "category", "capability descriptor category", 64)?
            .as_str(),
        "cut-handoff" | "host-bridge" | "render-export" | "scene-orchestration"
    ) {
        return Err(ConsumerError::refusal(
            "capability descriptor category is unsupported",
        ));
    }
    let documentation = required_object(
        descriptor.get("documentation").unwrap_or(&Value::Null),
        &["resource", "anchor", "resourceFingerprint"],
        "descriptor documentation",
    )?;
    let resource = identifier(
        &required_string_from(
            documentation,
            "resource",
            "descriptor documentation resource",
            128,
        )?,
        "descriptor documentation resource",
    )?;
    if resources.get(&resource)
        != Some(&sha256_string(&required_string_from(
            documentation,
            "resourceFingerprint",
            "descriptor documentation fingerprint",
            64,
        )?)?)
    {
        return Err(ConsumerError::refusal(
            "capability descriptor documentation resource is untrusted",
        ));
    }
    let anchor = required_string_from(
        documentation,
        "anchor",
        "descriptor documentation anchor",
        128,
    )?;
    if !anchor
        .chars()
        .all(|item| item.is_ascii_lowercase() || item.is_ascii_digit() || item == '-')
    {
        return Err(ConsumerError::refusal(
            "capability descriptor documentation anchor is unsafe",
        ));
    }
    let availability = required_object(
        descriptor.get("availability").unwrap_or(&Value::Null),
        &["state", "reason", "platforms", "execution"],
        "descriptor availability",
    )?;
    let availability_state =
        required_string_from(availability, "state", "descriptor availability state", 32)?;
    let execution = required_string_from(
        availability,
        "execution",
        "descriptor availability execution",
        64,
    )?;
    required_string_from(
        availability,
        "reason",
        "descriptor availability reason",
        320,
    )?;
    let platforms = string_array(
        availability.get("platforms").unwrap_or(&Value::Null),
        "descriptor availability platforms",
        1,
        3,
        16,
    )?;
    if platforms
        .iter()
        .any(|platform| !matches!(platform.as_str(), "darwin" | "linux" | "win32"))
    {
        return Err(ConsumerError::refusal(
            "capability descriptor names an unsupported platform",
        ));
    }
    let invocation = required_object(
        descriptor.get("invocation").unwrap_or(&Value::Null),
        &["schema", "model", "admission", "jobControls"],
        "descriptor invocation",
    )?;
    if required_string_from(invocation, "schema", "descriptor invocation schema", 96)? != JOB_SCHEMA
        || required_string_from(invocation, "model", "descriptor invocation model", 64)?
            != "fixed-generic-connector-job"
    {
        return Err(ConsumerError::refusal(
            "capability descriptor invocation class is unsupported",
        ));
    }
    let controls = string_array(
        invocation.get("jobControls").unwrap_or(&Value::Null),
        "descriptor invocation controls",
        0,
        5,
        16,
    )?;
    let admission = required_string_from(
        invocation,
        "admission",
        "descriptor invocation admission",
        32,
    )?;
    let admitted = availability_state == "conditional"
        && execution == "generic-connector-job"
        && admission == "admitted"
        && controls.as_slice() == REQUIRED_CONTROLS;
    let compatibility = availability_state == "compatibility-only"
        && execution == "named-cli-compatibility-only"
        && admission == "compatibility-only"
        && controls.is_empty();
    let refused = availability_state == "refused"
        && execution == "not-admitted"
        && admission == "not-admitted"
        && controls.is_empty();
    if !(admitted || compatibility || refused) {
        return Err(ConsumerError::refusal(
            "capability descriptor availability and invocation are inconsistent",
        ));
    }
    let request = parse_request_schema(descriptor.get("request").unwrap_or(&Value::Null))?;
    let outputs = parse_outputs(descriptor.get("outputs").unwrap_or(&Value::Null), admitted)?;
    let requirements = required_object(
        descriptor.get("requirements").unwrap_or(&Value::Null),
        &["integrationModes", "integrationFeatures", "permissionTier"],
        "descriptor requirements",
    )?;
    let modes = string_array(
        requirements.get("integrationModes").unwrap_or(&Value::Null),
        "descriptor integration modes",
        0,
        16,
        128,
    )?;
    let features = string_array(
        requirements
            .get("integrationFeatures")
            .unwrap_or(&Value::Null),
        "descriptor integration features",
        0,
        16,
        128,
    )?;
    if admitted
        && (modes != ["cut.import.plan"]
            || features != ["artifact.attestation"]
            || required_string_from(
                requirements,
                "permissionTier",
                "descriptor permission tier",
                32,
            )? != "render_motion")
    {
        return Err(ConsumerError::refusal(
            "admitted capability descriptor trust or integration class is unsupported",
        ));
    }
    Ok((
        id,
        Descriptor {
            revision,
            fingerprint,
            request,
            controls: controls.into_iter().collect(),
            outputs,
            admitted,
            availability_state,
            platforms: platforms.into_iter().collect(),
        },
    ))
}

pub(super) fn validate_integration(value: &Value) -> Result<(), ConsumerError> {
    let integration = required_object(
        value,
        &[
            "schema", "host", "protocol", "schemas", "modes", "presets", "features", "limits",
        ],
        "integration capabilities",
    )?;
    required_exact_string(
        integration,
        "schema",
        "shellx-motion/integration-capabilities@1",
        "integration capabilities",
    )?;
    required_exact_string(
        integration,
        "host",
        "shellx-motion",
        "integration capabilities",
    )?;
    require_protocol(
        integration.get("protocol").unwrap_or(&Value::Null),
        1,
        "integration protocol",
    )?;
    if !string_array(
        integration.get("modes").unwrap_or(&Value::Null),
        "integration modes",
        1,
        64,
        128,
    )?
    .contains(&"cut.import.plan".to_owned())
        || !string_array(
            integration.get("features").unwrap_or(&Value::Null),
            "integration features",
            1,
            64,
            128,
        )?
        .contains(&"artifact.attestation".to_owned())
    {
        return Err(ConsumerError::refusal(
            "Motion integration does not advertise the Cut import trust class",
        ));
    }
    let schemas = integration
        .get("schemas")
        .and_then(Value::as_object)
        .ok_or_else(|| ConsumerError::refusal("integration schemas must be an object"))?;
    if !schemas
        .get("cut")
        .and_then(Value::as_array)
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some("shellx-motion/cut-import-plan@1"))
        })
    {
        return Err(ConsumerError::refusal(
            "Motion integration does not advertise the Cut import plan schema",
        ));
    }
    Ok(())
}
