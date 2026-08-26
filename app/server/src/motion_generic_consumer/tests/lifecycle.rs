use super::*;
use serde_json::json;

#[test]
fn retry_requires_a_retryable_failure_and_refreshes_the_original_binding() {
    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog);
    let fingerprint = descriptor(&transport.catalog, FUTURE_CAPABILITY)["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    transport
        .submissions
        .push_back(Ok(submission(&transport.catalog, &fingerprint)));
    transport.gets.push_back(Ok(failed_job()));
    transport.retries.push_back(Ok(
        json!({ "jobId": "cut:future-2", "priorJobId": "cut:future-1" }),
    ));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    let retry = consumer.retry(&job).unwrap();
    assert_eq!(retry.job_id, "cut:future-2");
    assert_eq!(
        &consumer.transport().calls[5..],
        [
            "job-get",
            "runtime-probe",
            "connector-catalog",
            "connector-describe:connector.future-render-to-cut@9",
            "caller-id",
            "job-retry",
        ]
    );

    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog.clone());
    let fingerprint = descriptor(&catalog, FUTURE_CAPABILITY)["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    transport
        .submissions
        .push_back(Ok(submission(&catalog, &fingerprint)));
    transport.gets.push_back(Ok(failed_job()));
    transport
        .describes
        .push_back(Ok(describe(&catalog, FUTURE_CAPABILITY)));
    let mut drift = describe(&catalog, FUTURE_CAPABILITY);
    drift["catalog"]["fingerprint"] = json!("a".repeat(64));
    transport.describes.push_back(Ok(drift));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    assert!(consumer.retry(&job).is_err());
    assert!(!consumer
        .transport()
        .calls
        .iter()
        .any(|call| call == "job-retry"));

    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog.clone());
    let fingerprint = descriptor(&catalog, FUTURE_CAPABILITY)["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    transport
        .submissions
        .push_back(Ok(submission(&catalog, &fingerprint)));
    transport.gets.push_back(Ok(failed_job()));
    transport.callers.push_back(Ok("cut:workspace".to_owned()));
    transport.callers.push_back(Ok("cut:other".to_owned()));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    assert!(consumer.retry(&job).is_err());
    assert!(!consumer
        .transport()
        .calls
        .iter()
        .any(|call| call == "job-retry"));
}

#[test]
fn cancel_requires_a_live_job_and_events_and_deliveries_refuse_authority_or_gaps() {
    let (probe, catalog) = fixture();
    let mut transport = MockTransport::new(probe, catalog);
    let fingerprint = descriptor(&transport.catalog, FUTURE_CAPABILITY)["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();
    transport
        .submissions
        .push_back(Ok(submission(&transport.catalog, &fingerprint)));
    transport.gets.push_back(Ok(succeeded_status()));
    transport.events.push_back(Ok(vec![
        event(1, "submitted", json!({})),
        event(3, "running", json!({})),
    ]));
    let mut consumer = GenericMotionConsumer::discover(transport).unwrap();
    let job = consumer
        .submit(
            consumer.prepare(FUTURE_CAPABILITY, request()).unwrap(),
            "cut:future-1".to_owned(),
        )
        .unwrap();
    assert!(consumer.cancel(&job, "too late").is_err());
    assert!(consumer.transport().cancels.is_empty());
    assert!(consumer.events(&job, Some(0)).is_err());
    assert!(super::super::delivery::contains_direct_authority(&json!({
        "nested": {
            "receiptPath": "/private/receipt.json",
            "sourceUrl": "https://unsafe.example",
            "workspaceRoot": "/private/workspace"
        }
    })));
}
