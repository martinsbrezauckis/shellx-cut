//! Exact ScreenCaptureKit display identities for capture admission and region work.
//!
//! This module intentionally accepts only ScreenCaptureKit's live display id.
//! It has no title, ordinal, primary-display, or geometry fallback.

use screencapturekit::prelude::{SCDisplay, SCShareableContent};

use crate::monitor_identity::{opaque_native_monitor_id, resolve_exact, NativeMonitorPlatform};

pub(crate) fn monitor_id(display: &SCDisplay) -> Option<String> {
    native_display_key(display)
        .and_then(|key| opaque_native_monitor_id(NativeMonitorPlatform::Macos, &key))
}

/// Re-enumerate ScreenCaptureKit and return only the live display whose native
/// id exactly matches the opaque selection. A missing or changed display stays
/// unresolved; callers must ask the user to choose again. The Record capture
/// backend shares the same exact matching rule against its fresh enumeration.
#[allow(dead_code)] // Region selection consumes this helper in a later slice.
pub(crate) fn resolve_monitor(id: &str) -> Option<SCDisplay> {
    crate::macos::sck_init_cg();
    let content = SCShareableContent::get().ok()?;
    resolve_exact(
        id,
        NativeMonitorPlatform::Macos,
        content.displays(),
        native_display_key,
    )
}

fn native_display_key(display: &SCDisplay) -> Option<String> {
    let native_id = display.display_id();
    (native_id != 0).then(|| native_id.to_string())
}
