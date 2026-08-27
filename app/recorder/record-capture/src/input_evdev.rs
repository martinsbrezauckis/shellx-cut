//! input_evdev.rs — global input capture via evdev (`/dev/input`), for WAYLAND.
//!
//! Wayland deliberately blocks passive global input hooks (anti-keylogger), so the
//! rdevin/X11 path doesn't work there. The proven cross-compositor approach — used by
//! showmethekey (libinput) and wshowkeys (evdev) — is to read the kernel input
//! devices directly. This is PASSIVE (does NOT grab input from the desktop, unlike
//! the InputCapture portal) but needs read access to `/dev/input/event*`: membership
//! in the `input` group (`sudo usermod -aG input <user>` + re-login), reported by
//! `doctor`. Captures clicks (BTN_*), keys (KEY_*, opt-in), and scroll wheels.
//!
//! CURSOR POSITION CAVEAT: relative mice report REL_X/REL_Y deltas, not an absolute
//! position. We accumulate from screen center and clamp — APPROXIMATE (constant
//! offset + slow edge drift). Wayland PipeWire metadata can later pair each click to
//! a fresh absolute cursor sample; until then this module marks every click
//! `Approximate` so no consumer silently treats the accumulator as exact.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use evdev::{EventSummary, EventType, KeyCode, RelativeAxisCode};
use record_core::{
    error_codes, ClickPositionQuality, ClickSample, CursorSample, KeySample, MouseButton,
    RecordError, Result, ScrollSample,
};

use crate::input::Input;

fn map_btn(k: KeyCode) -> Option<MouseButton> {
    match k {
        KeyCode::BTN_LEFT => Some(MouseButton::Left),
        KeyCode::BTN_RIGHT => Some(MouseButton::Right),
        KeyCode::BTN_MIDDLE => Some(MouseButton::Middle),
        _ => None,
    }
}

/// True if at least one `/dev/input/event*` device is readable (i.e. `input` group
/// or root). Used by `doctor` to report the Wayland input-capture prerequisite.
pub fn evdev_readable() -> bool {
    evdev::enumerate().next().is_some()
}

pub(crate) type EvdevSnapshot = (
    Vec<CursorSample>,
    Vec<ClickSample>,
    Vec<ScrollSample>,
    Vec<KeySample>,
);

/// Sample state for every evdev reader. `accepting` lives under this mutex with
/// the vectors, so sealing cannot race a callback that already passed the gate.
struct EvdevState {
    accepting: bool,
    input: Input,
}

/// Owns every Wayland evdev reader until a sealed snapshot has joined them.
///
/// The external capture stop remains a terminal input for the readers. The
/// private cancellation signal makes sealing self-contained, including callers
/// that need to close input before the outer capture stop is published.
pub(crate) struct EvdevListener {
    state: Arc<Mutex<EvdevState>>,
    cancel: Arc<AtomicBool>,
    external_stop: Arc<AtomicBool>,
    readers: Vec<JoinHandle<()>>,
}

impl EvdevListener {
    fn new(external_stop: Arc<AtomicBool>, screen_w: u32, screen_h: u32) -> Self {
        Self {
            state: Arc::new(Mutex::new(EvdevState {
                accepting: true,
                input: Input {
                    last: (screen_w as f64 / 2.0, screen_h as f64 / 2.0),
                    ..Input::default()
                },
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            external_stop,
            readers: Vec::new(),
        }
    }

    /// Close the acceptance gate, stop all readers, then return a half-open
    /// snapshot only after every reader has exited.
    pub(crate) fn seal(mut self, duration_ms: u64) -> Result<EvdevSnapshot> {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.accepting = false;
        }
        self.cancel.store(true, Ordering::Release);

        let reader_panicked = self.join_readers();
        if reader_panicked {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "evdev input reader panicked while sealing capture",
                "a Wayland evdev reader did not exit cleanly",
            )
            .with_action("retry the recording; if this repeats, collect the Linux capture logs"));
        }

        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(snapshot_before(&state.input, duration_ms))
    }

    fn join_readers(&mut self) -> bool {
        let mut reader_panicked = false;
        for reader in self.readers.drain(..) {
            if reader.join().is_err() {
                reader_panicked = true;
            }
        }
        reader_panicked
    }
}

impl Drop for EvdevListener {
    fn drop(&mut self) {
        // A capture error before `seal` must still not leave evdev readers alive.
        // Normal sealing drains this list first and reports a reader panic through
        // its Result; Drop has no error channel, so it can only make cleanup best effort.
        self.cancel.store(true, Ordering::Release);
        if self.join_readers() {
            eprintln!("evdev: reader panicked during cleanup");
        }
    }
}

