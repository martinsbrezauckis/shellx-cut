//! Process-local ownership for the one active native capture session.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use cut_core::{error_codes, CutError};

use super::capture_session_control::CaptureSessionControl;

static CAPTURE_SESSIONS: OnceLock<Mutex<HashMap<String, CaptureSessionControl>>> = OnceLock::new();

pub(super) fn capture_sessions() -> &'static Mutex<HashMap<String, CaptureSessionControl>> {
    CAPTURE_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) struct CaptureReservation {
    capture_id: String,
}

impl Drop for CaptureReservation {
    fn drop(&mut self) {
        release_capture(&self.capture_id);
    }
}

pub(super) fn reserve_capture(
    capture_id: String,
    control: CaptureSessionControl,
) -> Result<CaptureReservation, CutError> {
    let mut map = capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(active_id) = map.keys().next() {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "a screen recording is already active",
            format!("capture {active_id} still owns the recording devices"),
        )
        .with_suggested_action(
            "stop the active recording and wait for it to finalize before starting another",
        ));
    }
    map.insert(capture_id.clone(), control);
    Ok(CaptureReservation { capture_id })
}

fn release_capture(capture_id: &str) {
    capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(capture_id);
}

/// Signal one active capture to terminate while its reservation remains owned
/// through artifact and receipt finalization.
pub fn stop_capture(capture_id: &str) -> bool {
    let control = capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(capture_id)
        .cloned();
    if let Some(control) = control {
        control.terminalize();
        true
    } else {
        false
    }
}
