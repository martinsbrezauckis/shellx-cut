//! Saved examples are shared with the UI lane; keep Rust's serde wire identical.
use cut_core::export_checks::{export_check_catalog, ExportChecks};
use cut_core::RenderReceipt;
use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn every_saved_state_and_phase_round_trips_without_information_loss() {
    let examples: Vec<Value> = serde_json::from_str(include_str!(
        "../../../schema/fixtures/export-checks/states.json"
    ))
    .unwrap();
    let catalog = export_check_catalog();
    assert_eq!(examples.len(), catalog.len() * 5 * 2);
    let ids: BTreeSet<_> = catalog.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), catalog.len());
    for example in examples {
        let parsed: ExportChecks = serde_json::from_value(example.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), example);
    }
}

#[test]
fn mixed_snapshots_round_trip_and_old_receipts_still_read() {
    for text in [
        include_str!("../../../schema/fixtures/export-checks/before-export.json"),
        include_str!("../../../schema/fixtures/export-checks/finished-file.json"),
    ] {
        let reply: Value = serde_json::from_str(text).unwrap();
        let checks: ExportChecks = serde_json::from_value(reply["export_checks"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            reply["export_checks"]
        );
    }
    let old: RenderReceipt = serde_json::from_str(include_str!(
        "../../../schema/fixtures/export-checks/legacy-receipt.json"
    ))
    .unwrap();
    assert!(old.pass);
    assert!(old.checks.is_empty());
}
