//! Focused tests for the closed, read-only Motion discovery lane.

use super::*;

#[test]
fn management_status_keeps_a_verified_local_runtime_unmanaged_and_unexecuted() {
    let discovery = json!({
        "runtime": {
            "engine": { "name": "@shellx-motion/core", "version": "0.2.66" },
            "cli": { "name": "@shellx-motion/cli", "version": "0.2.66" },
            "platform": "linux",
        },
        "provenance": {
            "execution": "source",
            "managedDistribution": "unmanaged",
            "distributionQualification": "unverified",
            "cleanHostQualification": "unverified",
        },
        "connector": {
            "capabilityId": MOTION_DISCOVERY_CAPABILITY_ID,
            "descriptorRevision": 1,
            "descriptorFingerprint": "a".repeat(64),
            "availability": "conditional",
            "availableOnRuntime": true,
        },
    });
    let status = motion_management_status_from_discovery(&discovery);
    assert_eq!(status["schema"], MOTION_MANAGEMENT_STATUS_SCHEMA);
    assert_eq!(status["runtime"]["status"], "discovered-unmanaged");
    assert_eq!(status["runtime"]["cli"]["version"], "0.2.66");
    assert_eq!(status["connector"]["catalog"], "verified");
    assert_eq!(status["connector"]["descriptor"], "verified");
    assert_eq!(status["connector"]["execution"], "unqualified");
    assert_eq!(
        status["distribution"]["blocker"]["id"],
        MOTION_DIST_BLOCKER_ID
    );
    assert_eq!(status["distribution"]["manifest"]["candidate"], Value::Null);
    for action in ["install", "repair", "update", "remove"] {
        assert_eq!(
            status["distribution"]["actions"][action]["available"],
            false
        );
        assert_eq!(
            status["distribution"]["actions"][action]["prerequisite"],
            MOTION_DIST_BLOCKER_ID
        );
    }
}

#[test]
fn management_status_never_turns_an_unverified_candidate_into_an_install_claim() {
    let status = motion_management_status_without_discovery(true);
    assert_eq!(status["runtime"]["status"], "unverified");
    assert_eq!(status["connector"]["execution"], "not-executed");
    assert_eq!(status["distribution"]["installed"]["status"], "not-managed");
    assert_eq!(status["distribution"]["artifact"]["status"], "absent");
}