fn readers_should_stop(cancel: &AtomicBool, external_stop: &AtomicBool) -> bool {
    cancel.load(Ordering::Acquire) || external_stop.load(Ordering::Acquire)
}

/// Runs an append only while the shared sample gate is open. The gate check and
/// vector mutation share the same lock, so a callback queued before sealing cannot
/// append after the seal operation acquires that lock.
fn append_if_accepting(state: &Mutex<EvdevState>, append: impl FnOnce(&mut Input)) -> bool {
    let mut state = state.lock().unwrap();
    if !state.accepting {
        return false;
    }
    append(&mut state.input);
    true
}

fn snapshot_before(input: &Input, duration_ms: u64) -> EvdevSnapshot {
    let (mut cursor, mut clicks, mut scrolls, mut keys) = input.snapshot();
    cursor.retain(|sample| sample.t_ms < duration_ms);
    clicks.retain(|sample| sample.t_ms < duration_ms);
    scrolls.retain(|sample| sample.t_ms < duration_ms);
    keys.retain(|sample| sample.t_ms < duration_ms);
    (cursor, clicks, scrolls, keys)
}

/// Spawn evdev reader threads (one per key/rel device) in an owned listener.
/// Cursor is seeded at the screen center and accumulated from relative motion
/// (see caveat above). A listener with zero readable devices is still valid and
/// seals to an empty snapshot.
pub(crate) fn spawn_evdev_listener(
    start: Instant,
    external_stop: Arc<AtomicBool>,
    capture_keys: bool,
    screen_w: u32,
    screen_h: u32,
) -> EvdevListener {
    let mut listener = EvdevListener::new(external_stop, screen_w, screen_h);
    let (sw, sh) = (screen_w as f64, screen_h as f64);

    let mut opened = 0usize;
    for (path, mut dev) in evdev::enumerate() {
        // Only devices that emit keys/buttons or relative motion are interesting.
        let evs = dev.supported_events();
        if !(evs.contains(EventType::KEY) || evs.contains(EventType::RELATIVE)) {
            continue;
        }
        // `fetch_events` blocks by default. Readers must poll so the owned
        // listener can observe either its private cancellation or external stop
        // and join deterministically during sealing.
        if let Err(error) = dev.set_nonblocking(true) {
            eprintln!(
                "evdev: cannot make {} non-blocking: {error}",
                path.display()
            );
            continue;
        }
        if std::env::var("SHELLX_RECORD_DEBUG").is_ok() {
            eprintln!(
                "evdev: reading {} ({})",
                path.display(),
                dev.name().unwrap_or("?")
            );
        }
        opened += 1;
        let state = listener.state.clone();
        let cancel = listener.cancel.clone();
        let external_stop = listener.external_stop.clone();
        listener.readers.push(thread::spawn(move || loop {
            if readers_should_stop(&cancel, &external_stop) {
                break;
            }
            let events = match dev.fetch_events() {
                Ok(e) => e,
                // Non-blocking device → poll; only a real error ends the reader.
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(4));
                    continue;
                }
                Err(_) => break,
            };
            for event in events {
                if readers_should_stop(&cancel, &external_stop) {
                    break;
                }
                let t = start.elapsed().as_millis() as u64;
                append_if_accepting(&state, |input| match event.destructure() {
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_X, v) => {
                        input.last.0 = (input.last.0 + v as f64).clamp(0.0, sw);
                        let (x, y) = input.last;
                        input.cursor.push(CursorSample { t_ms: t, x, y });
                    }
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_Y, v) => {
                        input.last.1 = (input.last.1 + v as f64).clamp(0.0, sh);
                        let (x, y) = input.last;
                        input.cursor.push(CursorSample { t_ms: t, x, y });
                    }
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_WHEEL, v) => {
                        let (x, y) = input.last;
                        input.scrolls.push(ScrollSample {
                            t_ms: t,
                            x,
                            y,
                            dx: 0.0,
                            dy: v as f64,
                        });
                    }
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL, v) => {
                        let (x, y) = input.last;
                        input.scrolls.push(ScrollSample {
                            t_ms: t,
                            x,
                            y,
                            dx: v as f64,
                            dy: 0.0,
                        });
                    }
                    EventSummary::Key(_, code, value) => {
                        // value: 1 = press, 0 = release, 2 = autorepeat (ignored).
                        if value == 2 {
                            return;
                        }
                        let down = value == 1;
                        if let Some(button) = map_btn(code) {
                            let (x, y) = input.last;
                            input.clicks.push(ClickSample {
                                t_ms: t,
                                x,
                                y,
                                button,
                                down,
                                position_quality: ClickPositionQuality::Approximate,
                            });
                        } else if capture_keys {
                            input.keys.push(KeySample {
                                t_ms: t,
                                key: format!("{code:?}"),
                                down,
                            });
                        }
                    }
                    _ => {}
                });
            }
        }));
    }
    if opened == 0 {
        eprintln!("evdev: NO readable input devices — need `input` group or root");
    }
    listener
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn test_listener() -> EvdevListener {
        EvdevListener::new(Arc::new(AtomicBool::new(false)), 1920, 1080)
    }

    #[test]
    fn sealed_snapshot_is_half_open_for_every_sample_type() {
        let listener = test_listener();
        let state = listener.state.clone();
        assert!(append_if_accepting(&state, |input| {
            for t_ms in [9, 10] {
                input.cursor.push(CursorSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                });
                input.clicks.push(ClickSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                    button: MouseButton::Left,
                    down: true,
                    position_quality: ClickPositionQuality::Approximate,
                });
                input.scrolls.push(ScrollSample {
                    t_ms,
                    x: 1.0,
                    y: 2.0,
                    dx: 3.0,
                    dy: 4.0,
                });
                input.keys.push(KeySample {
                    t_ms,
                    key: "KEY_A".into(),
                    down: true,
                });
            }
        }));

        let (cursor, clicks, scrolls, keys) = listener.seal(10).unwrap();
        assert_eq!(
            cursor.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            clicks.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            scrolls.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
        assert_eq!(
            keys.iter().map(|sample| sample.t_ms).collect::<Vec<_>>(),
            [9]
        );
    }

    #[test]
    fn queued_callback_cannot_append_after_the_seal_gate_wins_the_mutex() {
        let listener = test_listener();
        let state = listener.state.clone();
        let mut gate = state.lock().unwrap();
        let queued_state = state.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let callback = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            append_if_accepting(&queued_state, |input| {
                input.cursor.push(CursorSample {
                    t_ms: 1,
                    x: 1.0,
                    y: 1.0,
                });
            })
        });
        ready_rx.recv().unwrap();

        // This is the first operation performed by `seal`, while the queued
        // callback remains blocked on the same mutex.
        gate.accepting = false;
        drop(gate);

        assert!(!callback.join().unwrap());
        assert!(listener.seal(10).unwrap().0.is_empty());
    }

    #[test]
    fn seal_cancels_and_joins_controlled_readers() {
        let mut listener = test_listener();
        let cancel = listener.cancel.clone();
        let joined = Arc::new(AtomicBool::new(false));
        let joined_by_reader = joined.clone();
        listener.readers.push(thread::spawn(move || {
            while !cancel.load(Ordering::Acquire) {
                thread::yield_now();
            }
            joined_by_reader.store(true, Ordering::Release);
        }));

        listener.seal(10).unwrap();
        assert!(joined.load(Ordering::Acquire));
    }

    #[test]
    fn reader_observes_external_stop_before_seal() {
        let external_stop = Arc::new(AtomicBool::new(false));
        let mut listener = EvdevListener::new(external_stop.clone(), 1920, 1080);
        let cancel = listener.cancel.clone();
        let observed_external_stop = Arc::new(AtomicBool::new(false));
        let observed_by_reader = observed_external_stop.clone();
        let external_for_reader = external_stop.clone();
        listener.readers.push(thread::spawn(move || {
            while !readers_should_stop(&cancel, &external_for_reader) {
                thread::yield_now();
            }
            observed_by_reader.store(
                external_for_reader.load(Ordering::Acquire),
                Ordering::Release,
            );
        }));

        external_stop.store(true, Ordering::Release);
        listener.seal(10).unwrap();
        assert!(observed_external_stop.load(Ordering::Acquire));
    }

    #[test]
    fn reader_panic_is_an_explicit_seal_error_after_other_readers_join() {
        let mut listener = test_listener();
        let cancel = listener.cancel.clone();
        let joined = Arc::new(AtomicBool::new(false));
        let joined_by_reader = joined.clone();
        listener.readers.push(thread::spawn(move || {
            while !cancel.load(Ordering::Acquire) {
                thread::yield_now();
            }
            joined_by_reader.store(true, Ordering::Release);
        }));
        listener
            .readers
            .push(thread::spawn(|| panic!("controlled reader panic")));

        let error = listener.seal(10).unwrap_err();
        assert_eq!(error.code, error_codes::CAPTURE);
        assert!(error.message.contains("reader panicked"));
        assert!(joined.load(Ordering::Acquire));
    }

    #[test]
    fn zero_device_listener_seals_to_an_empty_snapshot() {
        let (cursor, clicks, scrolls, keys) = test_listener().seal(10).unwrap();
        assert!(cursor.is_empty());
        assert!(clicks.is_empty());
        assert!(scrolls.is_empty());
        assert!(keys.is_empty());
    }
}
