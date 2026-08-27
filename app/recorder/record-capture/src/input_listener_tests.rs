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
    let (cursor, clicks, scrolls, keys) = listener.seal(1).unwrap();
    assert!(cursor.is_empty() && clicks.is_empty() && scrolls.is_empty() && keys.is_empty());
}

#[test]
fn process_global_listener_conflict_is_an_explicit_capture_error() {
    let error = listener_start_policy(rdevin::OwnedListenerStartError::AlreadyActive)
        .expect_err("a second owned listener must fail capture startup");
    assert!(error.to_string().contains("start passive input listener"));
}
