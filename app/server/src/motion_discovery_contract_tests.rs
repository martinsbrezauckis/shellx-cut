//! Focused tests for the closed Motion discovery command and response contract.

use super::*;
use serde_json::json;

#[test]
fn discovery_commands_are_literal_shell_free_and_never_carry_a_caller_id() {
    let cases = [
        (
            MotionDiscoveryCommand::RuntimeProbe,
            vec!["runtime-probe".to_string()],
        ),
        (
            MotionDiscoveryCommand::ConnectorCatalog,
            vec!["connector".to_string(), "catalog".to_string()],
        ),
        (
            MotionDiscoveryCommand::ConnectorDescribeTemplateToCut,
            vec![
                "connector".to_string(),
                "describe".to_string(),
                MOTION_DISCOVERY_CAPABILITY_ID.to_string(),
            ],
        ),
    ];

    for (command, expected_tail) in cases {
        let spec = build_motion_discovery_command(command);
        assert!(spec.args.ends_with(&expected_tail));
        assert!(!spec.args.iter().any(|arg| arg == "--caller-id"));
        assert!(!spec.args.iter().any(|arg| arg.starts_with("cut:")));
        assert!(
            (MIN_MOTION_DISCOVERY_TIMEOUT_MS..=MAX_MOTION_DISCOVERY_TIMEOUT_MS)
                .contains(&spec.timeout_ms),
            "discovery command must use its bounded timeout rather than the general job budget"
        );
    }
}

#[test]
fn discovery_timeout_is_independent_and_never_expands_past_fifteen_seconds() {
    assert_eq!(
        clamp_motion_discovery_timeout_ms(None),
        DEFAULT_MOTION_DISCOVERY_TIMEOUT_MS
    );
    assert_eq!(
        clamp_motion_discovery_timeout_ms(Some(1)),
        MIN_MOTION_DISCOVERY_TIMEOUT_MS
    );
    assert_eq!(clamp_motion_discovery_timeout_ms(Some(10_000)), 10_000);
    assert_eq!(
        clamp_motion_discovery_timeout_ms(Some(300_000)),
        MAX_MOTION_DISCOVERY_TIMEOUT_MS
    );
}

#[test]
fn discovery_payload_requires_its_exact_read_only_command_and_payload() {
    let probe = json!({
        "ok": true,
        "command": "runtime-probe",
        "probe": { "schema": "shellx-motion/runtime-probe@1" },
    });
    assert!(
        motion_discovery_payload(&probe, MotionDiscoveryCommand::RuntimeProbe, "probe",).is_ok()
    );

    let wrong_command = json!({
        "ok": true,
        "command": "connector catalog",
        "probe": {},
    });
    assert!(motion_discovery_payload(
        &wrong_command,
        MotionDiscoveryCommand::RuntimeProbe,
        "probe",
    )
    .is_err());

    let missing_payload = json!({ "ok": true, "command": "runtime-probe" });
    assert!(motion_discovery_payload(
        &missing_payload,
        MotionDiscoveryCommand::RuntimeProbe,
        "probe",
    )
    .is_err());
}

#[test]
fn discovery_descriptor_must_bind_exactly_to_the_validated_catalog() {
    let fingerprint = "a".repeat(64);
    let descriptor = json!({
        "id": MOTION_DISCOVERY_CAPABILITY_ID,
        "revision": 1,
        "fingerprint": "b".repeat(64),
    });
    let response = json!({
        "ok": true,
        "command": "connector describe",
        "catalog": {
            "schema": "shellx-motion/capability-catalog@2",
            "fingerprint": fingerprint.clone(),
        },
        "descriptor": descriptor.clone(),
    });
    assert!(validate_motion_discovery_descriptor(
        &response,
        "shellx-motion/capability-catalog@2",
        &fingerprint,
        &descriptor,
        &response["descriptor"],
    )
    .is_ok());

    let mut drifted = response.clone();
    drifted["descriptor"]["revision"] = json!(2);
    assert!(validate_motion_discovery_descriptor(
        &drifted,
        "shellx-motion/capability-catalog@2",
        &fingerprint,
        &descriptor,
        &drifted["descriptor"],
    )
    .is_err());
}
