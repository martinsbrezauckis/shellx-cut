//! Process-local ownership for the one active native capture session.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use cut_core::{error_codes, CutError};

use super::capture_session_control::CaptureSessionControl;

static CAPTURE_SESSIONS: OnceLock<Mutex<HashMap<String, CaptureSessionControl>>> = OnceLock::new();
static CAMERA_OWNERS: OnceLock<Mutex<HashMap<String, CameraOwnerEntry>>> = OnceLock::new();
static CAMERA_OWNER_SEQUENCE: AtomicU64 = AtomicU64::new(1);
// The process-local registry deliberately admits just one capture. Tests that
// exercise an active reservation therefore need one shared gate as well; a
// test-local mutex cannot protect against a reservation created by another
// screen-record test module running in parallel.
#[cfg(test)]
static CAPTURE_TEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn capture_test_lock() -> &'static tokio::sync::Mutex<()> {
    CAPTURE_TEST_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Debug, Clone)]
struct CameraOwnerEntry {
    token: u64,
    reservation: CaptureSessionControl,
}

/// Process-local capability for exactly one private camera owner. It has no
/// Drop release: a native runtime may still need an explicit terminal Stop
/// after ordinary worker cleanup. The owner releases this only after a
/// terminal no-media outcome or a retained sealed placement is durable.
#[derive(Debug)]
pub(super) struct CameraOwnerClaim {
    capture_id: String,
    token: u64,
    released: bool,
}

pub(super) fn capture_sessions() -> &'static Mutex<HashMap<String, CaptureSessionControl>> {
    CAPTURE_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn camera_owners() -> &'static Mutex<HashMap<String, CameraOwnerEntry>> {
    CAMERA_OWNERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Return the exact still-reserved control for one capture. Private sidecars
/// use this to reject a stale capture id rather than borrowing a new capture's
/// Stop signal after a project switch or worker finalization.
#[allow(
    dead_code,
    reason = "the only consumer is the Windows-private camera owner"
)]
pub(super) fn active_capture_control(capture_id: &str) -> Option<CaptureSessionControl> {
    capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(capture_id)
        .cloned()
}

/// Atomically claim the exact active screen reservation before constructing a
/// private native camera runtime. The session lock is acquired before the
/// camera map everywhere, so release/Stop cannot interleave a claim with a
/// different capture reservation.
#[allow(
    dead_code,
    reason = "the only consumer is the Windows-private camera owner"
)]
pub(super) fn reserve_camera_owner(
    capture_id: &str,
) -> Result<(CameraOwnerClaim, CaptureSessionControl), CutError> {
    let sessions = capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let reservation = sessions.get(capture_id).cloned().ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "private camera requires an active screen capture reservation",
            "the exact screen capture must remain reserved before camera use",
        )
    })?;
    let mut owners = camera_owners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if owners.contains_key(capture_id) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "private camera is already owned for this screen capture",
            "wait for the existing camera owner to reach a terminal outcome",
        ));
    }
    let token = CAMERA_OWNER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    owners.insert(
        capture_id.to_owned(),
        CameraOwnerEntry {
            token,
            reservation: reservation.clone(),
        },
    );
    Ok((
        CameraOwnerClaim {
            capture_id: capture_id.to_owned(),
            token,
            released: false,
        },
        reservation,
    ))
}

/// Return the current matching screen control for an unreleased camera claim.
/// `None` distinguishes a stale/cancelled reservation from the normal
/// stop-dominant case, where the exact control remains live but its Stop flag
/// is set.
#[allow(
    dead_code,
    reason = "the only consumer is the Windows-private camera owner"
)]
pub(super) fn active_camera_owner_control(
    claim: &CameraOwnerClaim,
) -> Option<CaptureSessionControl> {
    if claim.released {
        return None;
    }
    let sessions = capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let owners = camera_owners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let owner = owners.get(&claim.capture_id)?;
    let current = sessions.get(&claim.capture_id)?;
    (owner.token == claim.token && owner.reservation.same_reservation(current))
        .then(|| current.clone())
}

/// A sealed artifact may outlive ordinary screen-worker cleanup while its one
/// placement retry is pending. This predicate intentionally does not require
/// a live screen control, but still refuses a released/replaced claim.
#[allow(
    dead_code,
    reason = "the only consumer is the Windows-private camera owner"
)]
pub(super) fn camera_owner_claim_is_held(claim: &CameraOwnerClaim) -> bool {
    !claim.released
        && camera_owners()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&claim.capture_id)
            .is_some_and(|owner| owner.token == claim.token)
}

/// Release an exact claim after a retry-safe terminal outcome. This is
/// idempotent and never removes a newer claim for the same capture id.
#[allow(
    dead_code,
    reason = "the only consumer is the Windows-private camera owner"
)]
pub(super) fn release_camera_owner(claim: &mut CameraOwnerClaim) {
    if claim.released {
        return;
    }
    let mut owners = camera_owners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if owners
        .get(&claim.capture_id)
        .is_some_and(|owner| owner.token == claim.token)
    {
        owners.remove(&claim.capture_id);
    }
    claim.released = true;
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
        let _ = control.terminalize();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reservation owns the devices until worker cleanup. `stop_capture`
    /// terminalizes the control before signaling it, but deliberately retains
    /// the reservation while finalization runs.
    #[tokio::test]
    async fn enforces_single_owner_until_worker_release() {
        let _capture_lock = capture_test_lock().lock().await;
        let id = format!("cap_test_{}", std::process::id());
        let second_id = format!("cap_test_second_{}", std::process::id());
        let control = CaptureSessionControl::new(None, false, false, false);
        let reservation = reserve_capture(id.clone(), control.clone()).unwrap();
        let conflict = reserve_capture(
            second_id.clone(),
            CaptureSessionControl::new(None, false, false, false),
        )
        .err()
        .expect("a second capture must be rejected");
        assert_eq!(conflict.code, error_codes::CONFLICT);

        assert!(!control.status().stop_requested, "stop signal starts unset");
        assert!(stop_capture(&id), "registered control receives Stop");
        assert!(
            control.status().stop_requested,
            "the signal polled by the backend is set"
        );
        assert_eq!(
            control.status().phase,
            record_capture::SessionPhase::Stopped,
            "Stop is terminal before the physical signal"
        );
        assert!(
            stop_capture(&id),
            "repeat Stop remains idempotent during finalization"
        );
        assert!(
            capture_sessions().lock().unwrap().contains_key(&id),
            "ownership remains until worker cleanup"
        );

        drop(reservation);
        assert!(!stop_capture(&id), "worker release removes the reservation");
        let second = reserve_capture(
            second_id,
            CaptureSessionControl::new(None, false, false, false),
        )
        .unwrap();
        drop(second);
    }
}
