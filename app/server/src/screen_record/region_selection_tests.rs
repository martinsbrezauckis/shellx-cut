use super::region_selection::{
    NativeMonitorIdentity, NativeRegionCrop, RegionSelectionConsumeError,
    RegionSelectionIssueError, RegionSelectionRegistry, RegionSelectionValue,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10);

fn registry(capacity: usize) -> RegionSelectionRegistry {
    RegionSelectionRegistry::new(NonZeroUsize::new(capacity).unwrap(), TTL)
}

fn selection(monitor: &str) -> RegionSelectionValue {
    RegionSelectionValue::new(
        NativeMonitorIdentity::new(monitor).unwrap(),
        NativeRegionCrop::new(20, 40, 200, 100, 1920, 1080).unwrap(),
    )
}

fn fill(byte: u8) -> impl FnMut(&mut [u8; 32]) -> Result<(), RegionSelectionIssueError> {
    move |bytes| {
        *bytes = [byte; 32];
        Ok(())
    }
}

#[test]
fn crop_contract_refuses_non_encodable_or_out_of_bounds_geometry_without_clamping() {
    assert!(NativeRegionCrop::new(1, 40, 200, 100, 1920, 1080).is_none());
    assert!(NativeRegionCrop::new(20, 40, 199, 100, 1920, 1080).is_none());
    assert!(NativeRegionCrop::new(20, 40, 200, 100, 200, 1080).is_none());
    assert!(NativeMonitorIdentity::new("   ").is_none());
}

#[test]
fn selection_expires_and_purges_without_becoming_not_found_early() {
    let origin = Instant::now();
    let mut entries = registry(2);
    let ticket = entries
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(1))
        .unwrap();

    assert_eq!(entries.active_len(), 1);
    assert!(matches!(
        entries.consume_at(ticket.as_str(), origin + TTL, |_| true),
        Err(RegionSelectionConsumeError::ExpiredSelection)
    ));
    assert_eq!(entries.active_len(), 0);
}

#[test]
fn consume_is_single_use_and_preserves_the_exact_private_value() {
    let origin = Instant::now();
    let mut entries = registry(2);
    let ticket = entries
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(2))
        .unwrap();

    let consumed = entries
        .consume_at(ticket.as_str(), origin, |monitor| {
            monitor.as_str() == "monitor:exact-a"
        })
        .unwrap();
    assert_eq!(consumed.monitor_identity().as_str(), "monitor:exact-a");
    assert_eq!(
        consumed.crop().native_parts(),
        (20, 40, 200, 100, 1920, 1080)
    );
    assert!(matches!(
        entries.consume_at(ticket.as_str(), origin, |_| true),
        Err(RegionSelectionConsumeError::ConsumedSelection)
    ));
}

#[test]
fn malformed_unknown_and_replaced_monitor_are_distinct_and_never_fallback() {
    let origin = Instant::now();
    let mut entries = registry(2);
    let ticket = entries
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(3))
        .unwrap();
    let mut observed = Vec::new();
    assert!(matches!(
        entries.consume_at(ticket.as_str(), origin, |monitor| {
            observed.push(monitor.as_str().to_owned());
            false
        }),
        Err(RegionSelectionConsumeError::MonitorNotFound)
    ));
    assert_eq!(observed, ["monitor:exact-a"]);
    assert!(matches!(
        entries.consume_at(ticket.as_str(), origin, |_| true),
        Err(RegionSelectionConsumeError::ConsumedSelection)
    ));
    assert!(matches!(
        entries.consume_at("not-a-region-token", origin, |_| true),
        Err(RegionSelectionConsumeError::MalformedSelectionId)
    ));
    let unknown = format!("region_cap_{}", "a".repeat(64));
    assert!(matches!(
        entries.consume_at(&unknown, origin, |_| true),
        Err(RegionSelectionConsumeError::SelectionNotFound)
    ));
}

#[test]
fn collision_and_capacity_refuse_without_replacing_live_selection() {
    let origin = Instant::now();
    let mut entropy = registry(2);
    assert!(matches!(
        entropy.issue_at_with_random(selection("monitor:exact-a"), origin, |_| {
            Err(RegionSelectionIssueError::EntropyUnavailable)
        }),
        Err(RegionSelectionIssueError::EntropyUnavailable)
    ));
    assert_eq!(entropy.active_len(), 0);

    let mut collision = registry(2);
    collision
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(4))
        .unwrap();
    assert!(matches!(
        collision.issue_at_with_random(selection("monitor:exact-b"), origin, fill(4)),
        Err(RegionSelectionIssueError::TokenCollision)
    ));
    assert_eq!(collision.active_len(), 1);

    let mut capacity = registry(1);
    capacity
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(5))
        .unwrap();
    assert!(matches!(
        capacity.issue_at_with_random(selection("monitor:exact-b"), origin, fill(6)),
        Err(RegionSelectionIssueError::CapacityExhausted)
    ));
    assert_eq!(capacity.active_len(), 1);
}

#[test]
fn mutex_owned_registry_allows_exactly_one_concurrent_consumer() {
    let origin = Instant::now();
    let mut owner = registry(2);
    let ticket = owner
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(7))
        .unwrap();
    let owner = Arc::new(Mutex::new(owner));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let owner = Arc::clone(&owner);
        let ticket = ticket.clone();
        workers.push(std::thread::spawn(move || {
            owner
                .lock()
                .unwrap()
                .consume_at(ticket.as_str(), origin, |_| true)
        }));
    }
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| {
                matches!(outcome, Err(RegionSelectionConsumeError::ConsumedSelection))
            })
            .count(),
        7
    );
}
