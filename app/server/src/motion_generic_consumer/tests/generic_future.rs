use super::super::adapter::{GenericConnectorSubmit, GenericMotionConsumer, JobProjection};
use super::*;
use serde_json::json;

#[test]
fn generic_future_capability_discovers_submits_polls_events_and_accepts_rendered_import() {
    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog);
    let prepared_fingerprint = descriptor(&transport.catalog, FUTURE_CAPABILITY)["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    transport
        .submissions
        .push_back(Ok(submission(&transport.catalog, &prepared_fingerprint)));
    transport.gets.push_back(Ok(job("running")));
    transport.events.push_back(Ok(vec![
        event(1, "submitted", json!({ "capabilityId": FUTURE_CAPABILITY })),
        event(2, "running", json!({ "stage": "drawing" })),
    ]));
    transport.gets.push_back(Ok(job("running")));
    transport.gets.push_back(Ok(succeeded_status()));
    transport.deliveries.push_back(Ok(successful_delivery(
        &transport.catalog,
        &prepared_fingerprint,
    )));
    transport.cancels.push_back(Ok(cancel_requested_job()));
    transport
        .lists
        .push_back(Ok(vec![job_for("cut:other", "pending"), job("running")]));

    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let availability = consumer.availability();
    assert_eq!(availability.len(), 11);
    assert!(availability
        .iter()
        .any(|entry| entry.capability_id == FUTURE_CAPABILITY
            && entry.state == "conditional"
            && entry.available_on_runtime));
    assert!(availability
        .iter()
        .any(|entry| entry.capability_id == "cut.scene3d-handoff@1" && entry.state == "refused"));
    assert!(availability.iter().any(|entry| entry.capability_id
        == "connector.canvas-bridge-export@1"
        && entry.state == "compatibility-only"));
    assert!(consumer
        .prepare("cut.scene3d-handoff@1", json!({}))
        .is_err());
    assert!(consumer
        .prepare("connector.canvas-bridge-export@1", json!({}))
        .is_err());
    let prepared = consumer
        .prepare(
            FUTURE_CAPABILITY,
            json!({ "input": "cut_input_42", "output": "cut_output_42" }),
        )
        .unwrap();
    let job = consumer
        .submit(prepared, "cut:future-1".to_owned())
        .unwrap();
    assert_eq!(
        consumer.transport().submitted[0],
        GenericConnectorSubmit {
            job_id: "cut:future-1".to_owned(),
            capability_id: FUTURE_CAPABILITY.to_owned(),
            descriptor_revision: 2,
            descriptor_fingerprint: prepared_fingerprint.clone(),
            request_schema_id: "shellx-motion/connector-request/future-render@9".to_owned(),
            request: request(),
        },
        "the actual Motion submit wire has no caller/catalog/binding fields"
    );
    assert_eq!(
        &consumer.transport().calls[2..],
        [
            "caller-id",
            "connector-describe:connector.future-render-to-cut@9",
            "connector-submit",
        ]
    );
    assert!(matches!(
        consumer.get(&job).unwrap(),
        JobProjection::Running { .. }
    ));
    let events = consumer.events(&job, Some(0)).unwrap();
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[1].event_type, "running");
    let listed = consumer.list(&job).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(matches!(listed[0], JobProjection::Running { .. }));
    assert!(matches!(
        consumer
            .cancel(&job, "operator stopped next attempt")
            .unwrap(),
        JobProjection::Pending {
            cancel_requested: true,
            ..
        }
    ));
    let succeeded = consumer.get(&job).unwrap();
    assert!(matches!(
        succeeded,
        JobProjection::Succeeded {
            receipt_available: true,
            ..
        }
    ));
    assert_eq!(
        consumer
            .accept_delivery(&job, &succeeded)
            .unwrap()
            .artifacts
            .len(),
        2
    );
    for source in [
        include_str!("../contract.rs"),
        include_str!("../adapter.rs"),
        include_str!("../delivery.rs"),
        include_str!("../status.rs"),
    ] {
        assert!(!source.contains(FUTURE_CAPABILITY));
    }
}
