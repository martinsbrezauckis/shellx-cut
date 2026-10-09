//! WGC input listener startup and its per-capture readiness observation.

use std::time::Instant;

use record_core::Result;
use windows::Win32::Foundation::HWND;

use crate::input::InputListener;
use crate::windows_window_clicks::WindowClickCapture;
use crate::{windows::cap_err, CaptureConfig};

pub(super) fn start(
    cfg: &CaptureConfig,
    window_hwnd: Option<HWND>,
    width: u32,
    height: u32,
) -> Result<(Instant, InputListener, Option<WindowClickCapture>)> {
    let start = cfg
        .clock
        .as_ref()
        .map(crate::CaptureClock::start)
        .unwrap_or_else(Instant::now);
    let window_clicks = match window_hwnd {
        Some(hwnd) => {
            let id = cfg.window.as_deref().ok_or_else(|| {
                cap_err(
                    "bind selected-window clicks",
                    "the admitted window identity is missing",
                )
            })?;
            Some(
                WindowClickCapture::new(id, hwnd, width, height, start)
                    .map_err(|error| cap_err("bind selected-window clicks", error))?,
            )
        }
        None => None,
    };
    let input = if let Some(observer) = window_clicks.clone() {
        InputListener::start_observed(start, cfg.capture_keys, move |event, t_ms| {
            observer.observe_event(event, t_ms);
        })?
    } else {
        InputListener::start(start, cfg.capture_keys)?
    };
    if let Some(readiness) = cfg.readiness.as_ref() {
        readiness.publish_input_hook_startup(input.startup_observation(cfg.capture_keys));
    }
    Ok((start, input, window_clicks))
}
