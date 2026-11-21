//! Owned Linux input-route selection: stoppable X11 RECORD or existing Wayland evdev.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use record_core::{error_codes, RecordError, Result};

use crate::input;

pub(super) fn note_unavailable(correlation: &mut record_core::CursorCorrelation) {
    correlation.detail = Some(
        "Input events unavailable: /dev/input was not readable; relative pointer, button, scroll and key events were not recorded; screen cursor metadata may still be present"
            .into(),
    );
}

/// Both Linux input routes remain owned. X11 uses the maintained rdevin RECORD
/// listener; Wayland deliberately keeps its existing evdev owner unchanged.
pub(super) enum LinuxInputListener {
    Evdev(crate::input_evdev::EvdevListener),
    Rdevin(input::InputListener),
}

impl LinuxInputListener {
    pub(super) fn input_unavailable(&self) -> bool {
        matches!(self, Self::Evdev(listener) if !listener.has_readers())
    }

    pub(super) fn startup_observation(&self, capture_keys: bool) -> crate::InputHookStartup {
        match self {
            Self::Evdev(_) => crate::InputHookStartup::wayland_evdev(capture_keys),
            Self::Rdevin(listener) => listener.startup_observation(capture_keys),
        }
    }

    pub(super) fn finish(
        self,
        duration_ms: u64,
    ) -> Result<(
        crate::input_evdev::EvdevSnapshot,
        Option<crate::cursor_correlation::EvdevEvidence>,
    )> {
        match self {
            Self::Evdev(listener) => listener
                .seal_with_evidence(duration_ms)
                .map(|(snapshot, evidence)| (snapshot, Some(evidence))),
            Self::Rdevin(listener) => listener.seal(duration_ms).map(|snapshot| (snapshot, None)),
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
    if listener.input_unavailable() {
        if let Some(readiness) = readiness {
            readiness.publish_input_hook_startup(crate::InputHookStartup {
                state: crate::InputHookStartupState::Unavailable,
                reason: Some(crate::InputHookStartupReason::StartupFailed),
                ..crate::InputHookStartup::wayland_evdev(capture_keys)
            });
        }
        if capture_keys {
            return Err(RecordError::new(
                error_codes::CAPTURE,
                "keystroke capture cannot start on this Wayland desktop",
                "/dev/input is not readable; Show keystrokes needs input-device access",
            )
            .with_action("turn off Show keystrokes, or add your user to the input group with sudo usermod -aG input $USER and sign in again"));
        }
    } else if let Some(readiness) = readiness {
        readiness.publish_input_hook_startup(listener.startup_observation(capture_keys));
    }
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CaptureReadiness, InputHookBackend, InputHookStartupReason, InputHookStartupState,
    };

    #[test]
    fn unreadable_wayland_input_keeps_plain_video_start_but_refuses_opted_in_keys() {
        if crate::input_evdev::evdev_readable() {
            return; // This case requires a desktop account without input-device access.
        }
        let readiness = CaptureReadiness::default();
        let start_time = Instant::now();
        let stop = Arc::new(AtomicBool::new(false));
        let plain = start(
            true,
            start_time,
            stop.clone(),
            false,
            1920,
            1080,
            Some(&readiness),
        );
        assert!(
            plain.is_ok(),
            "screen and sound can start without input events"
        );
        let observation = readiness.input_hook_startup();
        assert_eq!(observation.state, InputHookStartupState::Unavailable);
        assert_eq!(observation.backend, InputHookBackend::WaylandEvdev);
        assert_eq!(
            observation.reason,
            Some(InputHookStartupReason::StartupFailed)
        );
        assert!(!observation.capture_keys);
        readiness.mark_first_screen_frame_delivered();
        assert!(readiness.status().ready);
        let _ = plain.unwrap().finish(1);

        let keys = start(true, start_time, stop, true, 1920, 1080, None);
        assert!(keys.is_err(), "opted-in keys need readable input devices");
    }
}
