use super::region_selection::{
    NativeMonitorIdentity, NativeRegionCrop, NativeTopologyFingerprint,
    RegionSelectionConsumeError, RegionSelectionIssueError, RegionSelectionRegistry,
    RegionSelectionValue,
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

fn topology_selection(monitor: &str, digest: u8) -> RegionSelectionValue {
    RegionSelectionValue::with_topology(
        NativeMonitorIdentity::new(monitor).unwrap(),
        NativeRegionCrop::new(20, 40, 200, 100, 1920, 1080).unwrap(),
        NativeTopologyFingerprint::new([digest; 32]).unwrap(),
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
    assert!(NativeTopologyFingerprint::new([0; 32]).is_none());
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
        .consume_at(ticket.as_str(), origin, |selection| {
            selection.monitor_identity().as_str() == "monitor:exact-a"
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
        entries.consume_at(ticket.as_str(), origin, |selection| {
            observed.push(selection.monitor_identity().as_str().to_owned());
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
fn topology_or_scale_change_burns_the_ticket_before_a_capture_can_reserve() {
    let origin = Instant::now();
    let mut entries = registry(2);
    let ticket = entries
        .issue_at_with_random(selection("monitor:exact-a"), origin, fill(8))
        .unwrap();

    assert!(matches!(
        entries.consume_at(ticket.as_str(), origin, |selection| {
            let (_, _, _, _, parent_width, parent_height) = selection.crop().native_parts();
            (parent_width, parent_height) == (3_840, 2_160)
        }),
        Err(RegionSelectionConsumeError::MonitorNotFound)
    ));
    assert!(matches!(
        entries.consume_at(ticket.as_str(), origin, |_| true),
        Err(RegionSelectionConsumeError::ConsumedSelection)
    ));
}

#[test]
fn equal_sized_windows_display_swap_or_topology_epoch_change_burns_snapshot_ticket() {
    let origin = Instant::now();
    let same_native_dimensions = (1920, 1080);
    for (current_monitor, current_digest) in [("target:physical-b", 7), ("target:physical-a", 8)] {
        let mut entries = registry(2);
        let ticket = entries
            .issue_at_with_random(
                topology_selection("target:physical-a", 7),
                origin,
                fill(current_digest),
            )
            .unwrap();
        assert!(matches!(
            entries.consume_at(ticket.as_str(), origin, |selection| {
                let (_, _, _, _, parent_width, parent_height) = selection.crop().native_parts();
                selection.monitor_identity().as_str() == current_monitor
                    && (parent_width, parent_height) == same_native_dimensions
                    && selection
                        .topology_fingerprint()
                        .is_some_and(|fingerprint| fingerprint.matches(&[current_digest; 32]))
            }),
            Err(RegionSelectionConsumeError::MonitorNotFound)
        ));
        assert!(matches!(
            entries.consume_at(ticket.as_str(), origin, |_| true),
            Err(RegionSelectionConsumeError::ConsumedSelection)
        ));
    }
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

#[test]
fn macos_visual_picker_contract_keeps_native_display_selection_private_and_keyboard_driven() {
    // This host-independent contract locks the macOS-only Objective-C++ seam
    // without pretending a Linux test runner can exercise AppKit. The installed
    // Mac target still needs a real foreground visual-selection qualification.
    let picker = include_str!("macos_region_picker.mm");
    for required in [
        "sxc_macos_region_picker_present",
        "NSPointInRect(point, screen.frame)",
        "convertRectToBacking:_selectedScreen.frame",
        "convertRectToBacking:_selection",
        "parentHeight - (selectedMaxY - parentMinY)",
        "NSEventModifierFlagShift",
        "keyCode == 53",
        "keyCode == 36 || keyCode == 76",
        "stopModalWithCode:SXCNativePickerCancelled",
        "stopModalWithCode:SXCNativePickerPicked",
        "stopModalWithCode:SXCNativePickerRefused",
        "- (void)resignKeyWindow",
        "[super resignKeyWindow]",
        "[NSThread isMainThread]",
        "NSCompositingOperationClear",
    ] {
        assert!(
            picker.contains(required),
            "native picker must retain `{required}`"
        );
    }
    assert!(
        !picker.contains("NSTextField"),
        "the native picker must not provide a typed coordinate entry path"
    );
}

#[test]
fn windows_visual_picker_contract_owns_one_physical_display_and_refuses_focus_or_dpi_drift() {
    // This source-level contract is intentionally host-independent. It proves
    // the native seam stays private and fail-closed on a Linux coordinator;
    // an installed Windows desktop must still qualify actual overlay, WGC crop,
    // mixed-DPI, display-change, and capture/input behavior before exposure.
    let picker = include_str!("windows_region_picker.cpp");
    for required in [
        "sxc_windows_region_picker_present",
        "GetForegroundWindow",
        "GetCurrentProcessId",
        "SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)",
        "SM_XVIRTUALSCREEN",
        "SM_CXVIRTUALSCREEN",
        "MonitorFromPoint(point, MONITOR_DEFAULTTONULL)",
        "clamp_to_monitor",
        "WM_ACTIVATEAPP",
        "WM_DISPLAYCHANGE",
        "WM_DPICHANGED",
        "#include <windowsx.h>",
        "MAKEINTRESOURCEW(32515)",
        "VK_ESCAPE",
        "VK_RETURN",
        "SXCWindowsNativePickerCancelled",
        "SXCWindowsNativePickerRefused",
        "sxc_windows_topology_snapshot_for_monitor",
        "sxc_windows_region_picker_selection_is_current",
        "left = static_cast<uint32_t>(left)",
        "width = static_cast<uint32_t>(right - left)",
    ] {
        assert!(
            picker.contains(required),
            "Windows native picker must retain `{required}`"
        );
    }
    for forbidden in ["GraphicsCapturePicker", "EditText", "WM_SETTEXT"] {
        assert!(
            !picker.contains(forbidden),
            "Windows native picker must not expose `{forbidden}`"
        );
    }

    let topology = include_str!("windows_region_topology.cpp");
    for required in [
        "QueryDisplayConfig",
        "DisplayConfigGetDeviceInfo",
        "BCRYPT_SHA256_ALGORITHM",
        "shellx-cut-windows-topology-v1",
        "source_gdi == selected",
        "target_path",
        "sxc_windows_topology_snapshot_for_gdi",
        "sxc_windows_topology_snapshot_for_monitor",
        "sxc_windows_topology_snapshot_is_current",
        "std::memcmp(snapshot->digest, digest",
    ] {
        assert!(
            topology.contains(required),
            "Windows topology snapshot must retain `{required}`"
        );
    }
}

#[test]
fn windows_region_native_shims_admit_the_registered_xwin_header_pairing() {
    // xwin follows the current MSVC SDK/header set while the central Linux
    // builder can legitimately carry the distribution clang-cl. Both native
    // consumers must opt into that supported header/compiler pairing or a
    // source-green Region change can fail only when the release build starts.
    for (consumer, build_script) in [
        (
            "record-capture",
            include_str!("../../../recorder/record-capture/build.rs"),
        ),
        (
            "desktop",
            include_str!("../../../desktop/src-tauri/build.rs"),
        ),
    ] {
        assert!(
            build_script.contains("_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH"),
            "{consumer} must retain the registered xwin MSVC header admission"
        );
    }
}
