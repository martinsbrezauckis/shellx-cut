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
//! offset + slow edge drift). The precise source is the ScreenCast portal's cursor
//! METADATA (CursorMode::Metadata); pairing click times to it is the planned upgrade.
//! Until then, click positions use this approximate cursor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use evdev::{EventSummary, EventType, KeyCode, RelativeAxisCode};
use record_core::{ClickSample, CursorSample, KeySample, MouseButton, ScrollSample};

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

/// Spawn evdev reader threads (one per key/rel device); returns the shared
/// accumulator, same contract as `input::spawn_listener`. Cursor is seeded at the
/// screen center and accumulated from relative motion (see caveat above).
pub fn spawn_evdev_listener(
    start: Instant,
    stop: Arc<AtomicBool>,
    capture_keys: bool,
    screen_w: u32,
    screen_h: u32,
) -> Arc<Mutex<Input>> {
    let input = Arc::new(Mutex::new(Input::default()));
    {
        let mut s = input.lock().unwrap();
        s.last = (screen_w as f64 / 2.0, screen_h as f64 / 2.0);
    }
    let (sw, sh) = (screen_w as f64, screen_h as f64);

    let mut opened = 0usize;
    for (path, dev) in evdev::enumerate() {
        // Only devices that emit keys/buttons or relative motion are interesting.
        let evs = dev.supported_events();
        if !(evs.contains(EventType::KEY) || evs.contains(EventType::RELATIVE)) {
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
        let ev = input.clone();
        let stop = stop.clone();
        let mut dev = dev;
        thread::spawn(move || loop {
            if stop.load(Ordering::Relaxed) {
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
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let t = start.elapsed().as_millis() as u64;
                let mut s = ev.lock().unwrap();
                match event.destructure() {
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_X, v) => {
                        s.last.0 = (s.last.0 + v as f64).clamp(0.0, sw);
                        let (x, y) = s.last;
                        s.cursor.push(CursorSample { t_ms: t, x, y });
                    }
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_Y, v) => {
                        s.last.1 = (s.last.1 + v as f64).clamp(0.0, sh);
                        let (x, y) = s.last;
                        s.cursor.push(CursorSample { t_ms: t, x, y });
                    }
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_WHEEL, v) => {
                        let (x, y) = s.last;
                        s.scrolls.push(ScrollSample {
                            t_ms: t,
                            x,
                            y,
                            dx: 0.0,
                            dy: v as f64,
                        });
                    }
                    EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL, v) => {
                        let (x, y) = s.last;
                        s.scrolls.push(ScrollSample {
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
                            continue;
                        }
                        let down = value == 1;
                        if let Some(button) = map_btn(code) {
                            let (x, y) = s.last;
                            s.clicks.push(ClickSample {
                                t_ms: t,
                                x,
                                y,
                                button,
                                down,
                            });
                        } else if capture_keys {
                            s.keys.push(KeySample {
                                t_ms: t,
                                key: format!("{code:?}"),
                                down,
                            });
                        }
                    }
                    _ => {}
                }
            }
        });
    }
    if opened == 0 {
        eprintln!("evdev: NO readable input devices — need `input` group or root");
    }
    input
}
