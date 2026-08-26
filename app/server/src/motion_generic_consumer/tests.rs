use super::adapter::{
    GenericConnectorSubmit, GenericMotionConsumer, GenericMotionTransport, JobProjection,
};
use super::canonical::test_fingerprint;
use super::{ConsumerError, MotionFailure};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;

#[path = "tests/generic_future.rs"]
mod generic_future;
#[path = "tests/lifecycle.rs"]
mod lifecycle;

const FUTURE_CAPABILITY: &str = "connector.future-render-to-cut@9";

struct MockTransport {
    probe: Value,
    catalog: Value,
    describes: VecDeque<Result<Value, ConsumerError>>,
    submissions: VecDeque<Result<Value, ConsumerError>>,
    gets: VecDeque<Result<Value, ConsumerError>>,
    lists: VecDeque<Result<Vec<Value>, ConsumerError>>,
    events: VecDeque<Result<Vec<Value>, ConsumerError>>,
    cancels: VecDeque<Result<Value, ConsumerError>>,
    retries: VecDeque<Result<Value, ConsumerError>>,
    deliveries: VecDeque<Result<Value, ConsumerError>>,
    callers: VecDeque<Result<String, ConsumerError>>,
    submitted: Vec<GenericConnectorSubmit>,
    calls: Vec<String>,
}

