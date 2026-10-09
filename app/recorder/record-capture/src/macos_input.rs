//! ScreenCaptureKit input startup and its per-capture readiness observation.

use std::time::Instant;

use record_core::Result;
use screencapturekit::prelude::*;

use crate::input::InputListener;
use crate::macos_window_clicks::WindowClickCapture;
use crate::CaptureConfig;

/// Arm Window input before SCK can deliver its first frame during startup.
pub(super) fn arm_window_start(
    window_clicks: Option<&WindowClickCapture>,
    cfg: &CaptureConfig,
) -> Option<Instant> {
    window_clicks.map(|owner| {
        let start = cfg
            .clock
            .as_ref()
            .map(crate::CaptureClock::start)
            .unwrap_or_else(Instant::now);
        owner.start(start);
        start
    })
}

/// Preserve Display/Region's post-acceptance clock and Window's pre-armed one.
/// Publish startup only after the same owned input listener has started.
pub(super) fn start(
    stream: &mut SCStream,
    cfg: &CaptureConfig,
    window_clicks: Option<WindowClickCapture>,
    window_start: Option<Instant>,
) -> Result<(Instant, InputListener)> {
    let start = window_start.unwrap_or_else(|| {
        cfg.clock
            .as_ref()
            .map(crate::CaptureClock::start)
            .unwrap_or_else(Instant::now)
    });
    let input_start = match window_clicks {
        Some(owner) => {
            InputListener::start_observed(start, cfg.capture_keys, move |event, timestamp| {
                owner.observe_event(event, timestamp)
            })
        }
        None => InputListener::start(start, cfg.capture_keys),
    };
    let listener = match input_start {
        Ok(listener) => listener,
        Err(error) => {
            let _ = stream.stop_capture();
            return Err(error);
        }
    };
    if let Some(readiness) = cfg.readiness.as_ref() {
        readiness.publish_input_hook_startup(listener.startup_observation(cfg.capture_keys));
    }
    Ok((start, listener))
}
