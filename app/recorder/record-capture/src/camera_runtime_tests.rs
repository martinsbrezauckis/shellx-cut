use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use record_core::{CameraMediaFacts, CameraTerminalState, RecordError, Result};

use crate::camera_runtime::{
    CameraEnumeration, CameraRuntime, CameraRuntimeAdapter, CameraStopResult, CameraUseOutcome,
};
use crate::camera_session::{CameraMediaSeal, CameraSessionBackend, CameraStopOutcome};
use crate::{
    CameraDevice, CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent,
    CaptureClock,
};

#[derive(Clone)]
struct FixtureAdapter {
    state: Arc<Mutex<FixtureState>>,
}

struct FixtureState {
    devices: Vec<CameraDevice>,
    readiness: CameraReadiness,
    start_error: Option<String>,
    readiness_after_start_error: Option<CameraReadiness>,
    stop_outcome: CameraStopOutcome,
    calls: Vec<String>,
}

impl FixtureAdapter {
    fn new(readiness: CameraReadiness, stop_outcome: CameraStopOutcome) -> Self {
        Self {
            state: Arc::new(Mutex::new(FixtureState {
                devices: vec![device()],
                readiness,
                start_error: None,
                readiness_after_start_error: None,
                stop_outcome,
                calls: Vec::new(),
            })),
        }
    }

    fn fail_start_as(&self, readiness: CameraReadiness) {
        let mut state = self.state.lock().unwrap();
        state.start_error = Some("native prompt refused".into());
        state.readiness_after_start_error = Some(readiness);
    }

    fn set_readiness(&self, readiness: CameraReadiness) {
        self.state.lock().unwrap().readiness = readiness;
    }

    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl CameraRuntimeAdapter for FixtureAdapter {
    fn enumerate(&self) -> Result<Vec<CameraDevice>> {
        let mut state = self.state.lock().unwrap();
        state.calls.push("enumerate".into());
        Ok(state.devices.clone())
    }
}

impl CameraSessionBackend for FixtureAdapter {
    fn readiness(&self, _request: &CameraRequest) -> CameraReadiness {
        self.state.lock().unwrap().readiness.clone()
    }

    fn start(&mut self, _intent: &CameraUseIntent, _screen_origin: Instant) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.calls.push("start".into());
        if let Some(readiness) = state.readiness_after_start_error.take() {
            state.readiness = readiness;
        }
        if let Some(message) = state.start_error.take() {
            return Err(RecordError::new(
                "capture",
                message,
                "fixture native prompt refusal",
            ));
        }
        Ok(())
    }

    fn stop(&mut self, terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(format!("stop:{terminal_state:?}"));
        Ok(state.stop_outcome.clone())
    }
}

fn device() -> CameraDevice {
    CameraDevice {
        id: "camera_01".into(),
        label: "Fixture camera".into(),
    }
}

fn request(capture_id: &str) -> CameraRequest {
    CameraRequest {
        capture_id: capture_id.into(),
        device_id: "camera_01".into(),
    }
}

fn intent(capture_id: &str) -> CameraUseIntent {
    CameraUseIntent::from_explicit_user_action(request(capture_id)).unwrap()
}

fn ready() -> CameraReadiness {
    CameraReadiness::Ready {
        device: device(),
        detail: "first native frame probe succeeded".into(),
    }
}

fn permission_required() -> CameraReadiness {
    CameraReadiness::PermissionRequired {
        device: device(),
        detail: "explicit use may request camera permission".into(),
    }
}

fn sealed() -> CameraStopOutcome {
    CameraStopOutcome::Sealed(
        CameraMediaSeal::fixture(
            "camera_artifact_01".into(),
            "camera/camera.mp4".into(),
            CameraMediaFacts {
                width: 640,
                height: 480,
                fps_num: 30,
                fps_den: 1,
                frame_count: 30,
                duration_ms: 1_000,
                sha256: "a".repeat(64),
            },
            64,
        )
        .unwrap(),
    )
}

#[test]
fn unavailable_registry_does_not_fabricate_devices_or_permission_access() {
    let mut runtime = CameraRuntime::unavailable_for_build();
    let enumeration = runtime.enumerate().unwrap();
    assert!(matches!(enumeration, CameraEnumeration::Unavailable { .. }));
    assert!(matches!(
        runtime.readiness(&request("cap_01")),
        CameraReadiness::Missing { .. }
    ));

    let clock = CaptureClock::new();
    clock.start();
    let stop = AtomicBool::new(false);
    assert!(matches!(
        runtime.use_camera(intent("cap_01"), &clock, &stop).unwrap(),
        CameraUseOutcome::Unavailable { .. }
    ));
}

