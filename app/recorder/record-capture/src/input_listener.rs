//! Bounded ownership for Cut's passive native input listener.
//!
//! A failed platform stop must never wedge the recording worker or release the
//! process-global admission guard while a native hook might still be live.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use record_core::{error_codes, ClickPositionQuality, RecordError, Result};

use crate::input::{map_button, Input, InputSnapshot};

const SEAL_WAIT: Duration = Duration::from_millis(250);
const RETIRED_POLL: Duration = Duration::from_millis(100);

/// Sample state and its acceptance gate deliberately share one mutex. A native
/// callback queued before sealing cannot append after `seal` acquires this lock.
struct InputState {
    accepting: bool,
    input: Input,
}

/// An owned native listener may be asked to stop, then observed to have exited.
/// `wait_for_exit` is bounded: `false` means ownership must remain retained.
pub(crate) trait NativeInputListener: Send + 'static {
    fn request_stop(&self) -> std::result::Result<(), String>;
    fn wait_for_exit(&mut self, timeout: Duration) -> std::result::Result<bool, String>;
}

impl NativeInputListener for rdevin::OwnedListener {
    fn request_stop(&self) -> std::result::Result<(), String> {
        self.request_stop().map_err(|error| error.to_string())
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> std::result::Result<bool, String> {
        rdevin::OwnedListener::wait_for_exit(self, timeout).map_err(|error| error.to_string())
    }
}

/// Owns the passive native listener, its acceptance gate, and every sample it
/// may append. Sealing stops native delivery before any post-video work begins.
pub(crate) struct InputListener<L: NativeInputListener = rdevin::OwnedListener> {
    state: Arc<Mutex<InputState>>,
    native: Option<L>,
}

impl InputListener<rdevin::OwnedListener> {
    pub(crate) fn start(start: Instant, capture_keys: bool) -> Result<Self> {
        Self::start_with_policy(start, capture_keys, false)
    }

    /// Start a listener that is mandatory for the caller's immutable stream
    /// selection. Unlike [`Self::start`], this never turns an unavailable
    /// native hook into an empty successful input stream.
    #[cfg(all(windows, feature = "capture-windows"))]
    pub(crate) fn start_required(start: Instant, capture_keys: bool) -> Result<Self> {
        Self::start_with_policy(start, capture_keys, true)
    }

    fn start_with_policy(start: Instant, capture_keys: bool, required: bool) -> Result<Self> {
        let state = Arc::new(Mutex::new(InputState {
            accepting: true,
            input: Input::default(),
        }));
        let callback_state = state.clone();
        let native = rdevin::listen_owned(move |event| {
            let timestamp_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            append_if_accepting(&callback_state, |input| match event.event_type {
                rdevin::EventType::MouseMove { x, y } => {
                    input.last = (x, y);
                    input.has_absolute_position = true;
                    input.cursor.push(record_core::CursorSample {
                        t_ms: timestamp_ms,
                        x,
                        y,
                    });
                }
                rdevin::EventType::ButtonPress(button) => {
                    push_click(input, timestamp_ms, map_button(button), true)
                }
                rdevin::EventType::ButtonRelease(button) => {
                    push_click(input, timestamp_ms, map_button(button), false)
                }
                rdevin::EventType::Wheel { delta_x, delta_y } => {
                    let (x, y) = input.last;
                    input.scrolls.push(record_core::ScrollSample {
                        t_ms: timestamp_ms,
                        x,
                        y,
                        dx: delta_x as f64,
                        dy: delta_y as f64,
                    });
                }
                rdevin::EventType::KeyPress(key) if capture_keys => {
                    input.keys.push(record_core::KeySample {
                        t_ms: timestamp_ms,
                        key: format!("{key:?}"),
                        down: true,
                    });
                }
                rdevin::EventType::KeyRelease(key) if capture_keys => {
                    input.keys.push(record_core::KeySample {
                        t_ms: timestamp_ms,
                        key: format!("{key:?}"),
                        down: false,
                    });
                }
                _ => {}
            });
        });
        let native = match native {
            Ok(listener) => Some(listener),
            Err(error) => {
                if required {
                    return Err(listener_error(
                        "start required passive input listener",
                        error,
                    ));
                }
                listener_start_policy(error)?;
                None
            }
        };
        Ok(Self { state, native })
    }
}

impl<L: NativeInputListener> InputListener<L> {
    /// Close acceptance, stop the native route, and accept a snapshot only once
    /// native teardown is observed. Failure retains ownership in a background
    /// reaper instead of risking a blocking Stop or a second global listener.
    pub(crate) fn seal(mut self, duration_ms: u64) -> Result<InputSnapshot> {
        self.close_gate();
        let Some(mut native) = self.native.take() else {
            return Ok(self.snapshot(duration_ms));
        };
        if let Err(error) = native.request_stop() {
            retain_until_exit(native, "stop delivery failed");
            return Err(listener_error("stop passive input listener", error));
        }
        match native.wait_for_exit(SEAL_WAIT) {
            Ok(true) => Ok(self.snapshot(duration_ms)),
            Ok(false) => {
                retain_until_exit(
                    native,
                    "native listener did not exit before the seal deadline",
                );
                Err(listener_error(
                    "seal passive input listener",
                    format!("listener did not exit within {} ms", SEAL_WAIT.as_millis()),
                ))
            }
            Err(error) => Err(listener_error("join passive input listener", error)),
        }
    }

