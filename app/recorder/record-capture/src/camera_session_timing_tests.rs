use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use record_core::{CameraMediaFacts, CameraTerminalState, Result};

use crate::camera_session::{
    CameraMediaSeal, CameraSession, CameraSessionBackend, CameraStopOutcome,
};
use crate::camera_session_test_support::observe;
use crate::{
    CameraDevice, CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent,
    CaptureClock,
};

struct TimingFixture;

impl CameraSessionBackend for TimingFixture {
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

    fn stop(&mut self, _terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        Ok(CameraStopOutcome::Sealed(
            CameraMediaSeal::fixture(
                "camera_01".into(),
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
        ))
    }
}

fn session() -> (CameraSession<TimingFixture>, Instant) {
    let clock = CaptureClock::new();
    let origin = clock.start();
    let stop = AtomicBool::new(false);
    let request = CameraRequest {
        capture_id: "cap_01".into(),
        device_id: "fixture_camera".into(),
    };
    let intent = CameraUseIntent::from_explicit_user_action(request).unwrap();
    (
        CameraSession::start(TimingFixture, intent, &clock, &stop).unwrap(),
        origin,
    )
}

#[test]
fn thirty_fps_one_second_uses_the_final_sample_end_not_its_start() {
    let (mut session, origin) = session();

    for sample in 0..30_u64 {
        observe(
            &mut session,
            origin,
            250 + sample * 1_000 / 30,
            250 + (sample + 1) * 1_000 / 30,
        );
    }
    session.stop(CameraTerminalState::Complete).unwrap();
    let evidence = session.seal().unwrap();

    assert_eq!(evidence.artifact().clock.first_frame_offset_ms, 250);
    assert_eq!(evidence.artifact().clock.end_frame_offset_ms, 1_250);
    assert_eq!(evidence.artifact().media.duration_ms, 1_000);
}

#[test]
fn invalid_or_non_monotonic_camera_intervals_are_rejected() {
    let (mut session, origin) = session();

    let zero_length = session.observe_frame(CameraFrameObservation::new(
        origin + Duration::from_millis(250),
        origin + Duration::from_millis(250),
    ));
    assert!(zero_length
        .unwrap_err()
        .message
        .contains("non-empty interval"));

    observe(&mut session, origin, 250, 300);
    let overlapping = session.observe_frame(CameraFrameObservation::new(
        origin + Duration::from_millis(299),
        origin + Duration::from_millis(350),
    ));
    assert!(overlapping.unwrap_err().message.contains("monotonic"));
}
