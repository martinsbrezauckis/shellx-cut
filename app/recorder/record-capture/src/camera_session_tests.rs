use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use record_core::{CameraMediaFacts, CameraTerminalState, Result};
use record_recovery::{
    CaptureRoot, CheckpointSequenceRange, DurableStateTransition, RecordingSessionIntent,
    RecordingSessionJournalFile, RecordingSessionState, RecordingStream, SealedRun,
    SessionTerminal, StreamFragment, StreamFragmentFacts, TerminalDisposition,
};

use crate::camera_session::{
    CameraMediaSeal, CameraSession, CameraSessionBackend, CameraStopOutcome,
};
use crate::camera_session_test_support::observe;
use crate::{CameraDevice, CameraReadiness, CameraRequest, CameraUseIntent, CaptureClock};

#[derive(Debug, Clone)]
struct FixtureCamera {
    readiness: CameraReadiness,
    stop_outcome: CameraStopOutcome,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl CameraSessionBackend for FixtureCamera {
    fn readiness(&self, _request: &CameraRequest) -> CameraReadiness {
        self.readiness.clone()
    }

    fn start(&mut self, _intent: &CameraUseIntent, _screen_origin: Instant) -> Result<()> {
        self.calls.lock().unwrap().push("start");
        Ok(())
    }

    fn stop(&mut self, _terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        self.calls.lock().unwrap().push("stop");
        Ok(self.stop_outcome.clone())
    }
}

fn request() -> CameraRequest {
    CameraRequest {
        capture_id: "cap_01".into(),
        device_id: "fixture_camera".into(),
    }
}

fn intent() -> CameraUseIntent {
    CameraUseIntent::from_explicit_user_action(request()).unwrap()
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

fn seal(duration_ms: u64, sha256: String) -> CameraMediaSeal {
    CameraMediaSeal::fixture(
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
    .unwrap()
}

fn backend(readiness: CameraReadiness, seal: CameraMediaSeal) -> FixtureCamera {
    FixtureCamera {
        readiness,
        stop_outcome: CameraStopOutcome::Sealed(seal),
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn no_media_backend(readiness: CameraReadiness) -> FixtureCamera {
    FixtureCamera {
        readiness,
        stop_outcome: CameraStopOutcome::NoMedia,
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

type StartedSession = (
    CameraSession<FixtureCamera>,
    Instant,
    CaptureClock,
    AtomicBool,
);

fn started_session(backend: FixtureCamera) -> StartedSession {
    let clock = CaptureClock::new();
    let origin = clock.start();
    let stop = AtomicBool::new(false);
    let session = CameraSession::start(backend, intent(), &clock, &stop).unwrap();
    (session, origin, clock, stop)
}

#[test]
fn delayed_first_frame_uses_only_actual_capture_clock_offsets() {
    let fixture = backend(ready(), seal(1_000, "a".repeat(64)));
    let calls = fixture.calls.clone();
    let (mut session, origin, _, _) = started_session(fixture);

    observe(&mut session, origin, 250, 300);
    observe(&mut session, origin, 1_200, 1_250);
    session.stop(CameraTerminalState::Complete).unwrap();
    let evidence = session.seal().unwrap();

    assert_eq!(evidence.artifact().clock.first_frame_offset_ms, 250);
    assert_eq!(evidence.artifact().clock.end_frame_offset_ms, 1_250);
    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}

#[test]
fn pre_start_probe_failure_refuses_session_start() {
    let fixture = backend(
        CameraReadiness::Missing {
            detail: "fixture camera is absent".into(),
        },
        seal(1_000, "a".repeat(64)),
    );
    let calls = fixture.calls.clone();
    let clock = CaptureClock::new();
    let stop = AtomicBool::new(false);

    let error = CameraSession::start(fixture, intent(), &clock, &stop).unwrap_err();

    assert_eq!(error.code, "capture");
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn cancellation_before_clock_open_leaves_camera_unstarted() {
    let fixture = backend(ready(), seal(1_000, "a".repeat(64)));
    let calls = fixture.calls.clone();
    let clock = CaptureClock::new();
    let stop = AtomicBool::new(true);

    let error = CameraSession::start(fixture, intent(), &clock, &stop).unwrap_err();

    assert_eq!(error.code, "capture");
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn camera_waits_for_the_screen_owned_clock_before_backend_start() {
    let fixture = backend(ready(), seal(1_000, "a".repeat(64)));
    let calls = fixture.calls.clone();
    let clock = CaptureClock::new();
    let waiter_clock = clock.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let waiter_stop = stop.clone();
    let (sent, received) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        sent.send(CameraSession::start(
            fixture,
            intent(),
            &waiter_clock,
            &waiter_stop,
        ))
        .unwrap();
    });

    assert!(received.recv_timeout(Duration::from_millis(20)).is_err());
    assert!(calls.lock().unwrap().is_empty());
    let origin = clock.start();
    let session = received
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    assert_eq!(clock.wait_started(&stop), Some(origin));
    assert_eq!(calls.lock().unwrap().as_slice(), ["start"]);
    drop(session);
    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
    waiter.join().unwrap();
}

#[test]
fn device_loss_seals_only_the_independent_valid_camera_prefix() {
    let fixture = backend(ready(), seal(1_000, "a".repeat(64)));
    let (mut session, origin, clock, stop) = started_session(fixture);
    observe(&mut session, origin, 250, 300);
    observe(&mut session, origin, 1_200, 1_250);
    session.stop(CameraTerminalState::DeviceLost).unwrap();
    let evidence = session.seal().unwrap();

    assert_eq!(
        evidence.artifact().terminal_state,
        CameraTerminalState::DeviceLost
    );
    assert_eq!(clock.wait_started(&stop), Some(origin));
    assert!(!stop.load(Ordering::Relaxed));
    let fragment = evidence.stream_fragment(0, 1_250, 0).unwrap();
    assert_eq!(fragment.stream, RecordingStream::CameraVideo);
    assert_eq!(fragment.facts.start_offset_ms, 250);
    assert_eq!(fragment.facts.end_offset_ms, 1_250);
}

#[test]
fn zero_frame_cancellation_stops_but_never_invents_evidence() {
    let fixture = no_media_backend(ready());
    let calls = fixture.calls.clone();
    let (mut session, _, _, _) = started_session(fixture);

    session.stop(CameraTerminalState::Cancelled).unwrap();
    assert!(session.seal().is_err());
    assert_eq!(calls.lock().unwrap().as_slice(), ["start", "stop"]);
}

#[test]
fn typed_camera_evidence_replays_durably_and_torn_tail_fails_closed() {
    let (mut session, origin, _, _) =
        started_session(backend(ready(), seal(1_000, "a".repeat(64))));
    observe(&mut session, origin, 250, 300);
    observe(&mut session, origin, 1_200, 1_250);
    session.stop(CameraTerminalState::DeviceLost).unwrap();
    let camera = session
        .seal()
        .unwrap()
        .stream_fragment(0, 1_250, 0)
        .unwrap();

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project.cutproj");
    fs::create_dir(&project).unwrap();
    let root = CaptureRoot::for_project(&project).unwrap();
    root.create_capture_dir("cap_01").unwrap();
    let intent = RecordingSessionIntent::new(
        "session_01",
        100,
        100,
        30.0,
        None,
        false,
        "opaque-target",
        vec![RecordingStream::ScreenVideo, RecordingStream::CameraVideo],
    );
    let mut journal = RecordingSessionJournalFile::create_new(&root, "cap_01", intent).unwrap();
    journal
        .append_transition(DurableStateTransition {
            sequence: 0,
            state: RecordingSessionState::Started,
            logical_offset_ms: 0,
            observed_unix_ms: 100,
        })
        .unwrap();
    journal
        .seal_run(SealedRun {
            sequence: 0,
            observed_start_ms: 100,
            observed_end_ms: 1_350,
            logical_start_ms: 0,
            logical_end_ms: 1_250,
            checkpoints: CheckpointSequenceRange { first: 0, last: 0 },
            fragments: vec![screen_fragment(), camera],
        })
        .unwrap();
    journal
        .append_transition(DurableStateTransition {
            sequence: 1,
            state: RecordingSessionState::Stopping,
            logical_offset_ms: 1_250,
            observed_unix_ms: 1_400,
        })
        .unwrap();
    journal
        .seal_terminal(SessionTerminal {
            disposition: TerminalDisposition::Interrupted,
            logical_end_ms: 1_250,
            observed_unix_ms: 1_500,
        })
        .unwrap();
    let path = journal.path().to_path_buf();
    let replay = RecordingSessionJournalFile::replay(&root, "cap_01").unwrap();
    assert_eq!(
        replay.sealed_runs()[0].fragments[1].stream,
        RecordingStream::CameraVideo
    );

    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"kind\":")
        .unwrap();
    let before = fs::read(&path).unwrap();
    assert!(RecordingSessionJournalFile::replay(&root, "cap_01").is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

fn screen_fragment() -> StreamFragment {
    StreamFragment {
        stream: RecordingStream::ScreenVideo,
        checkpoint_sequence: Some(0),
        stream_sequence: 0,
        artifact: "screen/video-0.mp4".into(),
        bytes: 64,
        sha256: "b".repeat(64),
        facts: StreamFragmentFacts {
            start_offset_ms: 0,
            end_offset_ms: 1_250,
            media_duration_ms: 1_250,
            decoded_video_frames: Some(38),
            avg_frame_rate: None,
            r_frame_rate: None,
        },
    }
}