impl MockTransport {
    fn new(probe: Value, catalog: Value) -> Self {
        Self {
            probe,
            catalog,
            describes: VecDeque::new(),
            submissions: VecDeque::new(),
            gets: VecDeque::new(),
            lists: VecDeque::new(),
            events: VecDeque::new(),
            cancels: VecDeque::new(),
            retries: VecDeque::new(),
            deliveries: VecDeque::new(),
            callers: VecDeque::new(),
            submitted: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn next<T>(
        queue: &mut VecDeque<Result<T, ConsumerError>>,
        name: &str,
    ) -> Result<T, ConsumerError> {
        queue
            .pop_front()
            .unwrap_or_else(|| Err(ConsumerError::Transport(format!("unexpected {name} call"))))
    }
}

impl GenericMotionTransport for MockTransport {
    fn runtime_probe(&mut self) -> Result<Value, ConsumerError> {
        self.calls.push("runtime-probe".to_owned());
        Ok(self.probe.clone())
    }
    fn capability_catalog(&mut self) -> Result<Value, ConsumerError> {
        self.calls.push("connector-catalog".to_owned());
        Ok(self.catalog.clone())
    }
    fn capability_describe(&mut self, capability_id: &str) -> Result<Value, ConsumerError> {
        self.calls
            .push(format!("connector-describe:{capability_id}"));
        self.describes
            .pop_front()
            .unwrap_or_else(|| Ok(describe(&self.catalog, capability_id)))
    }
    fn caller_id(&mut self) -> Result<String, ConsumerError> {
        self.calls.push("caller-id".to_owned());
        self.callers
            .pop_front()
            .unwrap_or_else(|| Ok("cut:workspace".to_owned()))
    }
    fn submit_connector(
        &mut self,
        request: GenericConnectorSubmit,
    ) -> Result<Value, ConsumerError> {
        self.calls.push("connector-submit".to_owned());
        self.submitted.push(request);
        Self::next(&mut self.submissions, "submit")
    }
    fn job_get(&mut self, _job_id: &str) -> Result<Value, ConsumerError> {
        self.calls.push("job-get".to_owned());
        Self::next(&mut self.gets, "get")
    }
    fn job_list(&mut self) -> Result<Vec<Value>, ConsumerError> {
        self.calls.push("job-list".to_owned());
        Self::next(&mut self.lists, "list")
    }
    fn job_events(
        &mut self,
        _job_id: &str,
        _after: Option<u64>,
    ) -> Result<Vec<Value>, ConsumerError> {
        self.calls.push("job-events".to_owned());
        Self::next(&mut self.events, "events")
    }
    fn job_cancel(&mut self, _job_id: &str, _reason: &str) -> Result<Value, ConsumerError> {
        self.calls.push("job-cancel".to_owned());
        Self::next(&mut self.cancels, "cancel")
    }
    fn job_retry(&mut self, _job_id: &str) -> Result<Value, ConsumerError> {
        self.calls.push("job-retry".to_owned());
        Self::next(&mut self.retries, "retry")
    }
    fn connector_delivery(&mut self, _job_id: &str) -> Result<Value, ConsumerError> {
        self.calls.push("connector-delivery".to_owned());
        Self::next(&mut self.deliveries, "delivery")
    }
}

#[test]
fn future_typed_failures_and_lookup_errors_remain_distinct() {
    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog);
    transport.submissions.push_back(Ok(submission(
        &transport.catalog,
        descriptor(&transport.catalog, FUTURE_CAPABILITY)["fingerprint"]
            .as_str()
            .unwrap(),
    )));
    transport.gets.push_back(Ok(failed_job()));
    let lookup = MotionFailure {
        code: "job_not_visible".to_owned(),
        message: "another owner".to_owned(),
        retryable: false,
        remedy: None,
        retry_after_ms: None,
        suggested_action: Some("use the owning caller".to_owned()),
    };
    transport
        .gets
        .push_back(Err(ConsumerError::Lookup(lookup.clone())));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    let failure = match consumer.get(&job).unwrap() {
        JobProjection::Failed { error, .. } => error,
        other => panic!("expected typed failure, got {other:?}"),
    };
    assert_eq!(failure.code, "future.connector.capacity");
    assert_eq!(failure.message, "new connector capacity condition");
    assert!(failure.retryable);
    assert_eq!(failure.remedy.as_deref(), Some("wait"));
    assert_eq!(failure.retry_after_ms, Some(4_000));
    assert_eq!(
        failure.suggested_action.as_deref(),
        Some("retry after the host drains")
    );
    assert_eq!(consumer.get(&job), Err(ConsumerError::Lookup(lookup)));
}

#[test]
fn descriptor_drift_and_unsafe_or_unsupported_classes_refuse_before_projection() {
    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe.clone(), catalog.clone());
    let mut drift = describe(&catalog, FUTURE_CAPABILITY);
    drift["descriptor"]["revision"] = json!(3);
    set_fingerprint(&mut drift["descriptor"]);
    transport.describes.push_back(Ok(drift));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    assert!(consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned()
        )
        .is_err());
    assert!(consumer.transport().submitted.is_empty());

    let mut unsafe_catalog = catalog.clone();
    descriptor_mut(&mut unsafe_catalog, FUTURE_CAPABILITY)
        .as_object_mut()
        .unwrap()
        .get_mut("request")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("executable".to_owned(), json!("never"));
    resign_catalog(&mut unsafe_catalog);
    assert!(
        GenericMotionConsumer::discover(MockTransport::new(probe.clone(), unsafe_catalog)).is_err()
    );

    let mut string_catalog = catalog.clone();
    let field = &mut descriptor_mut(&mut string_catalog, FUTURE_CAPABILITY)["request"]["fields"][0];
    field["type"] = json!("string");
    field["maxLength"] = json!(128);
    resign_catalog(&mut string_catalog);
    assert!(
        GenericMotionConsumer::discover(MockTransport::new(probe.clone(), string_catalog)).is_err()
    );

    let mut partial_control_catalog = catalog.clone();
    descriptor_mut(&mut partial_control_catalog, FUTURE_CAPABILITY)["invocation"]["jobControls"] =
        json!(["get"]);
    resign_catalog(&mut partial_control_catalog);
    assert!(GenericMotionConsumer::discover(MockTransport::new(
        probe.clone(),
        partial_control_catalog
    ))
    .is_err());

    let mut bad_output = catalog.clone();
    descriptor_mut(&mut bad_output, FUTURE_CAPABILITY)
        .as_object_mut()
        .unwrap()
        .get_mut("outputs")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]["role"] = json!("motion_package");
    resign_catalog(&mut bad_output);
    assert!(
        GenericMotionConsumer::discover(MockTransport::new(probe.clone(), bad_output)).is_err()
    );

    let mut bad_trust_probe = probe;
    bad_trust_probe["provenance"]["managedDistribution"] = json!("managed");
    assert!(GenericMotionConsumer::discover(MockTransport::new(bad_trust_probe, catalog)).is_err());
}

