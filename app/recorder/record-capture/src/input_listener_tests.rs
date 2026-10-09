use super::*;
use record_core::{ClickSample, CursorSample, KeySample, MouseButton, ScrollSample};
use std::sync::mpsc;

struct FakeNative {
    calls: Arc<Mutex<Vec<&'static str>>>,
    stop_error: Option<&'static str>,
    wait_error: Option<&'static str>,
    exit: Option<mpsc::Receiver<std::result::Result<(), String>>>,
    wait_started: Option<mpsc::Sender<()>>,
    dropped: Option<mpsc::Sender<()>>,
}

impl Default for FakeNative {
    fn default() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            stop_error: None,
            wait_error: None,
            exit: None,
            wait_started: None,
            dropped: None,
        }
    }
}

impl Drop for FakeNative {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

impl NativeInputListener for FakeNative {
    fn request_stop(&self) -> std::result::Result<(), String> {
        self.calls.lock().unwrap().push("stop");
        self.stop_error
            .map_or(Ok(()), |error| Err(error.to_string()))
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> std::result::Result<bool, String> {
        self.calls.lock().unwrap().push("wait");
        if let Some(wait_started) = self.wait_started.take() {
            let _ = wait_started.send(());
        }
        if let Some(error) = self.wait_error {
            return Err(error.to_string());
        }
        let Some(exit) = self.exit.as_ref() else {
            return Ok(true);
        };
        match exit.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(true),
            Ok(Err(error)) => Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("fake listener disconnected".into()),
        }
    }
}

fn test_listener(native: FakeNative) -> InputListener<FakeNative> {
    InputListener {
        state: Arc::new(Mutex::new(InputState {
            accepting: true,
            input: Input::default(),
        })),
        native: Some(native),
    }
}

#[test]
fn seal_closes_gate_before_stop_and_observed_exit() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut native = FakeNative::default();
    native.calls = calls.clone();
    let listener = test_listener(native);
    let state = listener.state.clone();
    assert!(append_if_accepting(&state, |input| {
        input.cursor.push(CursorSample {
            t_ms: 4,
            x: 1.0,
            y: 2.0,
        });
    }));
    let (cursor, ..) = listener.seal(5).unwrap();
    assert_eq!(cursor.len(), 1);
    assert!(!append_if_accepting(&state, |_| {}));
    assert_eq!(*calls.lock().unwrap(), ["stop", "wait"]);
}

#[test]
fn seal_uses_half_open_cutoff_for_every_sample_type() {
    let input = Input {
        cursor: vec![
            CursorSample {
                t_ms: 9,
                x: 0.0,
                y: 0.0,
            },
            CursorSample {
                t_ms: 10,
                x: 0.0,
                y: 0.0,
            },
        ],
        clicks: vec![
            ClickSample {
                t_ms: 9,
                x: 0.0,
                y: 0.0,
                button: MouseButton::Left,
                down: true,
                position_quality: ClickPositionQuality::Exact,
            },
            ClickSample {
                t_ms: 10,
                x: 0.0,
                y: 0.0,
                button: MouseButton::Left,
                down: true,
                position_quality: ClickPositionQuality::Exact,
            },
        ],
        scrolls: vec![
            ScrollSample {
                t_ms: 9,
                x: 0.0,
                y: 0.0,
                dx: 0.0,
                dy: 1.0,
            },
            ScrollSample {
                t_ms: 10,
                x: 0.0,
                y: 0.0,
                dx: 0.0,
                dy: 1.0,
            },
        ],
        keys: vec![
            KeySample {
                t_ms: 9,
                key: "A".into(),
                down: true,
            },
            KeySample {
                t_ms: 10,
                key: "A".into(),
                down: true,
            },
        ],
        ..Input::default()
    };
    let (cursor, clicks, scrolls, keys) = snapshot_before(&input, 10);
    assert_eq!(
        (cursor.len(), clicks.len(), scrolls.len(), keys.len()),
        (1, 1, 1, 1)
    );
}

