//! Shell-owned recording indication for the selected Cut webview.
//!
//! The app-lifetime controller calls `begin_recording_indicator` only after a
//! native Start acknowledges a capture ID, and `end_recording_indicator` only
//! after that same native capture is confirmed terminal. Stop requests and
//! timeouts must not call the latter. The shell keeps the ID across webview
//! remounts and refuses an older completion from clearing a newer capture.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
struct IndicatorOwner(Option<String>);

impl IndicatorOwner {
    fn begin(&mut self, capture_id: &str) -> Result<(), String> {
        if let Some(owner) = self.0.as_deref() {
            if owner != capture_id {
                return Err(format!(
                    "recording indicator still belongs to capture {owner}"
                ));
            }
        } else {
            self.0 = Some(capture_id.to_string());
        }
        Ok(())
    }

    fn is_current(&self, capture_id: &str) -> bool {
        self.0.as_deref() == Some(capture_id)
    }

    /// The native terminal acknowledgement releases ownership even if the OS
    /// fails to remove its decoration. That failure is reported separately.
    fn end_with(
        &mut self,
        capture_id: &str,
        clear: impl FnOnce() -> Result<(), String>,
    ) -> Option<Result<(), String>> {
        if !self.is_current(capture_id) {
            return None;
        }
        let result = clear();
        self.0 = None;
        Some(result)
    }
}

#[derive(Default)]
pub struct RecordIndicatorState(Mutex<IndicatorOwner>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordIndicatorResult {
    pub active_capture_id: Option<String>,
    pub applied: bool,
    pub reason: Option<String>,
}

fn valid_capture_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

// A small antialiased red circle, transparent everywhere else. Windows draws
// this over the existing taskbar icon; no packaged icon or image decoder needed.
#[cfg(windows)]
fn recording_dot() -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - SIZE as f32 / 2.0;
            let dy = y as f32 + 0.5 - SIZE as f32 / 2.0;
            let alpha = (13.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[222, 34, 45, (alpha * 255.0) as u8]);
        }
    }
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

fn apply(app: &AppHandle, active: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Cut main window is unavailable".to_string())?;
    #[cfg(windows)]
    {
        let icon = active.then(recording_dot);
        window
            .set_overlay_icon(icon)
            .map_err(|error| format!("Windows taskbar recording overlay failed: {error}"))
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        window
            .set_badge_count(active.then_some(1))
            .map_err(|error| format!("Dock recording badge failed: {error}"))
    }
}

/// Idempotent for the current capture. A second capture cannot replace an
/// unresolved native owner, even when setting the OS indicator failed.
#[tauri::command]
pub fn begin_recording_indicator(
    app: AppHandle,
    state: State<'_, RecordIndicatorState>,
    capture_id: String,
) -> Result<RecordIndicatorResult, String> {
    if !valid_capture_id(&capture_id) {
        return Err("recording indicator requires a valid capture ID".to_string());
    }
    let mut active = state
        .0
        .lock()
        .map_err(|_| "recording indicator state unavailable")?;
    active.begin(&capture_id)?;
    let reason = apply(&app, true).err();
    Ok(RecordIndicatorResult {
        active_capture_id: Some(capture_id),
        applied: reason.is_none(),
        reason,
    })
}

/// Called only after native Stop/status confirms that this exact capture ended.
/// A stale completion is a no-op and cannot remove a newer capture's indicator.
#[tauri::command]
pub fn end_recording_indicator(
    app: AppHandle,
    state: State<'_, RecordIndicatorState>,
    capture_id: String,
) -> Result<RecordIndicatorResult, String> {
    if !valid_capture_id(&capture_id) {
        return Err("recording indicator requires a valid capture ID".to_string());
    }
    let mut active = state
        .0
        .lock()
        .map_err(|_| "recording indicator state unavailable")?;
    let result = active.end_with(&capture_id, || apply(&app, false));
    let Some(result) = result else {
        return Ok(RecordIndicatorResult {
            active_capture_id: active.0.clone(),
            applied: false,
            reason: Some("capture ID is not the current indicator owner".to_string()),
        });
    };
    let reason = result.err();
    if reason.is_some() {
        retry_clear_later(app);
    }
    Ok(RecordIndicatorResult {
        active_capture_id: None,
        applied: reason.is_none(),
        reason,
    })
}

// One bounded retry handles a transient desktop failure. The ownership check
// is repeated under the mutex so an old retry cannot erase a new take's badge.
fn retry_clear_later(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let Some(state) = app.try_state::<RecordIndicatorState>() else {
            return;
        };
        let Ok(owner) = state.0.lock() else {
            return;
        };
        if owner.0.is_none() {
            if let Err(error) = apply(&app, false) {
                eprintln!("[shellx-cut] recording indicator clear retry failed: {error}");
            }
        }
    });
}

/// Reset on launch and best effort when the native window or app is closing.
/// The in-memory owner never survives a process exit.
pub fn reset(app: &AppHandle) {
    if let Err(error) = apply(app, false) {
        eprintln!("[shellx-cut] could not reset recording indicator: {error}");
    }
    if let Some(state) = app.try_state::<RecordIndicatorState>() {
        if let Ok(mut active) = state.0.lock() {
            active.0 = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{valid_capture_id, IndicatorOwner};

    #[test]
    fn rejects_empty_control_and_unbounded_capture_ids() {
        assert!(valid_capture_id("capture-123"));
        assert!(!valid_capture_id("  "));
        assert!(!valid_capture_id("capture\n123"));
        assert!(!valid_capture_id(&"a".repeat(257)));
    }

    #[test]
    fn stale_stop_cannot_clear_a_newer_capture() {
        let mut owner = IndicatorOwner::default();
        owner.begin("first").unwrap();
        assert!(owner.is_current("first"));
        assert!(owner.end_with("first", || Ok(())).unwrap().is_ok());
        owner.begin("second").unwrap();
        assert!(!owner.is_current("first"));
        assert!(owner.is_current("second"));
        assert!(owner
            .end_with("first", || panic!("stale clear called"))
            .is_none());
        assert!(owner.begin("third").is_err());
        assert!(owner.is_current("second"));
    }

    #[test]
    fn failed_os_clear_releases_terminal_capture_ownership() {
        let mut owner = IndicatorOwner::default();
        owner.begin("first").unwrap();
        assert!(owner
            .end_with("first", || Err("badge unsupported".to_string()))
            .unwrap()
            .is_err());
        assert!(owner.0.is_none());
        owner.begin("second").unwrap();
        assert!(owner.is_current("second"));
    }
}