#[test]
fn pending_cannot_be_queued_and_failed_is_never_success() {
    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog);
    transport.submissions.push_back(Ok(submission(
        &transport.catalog,
        descriptor(&transport.catalog, FUTURE_CAPABILITY)["fingerprint"]
            .as_str()
            .unwrap(),
    )));
    let mut queued = job("pending");
    queued["state"] = json!("queued");
    transport.gets.push_back(Ok(queued));
    transport.gets.push_back(Ok(failed_job()));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    assert!(consumer.get(&job).is_err());
    assert!(matches!(
        consumer.get(&job).unwrap(),
        JobProjection::Failed { .. }
    ));
}

#[test]
fn refuses_invented_event_semantics_and_submission_envelope_drift() {
    let (probe, catalog) = fixture();
    let fingerprint = descriptor(&catalog, FUTURE_CAPABILITY)["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut wrong_envelope = submission(&catalog, &fingerprint);
    wrong_envelope["callerId"] = json!("leaked-wire-field");
    let mut transport = MockTransport::new(probe.clone(), catalog.clone());
    transport.submissions.push_back(Ok(wrong_envelope));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    assert!(consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned()
        )
        .is_err());

    let mut transport = MockTransport::new(probe, catalog);
    transport
        .submissions
        .push_back(Ok(submission(&transport.catalog, &fingerprint)));
    transport.events.push_back(Ok(vec![event(
        1,
        "future_progress",
        json!({ "percent": 50 }),
    )]));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    assert!(consumer.events(&job, Some(0)).is_err());
}

fn fixture() -> (Value, Value) {
    let cut_resource = signed(
        json!({ "schema": "shellx-motion/docs-resource@1", "id": "motion.cut-and-design-studio", "revision": 1, "fingerprint": "" }),
    );
    let host_resource = signed(
        json!({ "schema": "shellx-motion/docs-resource@1", "id": "motion.host-integration", "revision": 1, "fingerprint": "" }),
    );
    let resource_fingerprint = host_resource["fingerprint"].clone();
    let future = admitted_descriptor(FUTURE_CAPABILITY, resource_fingerprint.clone());
    let descriptors = vec![
        compatibility_descriptor(
            "connector.canvas-bridge-export@1",
            resource_fingerprint.clone(),
        ),
        compatibility_descriptor("connector.canvas-to-mp4@1", resource_fingerprint.clone()),
        admitted_descriptor("connector.canvas-to-cut@1", resource_fingerprint.clone()),
        compatibility_descriptor(
            "connector.cut-generate-to-cut@1",
            resource_fingerprint.clone(),
        ),
        admitted_descriptor("connector.script-to-cut@1", resource_fingerprint.clone()),
        admitted_descriptor("connector.source-to-cut@1", resource_fingerprint.clone()),
        admitted_descriptor("connector.template-to-cut@1", resource_fingerprint.clone()),
        refused_descriptor("cut.c6-physics-handoff@1", resource_fingerprint.clone()),
        refused_descriptor(
            "cut.c7-scene-orchestration-handoff@1",
            resource_fingerprint.clone(),
        ),
        refused_descriptor("cut.scene3d-handoff@1", resource_fingerprint),
    ];
    let mut catalog = json!({
        "schema": "shellx-motion/capability-catalog@2", "protocol": { "min": 2, "max": 2, "preferred": 2 },
        "integrationCapabilities": integration(), "resources": [cut_resource, host_resource],
        "descriptors": descriptors, "fingerprint": ""
    });
    catalog["descriptors"].as_array_mut().unwrap().push(future);
    catalog["descriptors"]
        .as_array_mut()
        .unwrap()
        .sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    resign_catalog(&mut catalog);
    let probe = json!({
        "schema": "shellx-motion/runtime-probe@1", "engine": { "name": "@shellx-motion/core", "version": "0.6.110" }, "cli": { "name": "@shellx-motion/cli", "version": "0.6.110" },
        "runtime": { "platform": "linux", "architecture": "x64", "nodeVersion": "24.0.0" },
        "protocols": { "integration": { "min": 1, "max": 1, "preferred": 1 }, "capabilityCatalog": { "min": 1, "max": 2, "preferred": 2 }, "connectorJob": { "min": 1, "max": 2, "preferred": 2 } },
        "catalog": { "schema": "shellx-motion/capability-catalog@2", "fingerprint": catalog["fingerprint"], "descriptorCount": 11 },
        "provenance": { "execution": "source", "managedDistribution": "unmanaged", "distributionQualification": "unverified", "cleanHostQualification": "unverified" }
    });
    (probe, catalog)
}