#[test]
fn failed_stop_never_blocks_or_releases_ownership_before_late_exit() {
    let (exit_tx, exit_rx) = mpsc::channel();
    let (wait_started_tx, wait_started_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let mut native = FakeNative::default();
    native.stop_error = Some("stop failed");
    native.exit = Some(exit_rx);
    native.wait_started = Some(wait_started_tx);
    native.dropped = Some(dropped_tx);
    let error = test_listener(native)
        .seal(1)
        .expect_err("failed stop is an explicit capture failure");
    assert!(error.to_string().contains("stop passive input listener"));
    wait_started_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("reaper owns the listener after failed stop");
    assert!(dropped_rx.recv_timeout(Duration::from_millis(20)).is_err());
    exit_tx.send(Ok(())).unwrap();
    dropped_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("ownership releases only after late native exit");
}

#[test]
fn seal_timeout_retains_ownership_until_late_exit() {
    let (exit_tx, exit_rx) = mpsc::channel();
    let (wait_started_tx, wait_started_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let mut native = FakeNative::default();
    native.exit = Some(exit_rx);
    native.wait_started = Some(wait_started_tx);
    native.dropped = Some(dropped_tx);
    let error = test_listener(native)
        .seal(1)
        .expect_err("a non-terminating listener cannot seal input");
    assert!(error.to_string().contains("seal passive input listener"));
    wait_started_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("timeout path gives retained ownership to the reaper");
    assert!(dropped_rx.recv_timeout(Duration::from_millis(20)).is_err());
    exit_tx.send(Ok(())).unwrap();
    dropped_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("late exit releases retained ownership");
}

#[test]
fn seal_reports_observed_listener_failure() {
    let mut native = FakeNative::default();
    native.wait_error = Some("wait failed");
    let error = test_listener(native).seal(1).unwrap_err();
    assert!(error.to_string().contains("join passive input listener"));
}

#[test]
fn dropping_unsealed_listener_is_non_blocking_and_retires_it() {
    let (exit_tx, exit_rx) = mpsc::channel();
    let (wait_started_tx, wait_started_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let mut native = FakeNative::default();
    native.exit = Some(exit_rx);
    native.wait_started = Some(wait_started_tx);
    native.dropped = Some(dropped_tx);
    drop(test_listener(native));
    wait_started_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("drop delegates ownership to the reaper");
    assert!(dropped_rx.recv_timeout(Duration::from_millis(20)).is_err());
    exit_tx.send(Ok(())).unwrap();
    dropped_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("reaper releases ownership after exit");
}

#[test]
fn unavailable_listener_seals_an_empty_input_sidecar() {
    let listener = InputListener::<FakeNative> {
        state: Arc::new(Mutex::new(InputState {
            accepting: true,
            input: Input::default(),
        })),
        native: None,
    };
    let observation = listener.startup_observation(true);
    assert_eq!(observation.state, crate::InputHookStartupState::Unavailable);
    assert_eq!(
        observation.reason,
        Some(crate::InputHookStartupReason::StartupFailed)
    );
    assert!(observation.capture_keys);
    let (cursor, clicks, scrolls, keys) = listener.seal(1).unwrap();
    assert!(cursor.is_empty() && clicks.is_empty() && scrolls.is_empty() && keys.is_empty());
}

#[test]
fn process_global_listener_conflict_is_an_explicit_capture_error() {
    let error = listener_start_policy(rdevin::OwnedListenerStartError::AlreadyActive)
        .expect_err("a second owned listener must fail capture startup");
    assert!(error.to_string().contains("start passive input listener"));
}

#[test]
fn first_stationary_native_button_has_actual_position_and_event_time() {
    let mut input = Input::default();
    let point = rdevin::NativePointerSample {
        x: -640.0,
        y: 320.0,
        age_ms: 8,
    };
    push_click(&mut input, 20, MouseButton::Left, true, Some(&point));
    assert_eq!(
        (input.clicks[0].t_ms, input.clicks[0].x, input.clicks[0].y),
        (12, -640.0, 320.0)
    );
    assert_eq!(
        input.clicks[0].position_quality,
        ClickPositionQuality::Exact
    );
    assert_eq!((input.cursor[0].t_ms, input.cursor[0].x), (12, -640.0));
    let surface =
        crate::surface_coordinates::CaptureSurface::new(-1280.0, 0.0, 1280.0, 720.0).unwrap();
    let mapped = crate::surface_coordinates::map_rdevin_input(
        Some(surface),
        1280,
        720,
        input.cursor,
        &mut input.clicks,
        vec![],
    );
    assert_eq!((input.clicks[0].x, input.clicks[0].y), (640.0, 320.0));
    assert_eq!(mapped.correlation.exact_clicks, 1);
    assert_eq!((mapped.cursor[0].x, mapped.cursor[0].y), (640.0, 320.0));
}

#[test]
fn stale_native_button_never_uses_previous_move_as_exact() {
    let mut input = Input {
        last: (10.0, 20.0),
        has_absolute_position: true,
        ..Input::default()
    };
    let point = rdevin::NativePointerSample {
        x: 800.0,
        y: 600.0,
        age_ms: 101,
    };
    push_click(&mut input, 200, MouseButton::Left, true, Some(&point));
    assert_eq!((input.clicks[0].x, input.clicks[0].y), (800.0, 600.0));
    assert_eq!(
        input.clicks[0].position_quality,
        ClickPositionQuality::Unavailable
    );
    assert!(input.cursor.is_empty());
    assert_eq!(input.last, (10.0, 20.0));
}

#[test]
fn absent_native_payload_preserves_existing_fallback_and_pre_take_is_excluded() {
    let mut input = Input::default();
    push_click(&mut input, 20, MouseButton::Left, true, None);
    assert_eq!(
        input.clicks[0].position_quality,
        ClickPositionQuality::Unavailable
    );
    input.last = (10.0, 20.0);
    input.has_absolute_position = true;
    push_click(&mut input, 30, MouseButton::Left, false, None);
    assert_eq!((input.clicks[1].x, input.clicks[1].y), (10.0, 20.0));
    assert_eq!(
        input.clicks[1].position_quality,
        ClickPositionQuality::Exact
    );
    let old = rdevin::NativePointerSample {
        x: 4.0,
        y: 5.0,
        age_ms: 50,
    };
    push_click(&mut input, 40, MouseButton::Left, true, Some(&old));
    assert_eq!(input.clicks.len(), 2);
}

#[test]
fn native_payload_samples_are_sealed_in_event_time_order() {
    let mut input = Input::default();
    for (time, age) in [(20, 0), (30, 15)] {
        let point = rdevin::NativePointerSample {
            x: 4.0,
            y: 5.0,
            age_ms: age,
        };
        push_click(&mut input, time, MouseButton::Left, true, Some(&point));
    }
    let (cursor, clicks, ..) = snapshot_before(&input, 40);
    assert_eq!(cursor.iter().map(|s| s.t_ms).collect::<Vec<_>>(), [15, 20]);
    assert_eq!(clicks.iter().map(|s| s.t_ms).collect::<Vec<_>>(), [15, 20]);
}

#[test]
fn initial_passive_position_without_input_renders_stationary_monitor_cursor() {
    let mut input = Input::default();
    seed_initial_position(&mut input, 12, Some((-880.0, 300.0)));
    assert!(input.clicks.is_empty() && input.scrolls.is_empty() && input.keys.is_empty());
    let surface =
        crate::surface_coordinates::CaptureSurface::new(-1280.0, 0.0, 800.0, 600.0).unwrap();
    let mapped = crate::surface_coordinates::map_rdevin_input(
        Some(surface),
        800,
        600,
        input.cursor,
        &mut input.clicks,
        vec![],
    );
    let mut events = record_core::fixtures::generate("click-walkthrough").unwrap();
    events.screen_w = 800;
    events.screen_h = 600;
    events.duration_ms = 3000;
    events.cursor = mapped.cursor;
    events.clicks.clear();
    events.scrolls.clear();
    events.keys.clear();
    let mut plan = record_engine::autoedit(&events, &record_engine::EngineConfig::default());
    plan.frame.enabled = false;
    plan.background = record_core::Background::Transparent;
    assert_eq!(plan.cursor.smoothed.len(), 1);
    assert!(plan.clicks.is_empty());
    let mut source = tiny_skia::Pixmap::new(800, 600).unwrap();
    source.fill(tiny_skia::Color::BLACK);
    let visible = record_render::compose_frame(&source, &plan, 1000).unwrap();
    plan.cursor.smoothed.clear();
    let baseline = record_render::compose_frame(&source, &plan, 1000).unwrap();
    assert!(
        visible
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
            .count()
            > 10,
        "stationary cursor must be visible in actual polished pixels"
    );
    assert_eq!(
        baseline
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
            .count(),
        0
    );
}

#[test]
fn startup_read_never_overwrites_callback_or_fabricates_unavailable_position() {
    let mut input = Input::default();
    seed_initial_position(&mut input, 12, None);
    seed_initial_position(&mut input, 12, Some((f64::NAN, 4.0)));
    assert!(input.cursor.is_empty() && !input.has_absolute_position);
    seed_initial_position(&mut input, 13, Some((100.0, 200.0)));
    seed_initial_position(&mut input, 14, Some((300.0, 400.0)));
    assert_eq!(input.cursor.len(), 1);
    assert_eq!(input.last, (100.0, 200.0));
    assert_eq!(input.cursor[0].t_ms, 13);
}

#[test]
fn registered_listener_without_events_is_not_reported_as_failed() {
    let listener = test_listener(FakeNative::default());
    let observation = listener.startup_observation(false);
    assert_eq!(observation.state, crate::InputHookStartupState::Registered);
    assert_eq!(observation.reason, None);
    let (cursor, clicks, scrolls, keys) = listener.seal(1).unwrap();
    assert!(cursor.is_empty() && clicks.is_empty() && scrolls.is_empty() && keys.is_empty());
}