#[test]
fn enumeration_and_non_startable_states_never_call_permission_capable_start() {
    for readiness in [
        CameraReadiness::Enumerated {
            device: device(),
            detail: "enumeration is not readiness".into(),
        },
        CameraReadiness::PermissionDenied {
            device: device(),
            detail: "previous prompt was denied".into(),
        },
        CameraReadiness::Busy {
            detail: "another app owns the camera".into(),
        },
        CameraReadiness::NoFrame {
            device: device(),
            detail: "native start delivered no frames".into(),
        },
    ] {
        let fixture = FixtureAdapter::new(readiness.clone(), sealed());
        let mut runtime = CameraRuntime::with_adapter(Box::new(fixture.clone()));
        assert!(matches!(
            runtime.enumerate().unwrap(),
            CameraEnumeration::Devices(_)
        ));
        let clock = CaptureClock::new();
        clock.start();
        let stop = AtomicBool::new(false);

        assert_eq!(
            runtime.use_camera(intent("cap_01"), &clock, &stop).unwrap(),
            CameraUseOutcome::Refused(readiness)
        );
        assert_eq!(fixture.calls(), ["enumerate"]);
    }
}

#[test]
fn explicit_use_owns_start_stop_and_seals_the_exact_camera_prefix() {
    let fixture = FixtureAdapter::new(permission_required(), sealed());
    let mut runtime = CameraRuntime::with_adapter(Box::new(fixture.clone()));
    let clock = CaptureClock::new();
    let origin = clock.start();
    let stop = AtomicBool::new(false);

    assert_eq!(
        runtime.use_camera(intent("cap_01"), &clock, &stop).unwrap(),
        CameraUseOutcome::Started
    );
    assert_eq!(fixture.calls(), ["start"]);
    assert!(matches!(
        runtime.use_camera(intent("cap_02"), &clock, &stop).unwrap(),
        CameraUseOutcome::Refused(CameraReadiness::Busy { .. })
    ));
    assert_eq!(fixture.calls(), ["start"]);

    assert!(runtime
        .stop_for_screen_owner("cap_02", CameraTerminalState::Complete)
        .is_err());
    assert_eq!(fixture.calls(), ["start"]);
    runtime
        .observe_frame(
            "cap_01",
            CameraFrameObservation::new(
                origin + Duration::from_millis(250),
                origin + Duration::from_millis(300),
            ),
        )
        .unwrap();
    runtime
        .observe_frame(
            "cap_01",
            CameraFrameObservation::new(
                origin + Duration::from_millis(1_200),
                origin + Duration::from_millis(1_250),
            ),
        )
        .unwrap();

    let CameraStopResult::Sealed(evidence) = runtime
        .stop_for_screen_owner("cap_01", CameraTerminalState::Complete)
        .unwrap()
    else {
        panic!("camera frames must seal one independent artifact")
    };
    assert_eq!(evidence.artifact().capture_id, "cap_01");
    assert_eq!(evidence.artifact().clock.first_frame_offset_ms, 250);
    assert_eq!(evidence.artifact().clock.end_frame_offset_ms, 1_250);
    assert_eq!(evidence.artifact().media.sha256, "a".repeat(64));
    assert_eq!(
        fixture.calls(),
        ["start", "stop:Complete"],
        "the matching screen owner performs exactly one Stop"
    );
}

#[test]
fn start_failure_recovers_a_typed_permission_denial_for_a_later_explicit_retry() {
    let fixture = FixtureAdapter::new(permission_required(), sealed());
    fixture.fail_start_as(CameraReadiness::PermissionDenied {
        device: device(),
        detail: "system permission was denied".into(),
    });
    let mut runtime = CameraRuntime::with_adapter(Box::new(fixture.clone()));
    let clock = CaptureClock::new();
    clock.start();
    let stop = AtomicBool::new(false);

    assert!(matches!(
        runtime.use_camera(intent("cap_01"), &clock, &stop).unwrap(),
        CameraUseOutcome::Refused(CameraReadiness::PermissionDenied { .. })
    ));
    assert_eq!(fixture.calls(), ["start", "stop:Cancelled"]);

    fixture.set_readiness(ready());
    assert_eq!(
        runtime.use_camera(intent("cap_01"), &clock, &stop).unwrap(),
        CameraUseOutcome::Started
    );
    assert_eq!(fixture.calls(), ["start", "stop:Cancelled", "start"]);
}

#[test]
fn zero_frame_stop_has_no_artifact_but_returns_the_adapter_for_a_new_explicit_use() {
    let fixture = FixtureAdapter::new(ready(), CameraStopOutcome::NoMedia);
    let mut runtime = CameraRuntime::with_adapter(Box::new(fixture.clone()));
    let clock = CaptureClock::new();
    clock.start();
    let stop = AtomicBool::new(false);

    assert_eq!(
        runtime.use_camera(intent("cap_01"), &clock, &stop).unwrap(),
        CameraUseOutcome::Started
    );
    assert_eq!(
        runtime
            .stop_for_screen_owner("cap_01", CameraTerminalState::Cancelled)
            .unwrap(),
        CameraStopResult::NoMedia
    );
    assert_eq!(
        runtime.use_camera(intent("cap_02"), &clock, &stop).unwrap(),
        CameraUseOutcome::Started
    );
    assert_eq!(
        fixture.calls(),
        ["start", "stop:Cancelled", "start"],
        "zero-frame completion is truthful and does not orphan the adapter"
    );
}
