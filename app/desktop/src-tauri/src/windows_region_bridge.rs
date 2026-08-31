//! Private Windows foreground Region bridge.
//!
//! The Win32 overlay runs only on the exact Tauri `main` HWND's owning thread.
//! Its raw DisplayConfig snapshot then crosses once to this shell's spawned
//! cutd child over the secret, epoch, PID, and correlated loopback lane shared
//! with the macOS owner. No public verb or browser-returned region capability
//! exists; the child burns and revalidates this input before the ordinary
//! Windows recorder reserves and starts its exact GPU-cropped WGC capture.

#[cfg(windows)]
use crate::macos_region_bridge::RegionStartOptions;
#[cfg(windows)]
use crate::windows_region_picker::{self, WindowsRegionSelection};
#[cfg(windows)]
use serde::Serialize;
#[cfg(windows)]
use serde_json::{json, Value};

#[cfg(windows)]
#[derive(Serialize)]
struct WireRegionSelection {
    source_gdi: String,
    target_path: String,
    topology_digest: [u8; 32],
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    parent_width: u32,
    parent_height: u32,
}

#[cfg(windows)]
impl WireRegionSelection {
    fn from_native(selection: &WindowsRegionSelection) -> Result<Self, String> {
        let (source_gdi, target_path, topology_digest) = selection.topology_parts()?;
        let (left, top, width, height, parent_width, parent_height) = selection.native_parts();
        Ok(Self {
            source_gdi,
            target_path,
            topology_digest,
            left,
            top,
            width,
            height,
            parent_width,
            parent_height,
        })
    }
}

/// The only concrete Windows foreground call site. The command is private to
/// the selected loopback engine origin and its own spawned child; it is absent
/// from Cut's schema, UI, and static bootstrap capability. The child burns and
/// revalidates the ticket before the ordinary recorder reserves its exact
/// GPU-cropped WGC capture; this command remains absent from product UI,
/// schema, and public capability state.
#[cfg(windows)]
#[tauri::command]
pub(crate) async fn start_windows_region_capture(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::ForegroundRegionBridgeState>,
    start: RegionStartOptions,
) -> Result<Value, String> {
    start.validate()?;
    let bridge = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or_else(|| {
            "Region capture requires this desktop's spawned foreground engine".to_string()
        })?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let picker_app = app.clone();
    app.run_on_main_thread(move || {
        // `choose_region_on_foreground_main_thread` obtains the Tauri `main`
        // HWND in this callback; the native ABI also rejects a cross-thread or
        // merely same-process foreground window before displaying its overlay.
        let _ = sender
            .send(windows_region_picker::choose_region_on_foreground_main_thread(&picker_app));
    })
    .map_err(|_| "could not schedule the foreground Region picker".to_string())?;
    let Some(selection) = receiver
        .await
        .map_err(|_| "the foreground Region picker did not return".to_string())??
    else {
        return Ok(json!({"ok": false, "cancelled": true}));
    };
    // The picker already checks immediately after selection. Check one more
    // time immediately before creating the private child request, so a display
    // replacement during the async handoff never gets a ticket.
    let selection = selection.require_current()?;
    let selection = WireRegionSelection::from_native(&selection)?;
    tauri::async_runtime::spawn_blocking(move || bridge.submit(selection, start))
        .await
        .map_err(|_| "the foreground Region admission worker stopped unexpectedly".to_string())?
}

#[cfg(not(windows))]
#[tauri::command]
pub(crate) async fn start_windows_region_capture() -> Result<serde_json::Value, String> {
    Err("Windows Region capture is unavailable on this platform".to_string())
}
