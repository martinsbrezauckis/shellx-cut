//! input.rs — shared rdevin global input hook (Windows + macOS live capture).
//!
//! Both platform backends collect the SAME input event stream (cursor/click/
//! scroll/key) via rdevin, so it lives here. `ButtonPress` carries no coordinates,
//! so we track the last `MouseMove` position and stamp clicks/scrolls with it.
//! Timestamps are our own monotonic clock (Instant) relative to capture start.
//!
//! NOTE: `rdevin::listen` never returns — the listener thread dies with the
//! (one-shot CLI) process. A cutd daemon embedding needs a stoppable hook. macOS
//! additionally requires Accessibility permission; without it events are silently
//! dropped (no error).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use record_core::{
    ClickPositionQuality, ClickSample, CursorSample, KeySample, MouseButton, ScrollSample,
};

/// Accumulated input, filled by the listener thread.
#[derive(Default)]
pub struct Input {
    pub last: (f64, f64),
    /// A button event carries no coordinates. Until rdevin delivered a real global
    /// move, `last` is only its default and cannot identify a captured-frame point.
    pub has_absolute_position: bool,
    pub cursor: Vec<CursorSample>,
    pub clicks: Vec<ClickSample>,
    pub scrolls: Vec<ScrollSample>,
    pub keys: Vec<KeySample>,
}

impl Input {
    /// Clone out the collected events.
    pub fn snapshot(
        &self,
    ) -> (
        Vec<CursorSample>,
        Vec<ClickSample>,
        Vec<ScrollSample>,
        Vec<KeySample>,
    ) {
        (
            self.cursor.clone(),
            self.clicks.clone(),
            self.scrolls.clone(),
            self.keys.clone(),
        )
    }
}

fn map_button(b: rdevin::Button) -> MouseButton {
    match b {
        rdevin::Button::Left => MouseButton::Left,
        rdevin::Button::Right => MouseButton::Right,
        rdevin::Button::Middle => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

/// Spawn the rdevin global hook on a thread; returns the shared accumulator.
/// After `stop` is set, the callback ignores further events. `capture_keys` gates
/// KEYSTROKE recording — OFF by default, because key-cast can burn in passwords,
/// recovery phrases, API keys, or private input. Cursor/click/scroll are always
/// captured (positions, not content).
pub fn spawn_listener(
    start: Instant,
    stop: Arc<AtomicBool>,
    capture_keys: bool,
) -> Arc<Mutex<Input>> {
    let input = Arc::new(Mutex::new(Input::default()));
    let ev = input.clone();
    thread::spawn(move || {
        let _ = rdevin::listen(move |event| {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let t = start.elapsed().as_millis() as u64;
            let mut s = ev.lock().unwrap();
            match event.event_type {
                rdevin::EventType::MouseMove { x, y } => {
                    s.last = (x, y);
                    s.has_absolute_position = true;
                    s.cursor.push(CursorSample { t_ms: t, x, y });
                }
                rdevin::EventType::ButtonPress(b) => {
                    let (x, y) = s.last;
                    let position_quality = if s.has_absolute_position {
                        ClickPositionQuality::Exact
                    } else {
                        ClickPositionQuality::Unavailable
                    };
                    s.clicks.push(ClickSample {
                        t_ms: t,
                        x,
                        y,
                        button: map_button(b),
                        down: true,
                        position_quality,
                    });
                }
                rdevin::EventType::ButtonRelease(b) => {
                    let (x, y) = s.last;
                    let position_quality = if s.has_absolute_position {
                        ClickPositionQuality::Exact
                    } else {
                        ClickPositionQuality::Unavailable
                    };
                    s.clicks.push(ClickSample {
                        t_ms: t,
                        x,
                        y,
                        button: map_button(b),
                        down: false,
                        position_quality,
                    });
                }
                rdevin::EventType::Wheel { delta_x, delta_y } => {
                    let (x, y) = s.last;
                    s.scrolls.push(ScrollSample {
                        t_ms: t,
                        x,
                        y,
                        dx: delta_x as f64,
                        dy: delta_y as f64,
                    });
                }
                rdevin::EventType::KeyPress(k) => {
                    if capture_keys {
                        s.keys.push(KeySample {
                            t_ms: t,
                            key: format!("{k:?}"),
                            down: true,
                        });
                    }
                }
                rdevin::EventType::KeyRelease(k) => {
                    if capture_keys {
                        s.keys.push(KeySample {
                            t_ms: t,
                            key: format!("{k:?}"),
                            down: false,
                        });
                    }
                }
            }
        });
    });
    input
}
