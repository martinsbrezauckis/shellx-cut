//! WGC input listener startup and its per-capture readiness observation.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use record_core::Result;
use windows::Win32::Foundation::HWND;

use crate::input::InputListener;
use crate::windows_window_clicks::WindowClickCapture;
use crate::{windows::cap_err, CaptureConfig};

pub(super) fn start_deferred(
    cfg: &CaptureConfig,
    window_hwnd: Option<HWND>,
    width: u32,
    height: u32,
    origin: Arc<OnceLock<Instant>>,
) -> Result<(InputListener, Option<WindowClickCapture>)> {
    let window_clicks = match window_hwnd {
        Some(hwnd) => {
            let id = cfg.window.as_deref().ok_or_else(|| {
                cap_err(
                    "bind selected-window clicks",
                    "the admitted window identity is missing",
                )
            })?;
            Some(
                WindowClickCapture::new_deferred(id, hwnd, width, height, origin.clone())
                    .map_err(|error| cap_err("bind selected-window clicks", error))?,
            )
        }
        None => None,
    };
    let observer = window_clicks.clone().map(|clicks| {
        Box::new(move |event: &rdevin::Event, t_ms: u64| {
            clicks.observe_event(event, t_ms);
        }) as Box<dyn FnMut(&rdevin::Event, u64) + Send>
    });
    let input = InputListener::start_deferred(origin, cfg.capture_keys, observer)?;
    if let Some(readiness) = cfg.readiness.as_ref() {
        readiness.publish_input_hook_startup(input.startup_observation(cfg.capture_keys));
    }
    Ok((input, window_clicks))
}
