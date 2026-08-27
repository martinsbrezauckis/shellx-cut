use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use record_core::{CameraMediaFacts, CameraTerminalState, RecordError, Result};

use crate::camera_session::{
    CameraMediaSeal, CameraSession, CameraSessionBackend, CameraStopOutcome,
};
use crate::{CameraDevice, CameraReadiness, CameraRequest, CaptureClock};

#[derive(Debug, Clone)]
struct LifecycleFixture {
    readiness: CameraReadiness,
    start_result: std::result::Result<(), RecordError>,
    stop_result: std::result::Result<CameraStopOutcome, RecordError>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl CameraSessionBackend for LifecycleFixture {
    fn readiness(&self, _request: &CameraRequest) -> CameraReadiness {
        self.readiness.clone()
    }

    fn start(&mut self, _request: &CameraRequest, _screen_origin: Instant) -> Result<()> {
        self.calls.lock().unwrap().push("start");
        self.start_result.clone()
    }

    fn stop(&mut self, _terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        self.calls.lock().unwrap().push("stop");
        self.stop_result.clone()
    }
}

fn fixture(stop_result: std::result::Result<CameraStopOutcome, RecordError>) -> LifecycleFixture {
    configured_fixture(ready(), Ok(()), stop_result)
}

fn configured_fixture(
    readiness: CameraReadiness,
    start_result: std::result::Result<(), RecordError>,
    stop_result: std::result::Result<CameraStopOutcome, RecordError>,
) -> LifecycleFixture {
    LifecycleFixture {
        readiness,
        start_result,
        stop_result,
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn ready() -> CameraReadiness {
    CameraReadiness::Ready {
        device: CameraDevice {
            id: "fixture_camera".into(),
            label: "Fixture camera".into(),
        },
        detail: "deterministic fixture is ready".into(),
    }
}

fn valid_seal() -> CameraMediaSeal {
    CameraMediaSeal {
        artifact_id: "camera_01".into(),
        video: "camera/camera.mp4".into(),
        media: CameraMediaFacts {
            width: 640,
            height: 480,
            fps_num: 30,
            fps_den: 1,
            frame_count: 30,
            duration_ms: 1_000,
            sha256: "a".repeat(64),
        },
        bytes: 64,
    }
}

fn sealed() -> CameraStopOutcome {
    CameraStopOutcome::Sealed(valid_seal())
}

fn request() -> CameraRequest {
    CameraRequest {
        capture_id: "cap_01".into(),
        device_id: "fixture_camera".into(),
    }
}

fn session(fixture: LifecycleFixture) -> (CameraSession<LifecycleFixture>, Instant) {
    let clock = CaptureClock::new();
    let origin = clock.start();
    let stop = AtomicBool::new(false);
    (
        CameraSession::start(fixture, request(), &clock, &stop).unwrap(),
        origin,
    )
}

#[test]
fn drop_accepts_a_live_camera_no_media_terminal_exactly_once() {
    let fixture = fixture(Ok(CameraStopOutcome::NoMedia));
    let calls = fixture.calls.clone();
    let (session, _) = session(fixture);

    drop(session);

    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}

#[test]
fn explicit_stop_then_drop_never_double_stops() {
    let fixture = fixture(Ok(CameraStopOutcome::NoMedia));
    let calls = fixture.calls.clone();
    let (mut session, _) = session(fixture);

    session.stop(CameraTerminalState::Cancelled).unwrap();
    drop(session);

    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}

#[test]
fn stop_failure_is_terminal_and_never_retries_backend_stop() {
    let fixture = fixture(Err(RecordError::new(
        "capture",
        "fixture stop failed",
        "test injected backend failure",
    )));
    let calls = fixture.calls.clone();
    let (mut session, _) = session(fixture);

    assert_eq!(
        session
            .stop(CameraTerminalState::DeviceLost)
            .unwrap_err()
            .message,
        "fixture stop failed"
    );
    assert!(session.stop(CameraTerminalState::DeviceLost).is_err());
    assert!(session.seal().is_err());
    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}

#[test]
fn rejects_camera_prefix_that_exceeds_its_containing_physical_run() {
    let (mut session, origin) = session(fixture(Ok(sealed())));
    session
        .observe_frame_at(origin + Duration::from_millis(250))
        .unwrap();
    session
        .observe_frame_at(origin + Duration::from_millis(1_250))
        .unwrap();
    session.stop(CameraTerminalState::DeviceLost).unwrap();
    let evidence = session.seal().unwrap();

    assert!(evidence.stream_fragment(0, 1_000, 0).is_err());
}

#[test]
fn ready_probe_for_a_different_device_is_refused_before_start_or_wait() {
    let fixture = configured_fixture(
        CameraReadiness::Ready {
            device: CameraDevice {
                id: "other_camera".into(),
                label: "Other camera".into(),
            },
            detail: "wrong fixture device is ready".into(),
        },
        Ok(()),
        Ok(sealed()),
    );
    let calls = fixture.calls.clone();
    let clock = CaptureClock::new();
    let stop = AtomicBool::new(true);

    let error = CameraSession::start(fixture, request(), &clock, &stop).unwrap_err();

    assert_eq!(
        error.message,
        "selected camera probe returned a different device"
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn sealed_outcome_with_zero_bytes_is_terminal_but_refuses_evidence() {
    let mut invalid_seal = valid_seal();
    invalid_seal.bytes = 0;
    let fixture = fixture(Ok(CameraStopOutcome::Sealed(invalid_seal)));
    let calls = fixture.calls.clone();
    let (mut session, _) = session(fixture);

    assert!(session.stop(CameraTerminalState::Cancelled).is_err());
    assert!(session.seal().is_err());
    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}

#[test]
fn start_error_is_preserved_after_one_cancelled_cleanup_stop() {
    let fixture = configured_fixture(
        ready(),
        Err(RecordError::new(
            "capture",
            "fixture start failed",
            "test injected start failure",
        )),
        Err(RecordError::new(
            "capture",
            "fixture cleanup failed",
            "test injected cleanup failure",
        )),
    );
    let calls = fixture.calls.clone();
    let clock = CaptureClock::new();
    clock.start();
    let stop = AtomicBool::new(false);

    let error = CameraSession::start(fixture, request(), &clock, &stop).unwrap_err();

    assert_eq!(error.message, "fixture start failed");
    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}