    fn close_gate(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.accepting = false;
    }

    fn snapshot(&self, duration_ms: u64) -> InputSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot_before(&state.input, duration_ms)
    }
}

impl<L: NativeInputListener> Drop for InputListener<L> {
    fn drop(&mut self) {
        self.close_gate();
        if let Some(native) = self.native.take() {
            // A destructor cannot wait on a possibly wedged native listener.
            // The reaper owns both the handle and its singleton guard until exit.
            if let Err(error) = native.request_stop() {
                eprintln!("input listener cleanup stop failed: {error}");
            }
            retain_until_exit(native, "capture aborted before passive input sealed");
        }
    }
}

fn retain_until_exit<L: NativeInputListener>(native: L, reason: &'static str) {
    eprintln!("input listener retained after {reason}; waiting for native exit");
    let retained = Arc::new(Mutex::new(Some(native)));
    let worker = retained.clone();
    let spawn = std::thread::Builder::new()
        .name("cut-input-reaper".into())
        .spawn(move || {
            let mut native = worker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .expect("input reaper owns one retained listener");
            loop {
                match native.wait_for_exit(RETIRED_POLL) {
                    Ok(true) => {
                        eprintln!("input listener exited after {reason}");
                        return;
                    }
                    Ok(false) => continue,
                    Err(error) => {
                        eprintln!("input listener ended after {reason}: {error}");
                        return;
                    }
                }
            }
        });
    if let Err(error) = spawn {
        // Resource exhaustion must remain fail-closed. Leaking this single
        // ownership object is preferable to admitting another listener while
        // the native callback might still be registered.
        let native = retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("failed input reaper keeps its listener");
        Box::leak(Box::new(native));
        eprintln!("input listener reaper could not start; ownership retained: {error}");
    }
}

fn push_click(input: &mut Input, timestamp_ms: u64, button: record_core::MouseButton, down: bool) {
    let (x, y) = input.last;
    input.clicks.push(record_core::ClickSample {
        t_ms: timestamp_ms,
        x,
        y,
        button,
        down,
        position_quality: click_quality(input),
    });
}

fn click_quality(input: &Input) -> ClickPositionQuality {
    if input.has_absolute_position {
        ClickPositionQuality::Exact
    } else {
        ClickPositionQuality::Unavailable
    }
}

fn append_if_accepting(state: &Mutex<InputState>, append: impl FnOnce(&mut Input)) -> bool {
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !state.accepting {
        return false;
    }
    append(&mut state.input);
    true
}

fn snapshot_before(input: &Input, duration_ms: u64) -> InputSnapshot {
    let (mut cursor, mut clicks, mut scrolls, mut keys) = input.snapshot();
    cursor.retain(|sample| sample.t_ms < duration_ms);
    clicks.retain(|sample| sample.t_ms < duration_ms);
    scrolls.retain(|sample| sample.t_ms < duration_ms);
    keys.retain(|sample| sample.t_ms < duration_ms);
    (cursor, clicks, scrolls, keys)
}

fn listener_error(context: &str, error: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, error.to_string())
        .with_action("retry the recording; if this repeats, collect the capture logs")
}

fn listener_start_policy(error: rdevin::OwnedListenerStartError) -> Result<()> {
    if matches!(error, rdevin::OwnedListenerStartError::AlreadyActive) {
        return Err(listener_error("start passive input listener", error));
    }
    // Native input is optional when the host cannot provide a passive hook (for
    // example no X11 RECORD display or a denied macOS event tap). Do not invent
    // samples or fail an otherwise-valid screen capture.
    eprintln!("input listener unavailable; recording without input: {error}");
    Ok(())
}

#[cfg(test)]
#[path = "input_listener_tests.rs"]
mod tests;
