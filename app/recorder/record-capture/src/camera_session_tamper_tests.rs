//! Tampered camera-finalization evidence stays isolated from ordinary session tests.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use record_core::{CameraMediaFacts, CameraTerminalState, Result};

use crate::camera_session::{
    CameraMediaSeal, CameraSession, CameraSessionBackend, CameraStopOutcome,
};
use crate::camera_session_test_support::observe;
use crate::{CameraDevice, CameraReadiness, CameraRequest, CameraUseIntent, CaptureClock};

#[derive(Debug)]
struct TamperedCamera(CameraMediaSeal);

impl CameraSessionBackend for TamperedCamera {
    fn readiness(&self, _request: &CameraRequest) -> CameraReadiness {
        CameraReadiness::Ready {
            device: CameraDevice {
                id: "fixture_camera".into(),
                label: "Fixture camera".into(),
            },
            detail: "deterministic fixture is ready".into(),
        }
    }

    fn start(&mut self, _intent: &CameraUseIntent, _screen_origin: Instant) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self, _terminal: CameraTerminalState) -> Result<CameraStopOutcome> {
        Ok(CameraStopOutcome::Sealed(self.0.clone()))
    }
}

fn tampered_seal(duration_ms: u64, sha256: String) -> CameraMediaSeal {
    CameraMediaSeal::fixture_unchecked(
        "camera_01".into(),
        "camera/camera.mp4".into(),
        CameraMediaFacts {
            width: 640,
            height: 480,
            fps_num: 30,
            fps_den: 1,
            frame_count: 30,
            duration_ms,
            sha256,
        },
        64,
    )
}

fn session(seal: CameraMediaSeal) -> (CameraSession<TamperedCamera>, Instant) {
    let clock = CaptureClock::new();
    let origin = clock.start();
    let stop = AtomicBool::new(false);
    let request = CameraRequest {
        capture_id: "cap_01".into(),
        device_id: "fixture_camera".into(),
    };
    let intent = CameraUseIntent::from_explicit_user_action(request).unwrap();
    (
        CameraSession::start(TamperedCamera(seal), intent, &clock, &stop).unwrap(),
        origin,
    )
}

#[test]
fn seal_refuses_tampered_hash_or_timing_facts() {
    let (mut hash_session, origin) = session(tampered_seal(1_000, "A".repeat(64)));
    observe(&mut hash_session, origin, 250, 300);
    observe(&mut hash_session, origin, 1_200, 1_250);
    hash_session.stop(CameraTerminalState::Complete).unwrap();
    assert!(hash_session.seal().is_err());

    let (mut timing_session, origin) = session(tampered_seal(999, "a".repeat(64)));
    observe(&mut timing_session, origin, 250, 300);
    observe(&mut timing_session, origin, 1_200, 1_250);
    assert!(timing_session.stop(CameraTerminalState::Complete).is_err());
}