fn integration() -> Value {
    json!({ "schema": "shellx-motion/integration-capabilities@1", "host": "shellx-motion", "protocol": { "min": 1, "max": 1, "preferred": 1 }, "schemas": { "cut": ["shellx-motion/cut-import-plan@1"] }, "modes": ["cut.import.plan"], "presets": ["mp4-h264"], "features": ["artifact.attestation"], "limits": { "maxPlanBytes": 4096, "maxArtifactBytes": 4096, "maxOperations": 10 } })
}

fn admitted_descriptor(id: &str, resource_fingerprint: Value) -> Value {
    signed(json!({
        "schema": "shellx-motion/capability-descriptor@2", "id": id, "revision": 2, "fingerprint": "", "title": "Future rendered import", "summary": "A future same-class rendered import.", "category": "cut-handoff",
        "documentation": { "resource": "motion.host-integration", "anchor": "generic-jobs", "resourceFingerprint": resource_fingerprint },
        "availability": { "state": "conditional", "reason": "admitted on Linux", "platforms": ["linux"], "execution": "generic-connector-job" },
        "request": { "schema": "shellx-motion/connector-request-schema@1", "id": "shellx-motion/connector-request/future-render@9", "maxBytes": 512, "fields": [ { "id": "input", "type": "opaque-reference", "required": true, "maxLength": 128 }, { "id": "output", "type": "opaque-reference", "required": true, "maxLength": 128 } ] },
        "invocation": { "schema": "shellx-motion/connector-job@2", "model": "fixed-generic-connector-job", "admission": "admitted", "jobControls": ["cancel", "events", "get", "list", "retry"] },
        "outputs": output_classes(), "requirements": { "integrationModes": ["cut.import.plan"], "integrationFeatures": ["artifact.attestation"], "permissionTier": "render_motion" }
    }))
}

fn compatibility_descriptor(id: &str, resource_fingerprint: Value) -> Value {
    signed(json!({
        "schema": "shellx-motion/capability-descriptor@2", "id": id, "revision": 2, "fingerprint": "", "title": "Compatibility", "summary": "Visible but compatibility-only.", "category": "host-bridge",
        "documentation": { "resource": "motion.host-integration", "anchor": "generic-jobs", "resourceFingerprint": resource_fingerprint },
        "availability": { "state": "compatibility-only", "reason": "named route", "platforms": ["darwin", "linux", "win32"], "execution": "named-cli-compatibility-only" },
        "request": { "schema": "shellx-motion/connector-request-schema@1", "id": "shellx-motion/connector-request/compatibility@1", "maxBytes": 1, "fields": [] },
        "invocation": { "schema": "shellx-motion/connector-job@2", "model": "fixed-generic-connector-job", "admission": "compatibility-only", "jobControls": [] },
        "outputs": [{ "role": "canvas_frame_selection", "mediaKinds": ["application/json"], "schemas": ["shellx-motion/canvas-frame-selection@1"] }], "requirements": { "integrationModes": ["canvas.bridge"], "integrationFeatures": [], "permissionTier": "write_local" }
    }))
}

fn refused_descriptor(id: &str, resource_fingerprint: Value) -> Value {
    signed(json!({
        "schema": "shellx-motion/capability-descriptor@2", "id": id, "revision": 2, "fingerprint": "", "title": "Refused", "summary": "Visible but refused.", "category": "scene-orchestration",
        "documentation": { "resource": "motion.host-integration", "anchor": "generic-jobs", "resourceFingerprint": resource_fingerprint },
        "availability": { "state": "refused", "reason": "not admitted", "platforms": ["darwin", "linux", "win32"], "execution": "not-admitted" },
        "request": { "schema": "shellx-motion/connector-request-schema@1", "id": "shellx-motion/connector-request/refused@1", "maxBytes": 1, "fields": [] },
        "invocation": { "schema": "shellx-motion/connector-job@2", "model": "fixed-generic-connector-job", "admission": "not-admitted", "jobControls": [] },
        "outputs": [], "requirements": { "integrationModes": ["cut.import.plan"], "integrationFeatures": [], "permissionTier": "write_local" }
    }))
}

