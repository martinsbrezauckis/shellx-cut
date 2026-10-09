//! Owned Linux input-route selection: stoppable X11 RECORD or existing Wayland evdev.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use record_core::Result;

use crate::input;

/// Both Linux input routes remain owned. X11 uses the maintained rdevin RECORD
/// listener; Wayland deliberately keeps its existing evdev owner unchanged.
pub(super) enum LinuxInputListener {
    Evdev(crate::input_evdev::EvdevListener),
    Rdevin(input::InputListener),
}

impl LinuxInputListener {
    pub(super) fn startup_observation(&self, capture_keys: bool) -> crate::InputHookStartup {
        match self {
            Self::Evdev(_) => crate::InputHookStartup::wayland_evdev(capture_keys),
            Self::Rdevin(listener) => listener.startup_observation(capture_keys),
        }
    }

    pub(super) fn finish(self, duration_ms: u64) -> Result<crate::input_evdev::EvdevSnapshot> {
        match self {
            Self::Evdev(listener) => listener.seal(duration_ms),
            Self::Rdevin(listener) => listener.seal(duration_ms),
        }
    }
}

pub(super) fn start(
    use_evdev: bool,
    start: Instant,
    external_stop: Arc<AtomicBool>,
    capture_keys: bool,
    screen_width: u32,
    screen_height: u32,
    readiness: Option<&crate::CaptureReadiness>,
) -> Result<LinuxInputListener> {
    let listener = if use_evdev {
        LinuxInputListener::Evdev(crate::input_evdev::spawn_evdev_listener(
            start,
            external_stop,
            capture_keys,
            screen_width,
            screen_height,
        ))
    } else {
        LinuxInputListener::Rdevin(input::InputListener::start(start, capture_keys)?)
    };
    if let Some(readiness) = readiness {
        readiness.publish_input_hook_startup(listener.startup_observation(capture_keys));
    }
    Ok(listener)
}
