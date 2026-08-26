use super::descriptor::{parse_descriptor, validate_integration};
use super::syntax::*;
use super::{contract::Descriptor, ConsumerError};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const CATALOG_SCHEMA: &str = "shellx-motion/capability-catalog@2";
const RUNTIME_PROBE_SCHEMA: &str = "shellx-motion/runtime-probe@1";
pub(super) const REQUIRED_CONTROLS: [&str; 5] = ["cancel", "events", "get", "list", "retry"];

pub(super) fn validate_runtime_probe(
    value: &Value,
) -> Result<(String, String, usize), ConsumerError> {
    let probe = required_object(
        value,
        &[
            "schema",
            "engine",
            "cli",
            "runtime",
            "protocols",
            "catalog",
            "provenance",
        ],
        "runtime probe",
    )?;
    required_exact_string(probe, "schema", RUNTIME_PROBE_SCHEMA, "runtime probe")?;
    let engine_version = named_version(
        probe.get("engine").unwrap_or(&Value::Null),
        "@shellx-motion/core",
    )?;
    let cli_version = named_version(
        probe.get("cli").unwrap_or(&Value::Null),
        "@shellx-motion/cli",
    )?;
    if engine_version != cli_version {
        return Err(ConsumerError::refusal(
            "Motion runtime engine and CLI versions do not identify the same build",
        ));
    }
    let runtime = required_object(
        probe.get("runtime").unwrap_or(&Value::Null),
        &["platform", "architecture", "nodeVersion"],
        "runtime",
    )?;
    let platform = required_string_from(runtime, "platform", "runtime platform", 16)?;
    if !matches!(platform.as_str(), "darwin" | "linux" | "win32") {
        return Err(ConsumerError::refusal(
            "Motion runtime platform is unsupported",
        ));
    }
    required_string_from(runtime, "architecture", "runtime architecture", 64)?;
    required_string_from(runtime, "nodeVersion", "runtime nodeVersion", 64)?;
    let protocols = required_object(
        probe.get("protocols").unwrap_or(&Value::Null),
        &["integration", "capabilityCatalog", "connectorJob"],
        "runtime protocols",
    )?;
    require_protocol(
        protocols.get("integration").unwrap_or(&Value::Null),
        1,
        "integration protocol",
    )?;
    require_protocol(
        protocols.get("capabilityCatalog").unwrap_or(&Value::Null),
        2,
        "capability catalog protocol",
    )?;
    require_protocol(
        protocols.get("connectorJob").unwrap_or(&Value::Null),
        2,
        "connector job protocol",
    )?;
    let catalog = required_object(
        probe.get("catalog").unwrap_or(&Value::Null),
        &["schema", "fingerprint", "descriptorCount"],
        "runtime probe catalog",
    )?;
    let schema = required_string_from(catalog, "schema", "runtime probe catalog schema", 96)?;
    let fingerprint = sha256_string(&required_string_from(
        catalog,
        "fingerprint",
        "runtime probe catalog fingerprint",
        64,
    )?)?;
    let descriptor_count = integer(
        catalog.get("descriptorCount").unwrap_or(&Value::Null),
        "runtime probe catalog descriptorCount",
        1,
        256,
    )? as usize;
    let provenance = required_object(
        probe.get("provenance").unwrap_or(&Value::Null),
        &[
            "execution",
            "managedDistribution",
            "distributionQualification",
            "cleanHostQualification",
        ],
        "runtime probe provenance",
    )?;
    if !matches!(
        required_string_from(provenance, "execution", "runtime provenance execution", 16)?.as_str(),
        "source" | "packed"
    ) || required_string_from(
        provenance,
        "managedDistribution",
        "runtime managed distribution",
        16,
    )? != "unmanaged"
        || required_string_from(
            provenance,
            "distributionQualification",
            "runtime distribution qualification",
            16,
        )? != "unverified"
        || required_string_from(
            provenance,
            "cleanHostQualification",
            "runtime clean-host qualification",
            16,
        )? != "unverified"
    {
        return Err(ConsumerError::refusal(
            "Motion runtime trust class is not admitted by the structural Cut consumer",
        ));
    }
    Ok((schema, fingerprint, descriptor_count))
}

pub(super) fn validate_catalog(
    value: &Value,
) -> Result<(String, Vec<(String, Descriptor)>, BTreeMap<String, String>), ConsumerError> {
    let catalog = required_object(
        value,
        &[
            "schema",
            "protocol",
            "integrationCapabilities",
            "resources",
            "descriptors",
            "fingerprint",
        ],
        "capability catalog",
    )?;
    required_exact_string(catalog, "schema", CATALOG_SCHEMA, "capability catalog")?;
    require_exact_protocol(
        catalog.get("protocol").unwrap_or(&Value::Null),
        2,
        "capability catalog protocol",
    )?;
    validate_integration(
        catalog
            .get("integrationCapabilities")
            .unwrap_or(&Value::Null),
    )?;
    let fingerprint = checked_fingerprint(catalog, "capability catalog")?;
    let resources = catalog
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| ConsumerError::refusal("capability catalog resources must be an array"))?;
    if resources.is_empty() || resources.len() > 64 {
        return Err(ConsumerError::refusal(
            "capability catalog resource count is outside bounds",
        ));
    }
    let mut resource_fingerprints = BTreeMap::new();
    let mut resource_ids = Vec::with_capacity(resources.len());
    for resource in resources {
        let resource = required_object(
            resource,
            &["schema", "id", "revision", "fingerprint"],
            "documentation resource",
        )?;
        required_exact_string(
            resource,
            "schema",
            "shellx-motion/docs-resource@1",
            "documentation resource",
        )?;
        let id = identifier(
            &required_string_from(resource, "id", "documentation resource id", 128)?,
            "documentation resource id",
        )?;
        integer(
            resource.get("revision").unwrap_or(&Value::Null),
            "documentation resource revision",
            1,
            1_000_000,
        )?;
        resource_ids.push(id.clone());
        resource_fingerprints.insert(id, checked_fingerprint(resource, "documentation resource")?);
    }
    require_sorted(resource_ids, "documentation resources")?;
    let descriptors = catalog
        .get("descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| ConsumerError::refusal("capability catalog descriptors must be an array"))?;
    if descriptors.is_empty() || descriptors.len() > 256 {
        return Err(ConsumerError::refusal(
            "capability catalog descriptor count is outside bounds",
        ));
    }
    let mut parsed = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        parsed.push(parse_descriptor(descriptor, &resource_fingerprints)?);
    }
    require_sorted(
        parsed.iter().map(|(id, _)| id.clone()).collect(),
        "capability descriptors",
    )?;
    Ok((fingerprint, parsed, resource_fingerprints))
}