fn output_classes() -> Value {
    json!([
        { "role": "artifact_handle", "mediaKinds": ["application/json"], "schemas": ["shellx-motion/artifact-handle@1"] },
        { "role": "cut_import_plan", "mediaKinds": ["application/json"], "schemas": ["shellx-motion/cut-import-plan@1"] },
        { "role": "receipt", "mediaKinds": ["application/json"], "schemas": ["shellx-motion/receipt@1"] },
        { "role": "rendered_media", "mediaKinds": ["video/mp4"], "schemas": ["shellx-motion/artifact-handle-ref@1"] }
    ])
}

fn submission(catalog: &Value, fingerprint: &str) -> Value {
    let catalog_fingerprint = catalog["fingerprint"].as_str().unwrap();
    let binding_fingerprint = test_fingerprint(&json!({
        "schema": "shellx-motion/connector-job-binding@1", "jobId": "cut:future-1", "callerId": "cut:workspace",
        "capabilityId": FUTURE_CAPABILITY, "descriptorRevision": 2, "descriptorFingerprint": fingerprint,
        "requestSchemaId": "shellx-motion/connector-request/future-render@9", "catalogFingerprint": catalog_fingerprint, "request": request()
    }));
    json!({ "ok": true, "jobId": "cut:future-1", "state": "pending", "lifecycle": "pending", "binding": {
        "capabilityId": FUTURE_CAPABILITY, "descriptorRevision": 2, "descriptorFingerprint": fingerprint,
        "requestSchemaId": "shellx-motion/connector-request/future-render@9", "catalogFingerprint": catalog_fingerprint,
        "bindingFingerprint": binding_fingerprint
    } })
}

fn describe(catalog: &Value, capability_id: &str) -> Value {
    json!({
        "ok": true,
        "command": "connector describe",
        "catalog": {
            "schema": "shellx-motion/capability-catalog@2",
            "fingerprint": catalog["fingerprint"]
        },
        "descriptor": descriptor(catalog, capability_id)
    })
}

fn request() -> Value {
    json!({ "input": "cut_input_42", "output": "cut_output_42" })
}
fn job(state: &str) -> Value {
    job_for("cut:future-1", state)
}
fn job_for(job_id: &str, state: &str) -> Value {
    match state {
        "pending" => {
            json!({ "schema": "shellx-motion/job-status@1", "jobId": job_id, "callerId": "cut:workspace", "lane": "connector", "operation": FUTURE_CAPABILITY, "lifecycle": "pending", "outcome": null, "state": "pending", "createdAtMs": 1, "pid": 10, "cancelRequested": null, "warnings": [], "pollAfterMs": 2000 })
        }
        "running" => {
            json!({ "schema": "shellx-motion/job-status@1", "jobId": job_id, "callerId": "cut:workspace", "lane": "connector", "operation": FUTURE_CAPABILITY, "lifecycle": "running", "outcome": null, "state": "running", "createdAtMs": 1, "startedAtMs": 10, "queueWaitMs": 9, "pid": 10, "cancelRequested": null, "warnings": [], "pollAfterMs": 2000 })
        }
        _ => unreachable!("fixture only models current live job states"),
    }
}

