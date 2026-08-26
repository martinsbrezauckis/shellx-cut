use super::catalog::{validate_catalog, validate_runtime_probe, CATALOG_SCHEMA};
use super::descriptor::parse_descriptor;
use super::schema::validate_request;
use super::syntax::{required_exact_string, required_object, required_string_from, sha256_string};
use super::ConsumerError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(crate) struct ConsumerContract {
    platform: String,
    catalog_fingerprint: String,
    descriptors: BTreeMap<String, Descriptor>,
    resources: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(super) struct Descriptor {
    pub(super) revision: u64,
    pub(super) fingerprint: String,
    pub(super) request: RequestSchema,
    pub(super) controls: BTreeSet<String>,
    pub(super) outputs: BTreeMap<String, OutputClass>,
    pub(super) admitted: bool,
    pub(super) availability_state: String,
    pub(super) platforms: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub(super) struct RequestSchema {
    pub(super) id: String,
    pub(super) max_bytes: usize,
    pub(super) fields: Vec<RequestField>,
}

#[derive(Clone, Debug)]
pub(super) struct RequestField {
    pub(super) id: String,
    pub(super) required: bool,
    pub(super) class: RequestClass,
}

#[derive(Clone, Debug)]
pub(super) enum RequestClass {
    Boolean,
    Enum(BTreeSet<String>),
    Integer { minimum: i64, maximum: i64 },
    OpaqueReference { max_length: usize },
}

#[derive(Clone, Debug)]
pub(super) struct OutputClass {
    pub(super) media_kind: String,
    pub(super) schema: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedMotionRequest {
    pub(crate) capability_id: String,
    pub(crate) descriptor_revision: u64,
    pub(crate) descriptor_fingerprint: String,
    pub(crate) request_schema_id: String,
    pub(crate) catalog_fingerprint: String,
    pub(crate) request: Value,
    pub(crate) controls: BTreeSet<String>,
    pub(super) outputs: BTreeMap<String, OutputClass>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapabilityAvailability {
    pub(crate) capability_id: String,
    pub(crate) state: String,
    pub(crate) available_on_runtime: bool,
    pub(crate) controls: BTreeSet<String>,
}

impl ConsumerContract {
    pub(crate) fn negotiate(probe: &Value, catalog: &Value) -> Result<Self, ConsumerError> {
        let probe_catalog = validate_runtime_probe(probe)?;
        let platform = probe
            .get("runtime")
            .and_then(Value::as_object)
            .and_then(|runtime| runtime.get("platform"))
            .and_then(Value::as_str)
            .filter(|platform| matches!(*platform, "darwin" | "linux" | "win32"))
            .ok_or_else(|| ConsumerError::refusal("Motion runtime platform is unsupported"))?
            .to_owned();
        let (catalog_fingerprint, descriptors, resources) = validate_catalog(catalog)?;
        if probe_catalog.0 != "shellx-motion/capability-catalog@2"
            || probe_catalog.1 != catalog_fingerprint
            || probe_catalog.2 != descriptors.len()
        {
            return Err(ConsumerError::refusal(
                "Motion runtime probe and capability catalog do not describe the same catalog",
            ));
        }
        Ok(Self {
            platform,
            catalog_fingerprint,
            descriptors: descriptors.into_iter().collect(),
            resources,
        })
    }

    pub(crate) fn prepare(
        &self,
        capability_id: &str,
        request: Value,
    ) -> Result<PreparedMotionRequest, ConsumerError> {
        let descriptor = self.descriptors.get(capability_id).ok_or_else(|| {
            ConsumerError::refusal("Motion capability is not present in the discovered catalog")
        })?;
        if !descriptor.admitted {
            return Err(ConsumerError::refusal(
                "Motion capability is visible but not admitted for generic Cut submission",
            ));
        }
        if !descriptor.platforms.contains(&self.platform) {
            return Err(ConsumerError::refusal(
                "Motion capability is not admitted on the probed runtime platform",
            ));
        }
        validate_request(&descriptor.request, &request)?;
        Ok(PreparedMotionRequest {
            capability_id: capability_id.to_owned(),
            descriptor_revision: descriptor.revision,
            descriptor_fingerprint: descriptor.fingerprint.clone(),
            request_schema_id: descriptor.request.id.clone(),
            catalog_fingerprint: self.catalog_fingerprint.clone(),
            request,
            controls: descriptor.controls.clone(),
            outputs: descriptor.outputs.clone(),
        })
    }

    pub(crate) fn availability(&self) -> Vec<CapabilityAvailability> {
        self.descriptors
            .iter()
            .map(|(capability_id, descriptor)| CapabilityAvailability {
                capability_id: capability_id.clone(),
                state: descriptor.availability_state.clone(),
                available_on_runtime: descriptor.platforms.contains(&self.platform),
                controls: descriptor.controls.clone(),
            })
            .collect()
    }

    pub(crate) fn require_described_descriptor(
        &self,
        prepared: &PreparedMotionRequest,
        value: Value,
    ) -> Result<(), ConsumerError> {
        let envelope = required_object(
            &value,
            &["ok", "command", "catalog", "descriptor"],
            "Motion connector describe response",
        )?;
        if envelope.get("ok").and_then(Value::as_bool) != Some(true)
            || required_string_from(
                envelope,
                "command",
                "Motion connector describe response",
                64,
            )? != "connector describe"
        {
            return Err(ConsumerError::refusal(
                "Motion connector describe response is not successful",
            ));
        }
        let catalog = required_object(
            envelope.get("catalog").unwrap_or(&Value::Null),
            &["schema", "fingerprint"],
            "Motion connector describe catalog",
        )?;
        required_exact_string(
            catalog,
            "schema",
            CATALOG_SCHEMA,
            "Motion connector describe catalog",
        )?;
        if sha256_string(&required_string_from(
            catalog,
            "fingerprint",
            "Motion connector describe catalog",
            64,
        )?)? != prepared.catalog_fingerprint
        {
            return Err(ConsumerError::refusal(
                "Motion connector describe catalog drifted from the prepared request",
            ));
        }
        let (id, descriptor) = parse_descriptor(
            envelope.get("descriptor").unwrap_or(&Value::Null),
            &self.resources,
        )?;
        if id != prepared.capability_id
            || descriptor.revision != prepared.descriptor_revision
            || descriptor.fingerprint != prepared.descriptor_fingerprint
            || descriptor.request.id != prepared.request_schema_id
            || !descriptor.admitted
            || !descriptor.platforms.contains(&self.platform)
        {
            return Err(ConsumerError::refusal(
                "Motion connector describe descriptor drifted from the prepared request",
            ));
        }
        Ok(())
    }
}