fn cancel_requested_job() -> Value {
    json!({ "schema": "shellx-motion/job-status@1", "jobId": "cut:future-1", "callerId": "cut:workspace", "lane": "connector", "operation": FUTURE_CAPABILITY, "lifecycle": "pending", "outcome": null, "state": "pending", "createdAtMs": 1, "pid": 10, "cancelRequested": { "requestedBy": "cut:workspace", "reason": "operator stopped next attempt", "requestedAtMs": 2 }, "warnings": [], "pollAfterMs": 2000 })
}
fn event(sequence: u64, event_type: &str, data: Value) -> Value {
    json!({ "schema": "shellx-motion/job-event@1", "seq": sequence, "atMs": 1_780_000_000_000_u64 + sequence, "type": event_type, "data": data })
}
fn failed_job() -> Value {
    json!({ "schema": "shellx-motion/job-status@1", "jobId": "cut:future-1", "callerId": "cut:workspace", "lane": "connector", "operation": FUTURE_CAPABILITY, "lifecycle": "ended", "outcome": "failed", "state": "failed", "createdAtMs": 1, "endedAtMs": 20, "durationMs": 10, "queueWaitMs": 9, "cancelRequested": null, "warnings": [], "error": { "code": "future.connector.capacity", "message": "new connector capacity condition", "retryable": true, "remedy": "wait", "retryAfterMs": 4000, "suggestedAction": "retry after the host drains" } })
}
fn succeeded_status() -> Value {
    json!({ "schema": "shellx-motion/job-status@1", "jobId": "cut:future-1", "callerId": "cut:workspace", "lane": "connector", "operation": FUTURE_CAPABILITY, "lifecycle": "ended", "outcome": "succeeded", "state": "succeeded", "createdAtMs": 1, "endedAtMs": 20, "durationMs": 10, "queueWaitMs": 9, "cancelRequested": null, "warnings": [], "receiptId": "receipt-future", "receiptPath": "/controlled/receipts/receipt-future.json" })
}
fn successful_delivery(catalog: &Value, fingerprint: &str) -> Value {
    let catalog_fingerprint = catalog["fingerprint"].as_str().unwrap();
    let binding_fingerprint = test_fingerprint(&json!({
        "schema": "shellx-motion/connector-job-binding@1", "jobId": "cut:future-1", "callerId": "cut:workspace",
        "capabilityId": FUTURE_CAPABILITY, "descriptorRevision": 2, "descriptorFingerprint": fingerprint,
        "requestSchemaId": "shellx-motion/connector-request/future-render@9", "catalogFingerprint": catalog_fingerprint, "request": request()
    }));
    json!({ "schema": "shellx-cut/motion-connector-delivery@1", "jobId": "cut:future-1", "binding": {
        "capabilityId": FUTURE_CAPABILITY, "descriptorRevision": 2, "descriptorFingerprint": fingerprint,
        "requestSchemaId": "shellx-motion/connector-request/future-render@9", "catalogFingerprint": catalog_fingerprint,
        "bindingFingerprint": binding_fingerprint
    }, "artifacts": [
        { "role": "artifact_handle", "schema": "shellx-motion/artifact-handle@1", "mediaKind": "application/json" },
        { "role": "rendered_media", "schema": "shellx-motion/artifact-handle-ref@1", "mediaKind": "video/mp4" }
    ], "receipts": [{ "role": "receipt", "schema": "shellx-motion/receipt@1", "status": "warning" }], "importPlan": { "schema": "shellx-motion/cut-import-plan@1", "ok": true, "mode": "rendered_media", "operations": [{ "verb": "cut.media.import_rendered", "renderedMedia": { "dryRun": false, "handle": { "schema": "shellx-motion/artifact-handle-ref@1" } } }], "receipt": { "schema": "shellx-motion/receipt@1", "status": "passed" } } })
}

fn descriptor<'a>(catalog: &'a Value, id: &str) -> &'a Value {
    catalog["descriptors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"].as_str() == Some(id))
        .unwrap()
}
fn descriptor_mut<'a>(catalog: &'a mut Value, id: &str) -> &'a mut Value {
    catalog["descriptors"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|value| value["id"].as_str() == Some(id))
        .unwrap()
}
fn signed(mut value: Value) -> Value {
    set_fingerprint(&mut value);
    value
}
fn set_fingerprint(value: &mut Value) {
    let object = value.as_object().unwrap();
    let mut content = Map::new();
    for (key, value) in object {
        if key != "fingerprint" {
            content.insert(key.clone(), value.clone());
        }
    }
    value["fingerprint"] = Value::String(test_fingerprint(&Value::Object(content)));
}
fn resign_catalog(catalog: &mut Value) {
    for descriptor in catalog["descriptors"].as_array_mut().unwrap() {
        set_fingerprint(descriptor);
    }
    set_fingerprint(catalog);
}
